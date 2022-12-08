//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::HashMap,
    convert::TryFrom,
    fs::create_dir_all,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use diesel::{
    dsl::now,
    prelude::*,
    result::{DatabaseErrorKind, Error},
    sql_query,
    sql_types::{BigInt, Binary, Nullable, Text},
    SqliteConnection,
};
use log::{debug, warn};
use serde_json::json;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_dan_common_types::{
    Epoch,
    NodeHeight,
    ObjectPledge,
    PayloadId,
    QuorumCertificate,
    ShardId,
    SubstateState,
    TreeNodeHash,
};
use tari_dan_core::{
    models::{
        vote_message::VoteMessage,
        HotStuffTreeNode,
        LeafNode,
        Payload,
        RecentTransaction,
        SQLSubstate,
        SQLTransaction,
        SubstateShardData,
        TariDanPayload,
    },
    storage::{
        shard_store::{ShardStore, ShardStoreTransaction},
        StorageError,
    },
};
use tari_dan_engine::transaction::{Transaction, TransactionMeta};
use tari_engine_types::{commit_result::FinalizeResult, instruction::Instruction, signature::InstructionSignature};
use tari_utilities::{hex::Hex, ByteArray};

use crate::{
    error::SqliteStorageError,
    models::{
        high_qc::{HighQc, NewHighQc},
        last_executed_height::{LastExecutedHeight, NewLastExecutedHeight},
        last_voted_height::{LastVotedHeight, NewLastVotedHeight},
        leader_proposals::{LeaderProposal, NewLeaderProposal},
        leaf_nodes::{LeafNode as DbLeafNode, NewLeafNode},
        lock_node_and_height::{LockNodeAndHeight, NewLockNodeAndHeight},
        node::{NewNode, Node},
        payload::{NewPayload, Payload as SqlPayload},
        pledge::{NewShardPledge, ShardPledge},
        received_votes::{NewReceivedVote, ReceivedVote},
        substate::{ImportedSubstate, NewSubstate, Substate},
    },
    schema::{
        high_qcs::dsl::high_qcs,
        last_executed_heights::dsl::last_executed_heights,
        last_voted_heights::dsl::last_voted_heights,
        leader_proposals::dsl::leader_proposals,
        leaf_nodes::dsl::leaf_nodes,
        lock_node_and_heights::dsl::lock_node_and_heights,
        nodes,
        payloads::dsl::payloads,
        received_votes::dsl::received_votes,
        substates::dsl::substates,
    },
    SqliteTransaction,
};

const LOG_TARGET: &str = "tari::dan::storage::sqlite::shard_store";

#[derive(Debug, QueryableByName)]
pub struct QueryableRecentTransaction {
    #[sql_type = "Binary"]
    pub payload_id: Vec<u8>,
    #[sql_type = "BigInt"]
    pub timestamp: i64,
    #[sql_type = "Text"]
    pub meta: String,
    #[sql_type = "Text"]
    pub instructions: String,
}

impl From<QueryableRecentTransaction> for RecentTransaction {
    fn from(recent_transaction: QueryableRecentTransaction) -> Self {
        Self {
            payload_id: recent_transaction.payload_id,
            timestamp: recent_transaction.timestamp,
            meta: recent_transaction.meta,
            instructions: recent_transaction.instructions,
        }
    }
}

#[derive(Debug, QueryableByName)]
pub struct QueryableTransaction {
    #[sql_type = "Binary"]
    pub node_hash: Vec<u8>,
    #[sql_type = "Binary"]
    pub parent_node_hash: Vec<u8>,
    #[sql_type = "Binary"]
    pub shard: Vec<u8>,
    #[sql_type = "BigInt"]
    pub height: i64,
    #[sql_type = "BigInt"]
    pub payload_height: i64,
    #[sql_type = "BigInt"]
    pub total_votes: i64,
    #[sql_type = "BigInt"]
    pub total_leader_proposals: i64,
}

impl From<QueryableTransaction> for SQLTransaction {
    fn from(transaction: QueryableTransaction) -> Self {
        Self {
            node_hash: transaction.node_hash,
            parent_node_hash: transaction.parent_node_hash,
            shard: transaction.shard,
            height: transaction.height,
            payload_height: transaction.payload_height,
            total_votes: transaction.total_votes,
            total_leader_proposals: transaction.total_leader_proposals,
        }
    }
}

#[derive(Debug, QueryableByName)]
pub struct QueryableSubstate {
    #[sql_type = "Binary"]
    pub shard_id: Vec<u8>,
    #[sql_type = "BigInt"]
    pub version: i64,
    #[sql_type = "Text"]
    pub data: String,
    #[sql_type = "Text"]
    pub created_justify: String,
    #[sql_type = "Binary"]
    pub created_node_hash: Vec<u8>,
    #[sql_type = "Nullable<Text>"]
    pub destroyed_justify: Option<String>,
    #[sql_type = "Nullable<Binary>"]
    pub destroyed_node_hash: Option<Vec<u8>>,
}

impl From<QueryableSubstate> for SQLSubstate {
    fn from(transaction: QueryableSubstate) -> Self {
        Self {
            shard_id: transaction.shard_id,
            version: transaction.version,
            data: transaction.data,
            created_justify: transaction.created_justify,
            destroyed_justify: transaction.destroyed_justify,
        }
    }
}

#[derive(Clone)]
pub struct SqliteShardStore {
    connection: Arc<Mutex<SqliteConnection>>,
}

impl SqliteShardStore {
    pub fn try_create(path: PathBuf) -> Result<Self, StorageError> {
        create_dir_all(path.parent().unwrap()).map_err(|_| StorageError::FileSystemPathDoesNotExist)?;

        let database_url = path.to_str().expect("database_url utf-8 error").to_string();
        let connection = SqliteConnection::establish(&database_url).map_err(SqliteStorageError::from)?;

        embed_migrations!("./migrations");
        if let Err(err) = embedded_migrations::run_with_output(&connection, &mut std::io::stdout()) {
            log::error!(target: LOG_TARGET, "Error running migrations: {}", err);
        }
        connection
            .execute("PRAGMA foreign_keys = ON;")
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "set pragma".to_string(),
            })?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }
}
impl ShardStore for SqliteShardStore {
    type Addr = PublicKey;
    type Payload = TariDanPayload;
    type Transaction<'a> = SqliteShardStoreTransaction<'a>;

    fn create_tx(&self) -> Result<Self::Transaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteShardStoreTransaction::new(tx))
    }
}

pub struct SqliteShardStoreTransaction<'a> {
    transaction: SqliteTransaction<'a>,
}

impl<'a> SqliteShardStoreTransaction<'a> {
    fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self { transaction }
    }

    fn create_pledge(&mut self, shard: ShardId, obj: ShardPledge) -> Result<ObjectPledge, StorageError> {
        use crate::schema::substates::{shard_id, version};
        let current_state: Option<Substate> = substates
            .filter(shard_id.eq(shard.as_bytes()))
            .order_by(version.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Create object pledge error: {}", e),
            })?;

        if let Some(current_state) = current_state {
            Ok(ObjectPledge {
                shard_id: shard,
                pledged_to_payload: PayloadId::try_from(obj.pledged_to_payload_id)?,
                current_state: if current_state.is_destroyed() {
                    SubstateState::Down {
                        deleted_by: PayloadId::try_from(current_state.destroyed_by_payload_id.unwrap_or_default())?,
                    }
                } else {
                    SubstateState::Up {
                        created_by: PayloadId::try_from(current_state.created_by_payload_id)?,
                        data: serde_json::from_str::<tari_engine_types::substate::Substate>(&current_state.data)
                            .map_err(|source| StorageError::SerdeJson {
                                source,
                                operation: "create_pledge".to_string(),
                                data: "pledge".to_string(),
                            })?,
                    }
                },
            })
        } else {
            Ok(ObjectPledge {
                shard_id: shard,
                pledged_to_payload: PayloadId::try_from(obj.pledged_to_payload_id)?,
                current_state: SubstateState::DoesNotExist,
            })
        }
    }

    fn map_substate_to_shard_data(ss: &Substate) -> Result<SubstateShardData, StorageError> {
        Ok(SubstateShardData::new(
            ShardId::try_from(ss.shard_id.clone())?,
            ss.version as u32,
            serde_json::from_str(&ss.data).map_err(|source| StorageError::SerdeJson {
                source,
                operation: "get_substate_states".to_string(),
                data: "substate data".to_string(),
            })?,
            NodeHeight(ss.created_height as u64),
            ss.destroyed_height.map(|v| NodeHeight(v as u64)),
            TreeNodeHash::try_from(ss.created_node_hash.clone()).map_err(|_| StorageError::DecodingError)?,
            ss.destroyed_node_hash
                .as_ref()
                .map(|v| TreeNodeHash::try_from(v.clone()).map_err(|_| StorageError::DecodingError))
                .transpose()?,
            PayloadId::try_from(ss.created_by_payload_id.clone()).map_err(|_| StorageError::DecodingError)?,
            ss.destroyed_by_payload_id
                .as_ref()
                .map(|v| PayloadId::try_from(v.clone()).map_err(|_| StorageError::DecodingError))
                .transpose()?,
            serde_json::from_str(&ss.created_justify).map_err(|source| StorageError::SerdeJson {
                source,
                operation: "get_substate_states".to_string(),
                data: "created_justify".to_string(),
            })?,
            ss.destroyed_justify
                .as_ref()
                .map(|v| {
                    serde_json::from_str(v).map_err(|source| StorageError::SerdeJson {
                        source,
                        operation: "get_substate_states".to_string(),
                        data: "destroyed_justify".to_string(),
                    })
                })
                .transpose()?,
        ))
    }
}

impl ShardStoreTransaction<PublicKey, TariDanPayload> for SqliteShardStoreTransaction<'_> {
    fn commit(self) -> Result<(), StorageError> {
        self.transaction.commit()?;
        Ok(())
    }

    fn count_high_qc_for(&self, shard_id: ShardId) -> Result<usize, StorageError> {
        use crate::schema::high_qcs::dsl;

        high_qcs
            .count()
            .filter(dsl::shard_id.eq(shard_id.as_bytes()))
            .get_result(self.transaction.connection())
            .map(|count: i64| count as usize)
            .map_err(|e| StorageError::QueryError {
                reason: format!("Failed to count high_qc: {}", e),
            })
    }

    fn update_high_qc(
        &mut self,
        identity: PublicKey,
        shard: ShardId,
        qc: QuorumCertificate,
    ) -> Result<(), StorageError> {
        // update all others for this shard to highest == false
        let shard = Vec::from(shard.0);

        let new_row = NewHighQc {
            shard_id: shard,
            height: qc.local_node_height().as_u64() as i64,
            qc_json: json!(qc).to_string(),
            identity: identity.to_vec(),
        };
        match diesel::insert_into(high_qcs)
            .values(&new_row)
            .execute(self.transaction.connection())
        {
            Ok(_) => Ok(()),
            // (shard_id, height) is a unique index
            Err(Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => {
                debug!(target: LOG_TARGET, "High QC already exists");
                Ok(())
            },
            Err(err) => Err(StorageError::QueryError {
                reason: format!("update high QC error: {}", err),
            }),
        }
    }

    fn set_payload(&mut self, payload: TariDanPayload) -> Result<(), StorageError> {
        let transaction = payload.transaction();
        let instructions = json!(&transaction.instructions()).to_string();

        let signature = transaction.signature();

        let public_nonce = Vec::from(signature.signature().get_public_nonce().as_bytes());
        let scalar = Vec::from(signature.signature().get_signature().as_bytes());

        let fee = transaction.fee() as i64;
        let sender_public_key = Vec::from(transaction.sender_public_key().as_bytes());

        let meta = json!(transaction.meta()).to_string();

        let payload_id = Vec::from(payload.to_id().as_slice());

        let new_row = NewPayload {
            payload_id,
            instructions,
            public_nonce,
            scalar,
            fee,
            sender_public_key,
            meta,
            result: None,
        };

        match diesel::insert_into(payloads)
            .values(&new_row)
            .execute(self.transaction.connection())
        {
            Ok(_) => {},
            Err(err) => {
                // It can happen that we get this payload from two shards that we are responsible
                match err {
                    Error::DatabaseError(kind, _) => {
                        if matches!(kind, DatabaseErrorKind::UniqueViolation) {
                            debug!(target: LOG_TARGET, "Payload already exists");
                            return Ok(());
                        }
                    },
                    _ => {
                        return Err(StorageError::QueryError {
                            reason: format!("Set payload error: {}", err),
                        })
                    },
                }
            },
        }
        Ok(())
    }

    fn get_leaf_node(&self, shard: ShardId) -> Result<LeafNode, StorageError> {
        use crate::schema::leaf_nodes::{node_height, shard_id};
        let leaf_node: Option<DbLeafNode> = leaf_nodes
            .filter(shard_id.eq(Vec::from(shard.as_bytes())))
            .order_by(node_height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get leaf node: {}", e),
            })?;

        if let Some(leaf_node) = leaf_node {
            Ok(LeafNode::new(
                TreeNodeHash::try_from(leaf_node.tree_node_hash).unwrap(),
                NodeHeight(leaf_node.node_height as u64),
            ))
        } else {
            // if no leaves, return genesis
            Ok(LeafNode::genesis())
        }
    }

    fn update_leaf_node(&mut self, shard: ShardId, node: TreeNodeHash, height: NodeHeight) -> Result<(), StorageError> {
        let shard = Vec::from(shard.0);
        let tree_node_hash = Vec::from(node.as_bytes());
        // This cast is lossless, it does not matter if the height wraps to a negative number
        let node_height = height.as_u64() as i64;

        let new_row = NewLeafNode {
            shard_id: shard,
            tree_node_hash,
            node_height,
        };

        // TODO: verify that we just need to add a new row to the table, instead
        // of possibly updating an existing row
        diesel::insert_into(leaf_nodes)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Update leaf node error: {}", e),
            })?;

        Ok(())
    }

    fn get_high_qc_for(&self, shard: ShardId) -> Result<QuorumCertificate, StorageError> {
        use crate::schema::high_qcs::dsl;
        let qc: Option<HighQc> = dsl::high_qcs
            .filter(dsl::shard_id.eq(Vec::from(shard.0)))
            .order_by(dsl::height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get high qc error: {}", e),
            })?;

        let qc = qc.ok_or_else(|| StorageError::NotFound {
            item: "high_qc".to_string(),
            key: shard.to_string(),
        })?;

        let qc = serde_json::from_str(&qc.qc_json).map_err(|source| StorageError::SerdeJson {
            source,
            operation: "get_high_qc_for".to_string(),
            data: qc.qc_json.to_string(),
        })?;
        Ok(qc)
    }

    fn get_payload(&self, id: &PayloadId) -> Result<TariDanPayload, StorageError> {
        use crate::schema::payloads::payload_id;

        let payload: Option<SqlPayload> = payloads
            .filter(payload_id.eq(Vec::from(id.as_slice())))
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get payload error: {}", e),
            })?;

        let payload = payload.ok_or_else(|| StorageError::NotFound {
            item: "payload".to_string(),
            key: id.to_string(),
        })?;

        let instructions: Vec<Instruction> =
            serde_json::from_str(&payload.instructions).map_err(|source| StorageError::SerdeJson {
                source,
                operation: "get_payload".to_string(),
                data: payload.instructions.to_string(),
            })?;

        let fee = payload.fee as u64;

        let public_nonce =
            PublicKey::from_vec(&payload.public_nonce).map_err(StorageError::InvalidByteArrayConversion)?;
        let signature =
            PrivateKey::from_bytes(payload.scalar.as_slice()).map_err(StorageError::InvalidByteArrayConversion)?;

        let signature: InstructionSignature = InstructionSignature::try_from(Signature::new(public_nonce, signature))
            .map_err(|e| StorageError::InvalidTypeCasting {
            reason: format!("Get payload error: {}", e),
        })?;

        let sender_public_key =
            PublicKey::from_vec(&payload.sender_public_key).map_err(StorageError::InvalidByteArrayConversion)?;
        let meta: TransactionMeta = serde_json::from_str(&payload.meta).map_err(|source| StorageError::SerdeJson {
            source,
            operation: "get_payload".to_string(),
            data: payload.meta.to_string(),
        })?;

        let transaction = Transaction::new(fee, instructions, signature, sender_public_key, meta);
        let mut tari_dan_payload = TariDanPayload::new(transaction);

        // deserialize the transaction result
        let result_field: Option<FinalizeResult> = match payload.result {
            Some(result_json) => {
                let result: FinalizeResult =
                    serde_json::from_str(&result_json).map_err(|_| StorageError::DecodingError)?;
                Some(result)
            },
            None => None,
        };
        if let Some(result) = result_field {
            tari_dan_payload.set_result(result);
        }

        Ok(tari_dan_payload)
    }

    fn get_node(&self, hash: &TreeNodeHash) -> Result<HotStuffTreeNode<PublicKey, TariDanPayload>, StorageError> {
        if hash == &TreeNodeHash::zero() {
            return Ok(HotStuffTreeNode::genesis());
        }

        use crate::schema::nodes::node_hash;

        let hash = Vec::from(hash.as_bytes());
        // TODO: Do we need to add an index to the table to order by `height` and `payload_height`
        // more efficiently ?
        let node: Option<Node> = nodes::dsl::nodes
            .filter(node_hash.eq(hash.clone()))
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get node error: {}", e),
            })?;

        if let Some(node) = node {
            let mut parent = [0u8; 32];
            parent.copy_from_slice(node.parent_node_hash.as_slice());

            let parent = TreeNodeHash::from(parent);
            let hgt = node.height as u64;

            let shard = ShardId::from_bytes(&node.shard)?;
            let payload = PayloadId::try_from(node.payload_id)?;

            let payload_hgt = node.payload_height as u64;
            let local_pledge: Option<ObjectPledge> =
                serde_json::from_str(&node.local_pledges).map_err(|source| StorageError::SerdeJson {
                    source,
                    operation: "get_node".to_string(),
                    // TODO: can't reference the actual value for some reason
                    data: "local_pledges".to_string(),
                })?;

            let epoch = node.epoch as u64;
            let proposed_by =
                PublicKey::from_vec(&node.proposed_by).map_err(StorageError::InvalidByteArrayConversion)?;

            let justify: QuorumCertificate =
                serde_json::from_str(&node.justify).map_err(|source| StorageError::SerdeJson {
                    source,
                    operation: "get_node".to_string(),
                    data: "justify".to_string(), // node.justify.to_string(),
                })?;

            Ok(HotStuffTreeNode::new(
                parent,
                shard,
                NodeHeight(hgt),
                payload,
                None,
                NodeHeight(payload_hgt),
                local_pledge,
                Epoch(epoch),
                proposed_by,
                justify,
            ))
        } else {
            Err(StorageError::NotFound {
                item: "node".to_string(),
                key: hash.to_hex(),
            })
        }
    }

    fn save_node(&mut self, node: HotStuffTreeNode<PublicKey, TariDanPayload>) -> Result<(), StorageError> {
        let node_hash = Vec::from(node.hash().as_bytes());
        let parent_node_hash = Vec::from(node.parent().as_bytes());

        let height = node.height().as_u64() as i64;
        let shard = Vec::from(node.shard().as_bytes());

        let payload_id = Vec::from(node.payload_id().as_bytes());
        let payload_height = node.payload_height().as_u64() as i64;

        let local_pledges = json!(&node.local_pledge()).to_string();

        let epoch = node.epoch().as_u64() as i64;
        let proposed_by = Vec::from(node.proposed_by().as_bytes());

        let justify = json!(node.justify()).to_string();

        let new_row = NewNode {
            node_hash,
            parent_node_hash,
            height,
            shard,
            payload_id,
            payload_height,
            local_pledges,
            epoch,
            proposed_by,
            justify,
        };

        match diesel::insert_into(nodes::dsl::nodes)
            .values(&new_row)
            .execute(self.transaction.connection())
        {
            Ok(_) => {},
            Err(err) => match err {
                Error::DatabaseError(kind, _) => {
                    if matches!(kind, DatabaseErrorKind::UniqueViolation) {
                        debug!(target: LOG_TARGET, "Node already exists");
                        return Ok(());
                    }
                },
                _ => {
                    return Err(StorageError::QueryError {
                        reason: format!("Save node error: {}", err),
                    })
                },
            },
        }

        Ok(())
    }

    fn get_locked_node_hash_and_height(&self, shard: ShardId) -> Result<(TreeNodeHash, NodeHeight), StorageError> {
        use crate::schema::lock_node_and_heights::{node_height, shard_id};

        let shard = Vec::from(shard.0);

        let lock_node_hash_and_height: Option<LockNodeAndHeight> = lock_node_and_heights
            .filter(shard_id.eq(shard))
            .order_by(node_height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get locked node hash and height error: {}", e),
            })?;

        if let Some(data) = lock_node_hash_and_height {
            let tree_node_hash = TreeNodeHash::try_from(data.tree_node_hash).unwrap();
            // This cast is lossless as i64 and u64 are both 64-bits
            let height = data.node_height as u64;
            Ok((tree_node_hash, NodeHeight(height)))
        } else {
            Ok((TreeNodeHash::zero(), NodeHeight(0)))
        }
    }

    fn set_locked(
        &mut self,
        shard: ShardId,
        node_hash: TreeNodeHash,
        node_height: NodeHeight,
    ) -> Result<(), StorageError> {
        let shard = Vec::from(shard.as_bytes());
        let node_hash = Vec::from(node_hash.as_bytes());
        let node_height = node_height.as_u64() as i64;

        let new_row = NewLockNodeAndHeight {
            shard_id: shard,
            tree_node_hash: node_hash,
            node_height,
        };

        diesel::insert_into(lock_node_and_heights)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set locked error: {}", e),
            })?;
        Ok(())
    }

    fn pledge_object(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        current_height: NodeHeight,
    ) -> Result<ObjectPledge, StorageError> {
        use crate::schema::shard_pledges::{
            created_height,
            dsl::shard_pledges,
            id,
            is_active,
            pledged_to_payload_id,
            shard_id,
        };
        let shard_vec = Vec::from(shard.as_bytes());
        let f_payload = Vec::from(payload.as_slice());

        let existing_pledge: Option<ShardPledge> = shard_pledges
            .filter(shard_id.eq(shard_vec.clone()))
            .filter(is_active.eq(true))
            .order_by(created_height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Pledge object error: {}", e),
            })?;

        if let Some(obj) = existing_pledge {
            // TODO: write test for this logic
            // if obj.pledged_until_height.unwrap_or_default() as u64 >= current_height.as_u64() {
            return self.create_pledge(shard, obj);
            // }
        }

        // otherwise save pledge
        let new_row = NewShardPledge {
            shard_id: shard_vec.clone(),
            created_height: current_height.as_u64() as i64,
            pledged_to_payload_id: f_payload.clone(),
            is_active: true,
        };
        let num_affected = diesel::insert_into(shard_pledges)
            .values(new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Pledge insert error: {}", e),
            })?;

        if num_affected != 1 {
            return Err(StorageError::QueryError {
                reason: "Pledge object error: no pledge created".to_string(),
            });
        }

        let new_pledge: ShardPledge = shard_pledges
            .filter(
                shard_id
                    .eq(&shard_vec)
                    .and(is_active.eq(true))
                    .and(pledged_to_payload_id.eq(f_payload)),
            )
            .then_order_by(id.desc())
            .first(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Pledge object error: {}", e),
            })?;

        self.create_pledge(shard, new_pledge)
    }

    fn set_last_executed_height(&mut self, shard: ShardId, height: NodeHeight) -> Result<(), StorageError> {
        let shard = Vec::from(shard.as_bytes());
        let node_height = height.as_u64() as i64;

        let new_row = NewLastExecutedHeight {
            shard_id: shard,
            node_height,
        };

        diesel::insert_into(last_executed_heights)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set last executed height error: {}", e),
            })?;
        Ok(())
    }

    fn get_last_executed_height(&self, shard: ShardId) -> Result<NodeHeight, StorageError> {
        use crate::schema::last_executed_heights::{node_height, shard_id};

        let last_executed_height: Option<LastExecutedHeight> = last_executed_heights
            .filter(shard_id.eq(Vec::from(shard.0)))
            .order_by(node_height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get last executed height: {}", e),
            })?;

        if let Some(last_exec_height) = last_executed_height {
            let height = last_exec_height.node_height as u64;
            Ok(NodeHeight(height))
        } else {
            Ok(NodeHeight(0))
        }
    }

    // TODO: the version number should be passed in here, or alternatively, the version number should be removed
    #[allow(clippy::too_many_lines)]
    fn save_substate_changes(
        &mut self,
        changes: &HashMap<ShardId, Vec<SubstateState>>,
        node: &HotStuffTreeNode<PublicKey, TariDanPayload>,
    ) -> Result<(), StorageError> {
        use crate::schema::substates::{
            destroyed_by_payload_id,
            destroyed_height,
            destroyed_justify,
            destroyed_node_hash,
            destroyed_timestamp,
            id,
            shard_id,
            version,
        };
        let payload_id = Vec::from(node.payload_id().as_slice());
        dbg!(&changes
            .iter()
            .map(|(k, v)| (
                Hex::to_hex(&k.as_bytes().to_vec()),
                v.iter().map(|vi| vi.as_str()).collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>());
        // Only save this node's shard state
        for (sid, st_changes) in changes.iter().filter(|(s, _)| *s == &node.shard()) {
            let shard = Vec::from(sid.as_bytes());

            let current_state = substates
                .filter(shard_id.eq(shard.clone()))
                .order_by(version.desc())
                .first::<Substate>(self.transaction.connection())
                .optional()
                .map_err(|e| StorageError::QueryError {
                    reason: format!("Save substate changes error. Could not retrieve current state: {}", e),
                })?;

            dbg!(sid);

            for st_change in st_changes {
                match st_change {
                    SubstateState::DoesNotExist => (),
                    SubstateState::Up { data: d, .. } => {
                        if let Some(s) = &current_state {
                            if !s.is_destroyed() {
                                return Err(StorageError::QueryError {
                                    reason: "Save substate changes error. Cannot create a substate that has not been \
                                             destroyed"
                                        .to_string(),
                                });
                            }
                        }

                        let pretty_data =
                            serde_json::to_string_pretty(d).map_err(|source| StorageError::SerdeJson {
                                source,
                                operation: "save_substate_changes".to_string(),
                                data: "substate data".to_string(),
                            })?;
                        let new_row = NewSubstate {
                            shard_id: shard.clone(),
                            version: current_state.as_ref().map(|s| s.version).unwrap_or(0) + 1,
                            data: pretty_data,
                            created_by_payload_id: payload_id.clone(),
                            created_justify: json!(node.justify()).to_string(),
                            created_height: node.height().as_u64() as i64,
                            created_node_hash: node.hash().as_bytes().to_vec(),
                        };
                        diesel::insert_into(substates)
                            .values(&new_row)
                            .execute(self.transaction.connection())
                            .map_err(|e| StorageError::QueryError {
                                reason: format!("Save substate changes error. Could not insert new substate: {}", e),
                            })?;
                    },
                    SubstateState::Down { .. } => {
                        if let Some(s) = &current_state {
                            if s.is_destroyed() {
                                return Err(StorageError::QueryError {
                                    reason: "Save substate changes error. Cannot destroy a substate that has already \
                                             been destroyed"
                                        .to_string(),
                                });
                            }

                            let rows_affected = diesel::update(substates.filter(id.eq(s.id)))
                                .set((
                                    destroyed_by_payload_id.eq(payload_id.clone()),
                                    destroyed_justify.eq(json!(node.justify()).to_string()),
                                    destroyed_height.eq(node.height().as_u64() as i64),
                                    destroyed_node_hash.eq(node.hash().as_bytes()),
                                    destroyed_timestamp.eq(now),
                                ))
                                .execute(self.transaction.connection())
                                .map_err(|e| StorageError::QueryError {
                                    reason: format!("Save substate changes error. Could not destroy substate: {}", e),
                                })?;
                            if rows_affected != 1 {
                                return Err(StorageError::QueryError {
                                    reason: "Save substate changes error. More or less than 1 row affected".to_string(),
                                });
                            }
                        } else {
                            return Err(StorageError::QueryError {
                                reason: "Save substate changes error. Cannot destroy a substate that does not exist"
                                    .to_string(),
                            });
                        }
                    },
                }
            }
        }
        Ok(())
    }

    fn insert_substates(&mut self, substate_data: SubstateShardData) -> Result<(), StorageError> {
        let new_row = ImportedSubstate {
            shard_id: substate_data.shard_id().as_bytes().to_vec(),
            version: i64::from(substate_data.version()),
            data: serde_json::to_string_pretty(substate_data.substate()).map_err(|source| StorageError::SerdeJson {
                source,
                operation: "insert_substates".to_string(),
                data: "substate data".to_string(),
            })?,
            created_by_payload_id: substate_data.created_payload_id().as_bytes().to_vec(),
            created_justify: serde_json::to_string_pretty(substate_data.created_justify()).map_err(|source| {
                StorageError::SerdeJson {
                    source,
                    operation: "insert_substates".to_string(),
                    data: "created_justify".to_string(),
                }
            })?,
            created_height: substate_data.created_height().as_u64() as i64,
            created_node_hash: substate_data.created_node_hash().as_bytes().to_vec(),
            destroyed_by_payload_id: substate_data.destroyed_payload_id().map(|v| v.as_bytes().to_vec()),
            destroyed_justify: substate_data
                .destroyed_justify()
                .as_ref()
                .map(|v| {
                    serde_json::to_string_pretty(&v).map_err(|source| StorageError::SerdeJson {
                        source,
                        operation: "insert_substates".to_string(),
                        data: "destroyed_justify".to_string(),
                    })
                })
                .transpose()?,
            destroyed_height: substate_data.destroyed_height().map(|v| v.as_u64() as i64),
            destroyed_node_hash: substate_data.destroyed_node_hash().map(|v| v.as_bytes().to_vec()),
        };

        diesel::insert_into(substates)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Insert substates: {}", e),
            })?;

        Ok(())
    }

    fn get_state_inventory(&self) -> Result<Vec<ShardId>, StorageError> {
        let substate_states: Option<Vec<Substate>> = substates
            .get_results(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get substate change error: {}", e),
            })
            .unwrap();

        if let Some(substate_states) = substate_states {
            substate_states
                .iter()
                .map(|ss| ShardId::from_bytes(ss.shard_id.as_slice()).map_err(|_| StorageError::DecodingError))
                .collect::<Result<Vec<_>, _>>()
        } else {
            Ok(vec![])
        }
    }

    fn get_substate_states(&self, shard_ids: &[ShardId]) -> Result<Vec<SubstateShardData>, StorageError> {
        use crate::schema::substates::shard_id;
        let shard_ids = shard_ids
            .iter()
            .map(|sh| Vec::from(sh.as_bytes()))
            .collect::<Vec<Vec<u8>>>();

        let substate_states: Option<Vec<crate::models::substate::Substate>> = substates
            .filter(shard_id.eq_any(shard_ids))
            .get_results(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get substate change error: {}", e),
            })
            .unwrap();

        if let Some(substate_states) = substate_states {
            substate_states
                .iter()
                .map(Self::map_substate_to_shard_data)
                .collect::<Result<_, _>>()
        } else {
            Err(StorageError::NotFound {
                item: "substate".to_string(),
                key: "No data found for available shards".to_string(),
            })
        }
    }

    fn get_substate_states_by_range(
        &self,
        start_shard_id: ShardId,
        end_shard_id: ShardId,
        excluded_shards: &[ShardId],
    ) -> Result<Vec<SubstateShardData>, StorageError> {
        use crate::schema::substates;
        let excluded_shards = excluded_shards
            .iter()
            .map(|sh| Vec::from(sh.as_bytes()))
            .collect::<Vec<Vec<u8>>>();

        let substate_states: Option<Vec<Substate>> = substates
            .filter(
                substates::shard_id
                    .ge(Vec::from(start_shard_id.as_bytes()))
                    .and(substates::shard_id.le(Vec::from(end_shard_id.as_bytes())))
                    .and(substates::shard_id.ne_all(excluded_shards)),
            )
            .get_results(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get substate change error: {}", e),
            })
            .unwrap();

        if let Some(substate_states) = substate_states {
            substate_states
                .iter()
                .map(Self::map_substate_to_shard_data)
                .collect::<Result<_, _>>()
        } else {
            Err(StorageError::NotFound {
                item: "substate".to_string(),
                key: "No data found for available shards".to_string(),
            })
        }
    }

    fn get_last_voted_height(&self, shard: ShardId) -> Result<NodeHeight, StorageError> {
        use crate::schema::last_voted_heights::{node_height, shard_id};

        let last_vote: Option<LastVotedHeight> = last_voted_heights
            .filter(shard_id.eq(Vec::from(shard.as_bytes())))
            .order_by(node_height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get last voted height error: {}", e),
            })?;

        if let Some(last_vote_height) = last_vote {
            let height = last_vote_height.node_height as u64;
            Ok(NodeHeight(height))
        } else {
            Ok(NodeHeight(0))
        }
    }

    fn set_last_voted_height(&mut self, shard: ShardId, height: NodeHeight) -> Result<(), StorageError> {
        let shard = Vec::from(shard.as_bytes());
        let height = height.as_u64() as i64;

        let new_row = NewLastVotedHeight {
            shard_id: shard,
            node_height: height,
        };

        diesel::insert_into(last_voted_heights)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set last voted height: {}", e),
            })?;
        Ok(())
    }

    fn get_leader_proposals(
        &self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shard: ShardId,
    ) -> Result<Option<HotStuffTreeNode<PublicKey, TariDanPayload>>, StorageError> {
        use crate::schema::leader_proposals::{payload_height as s_payload_height, payload_id, shard_id};

        let payload_vote: Option<LeaderProposal> = leader_proposals
            .filter(
                payload_id
                    .eq(Vec::from(payload.as_slice()))
                    .and(shard_id.eq(Vec::from(shard.as_bytes())))
                    .and(s_payload_height.eq(payload_height.as_u64() as i64)),
            )
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get payload vote: {}", e),
            })?;

        if let Some(payload_vote) = payload_vote {
            let hot_stuff_tree_node: HotStuffTreeNode<PublicKey, TariDanPayload> =
                serde_json::from_str(&payload_vote.hotstuff_tree_node).map_err(|source| StorageError::SerdeJson {
                    source,
                    operation: "get_leader_proposals".to_string(),
                    data: payload_vote.hotstuff_tree_node.to_string(),
                })?;
            Ok(Some(hot_stuff_tree_node))
        } else {
            Ok(None)
        }
    }

    fn save_leader_proposals(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        node: HotStuffTreeNode<PublicKey, TariDanPayload>,
    ) -> Result<(), StorageError> {
        let shard = Vec::from(shard.as_bytes());
        let payload = Vec::from(payload.as_slice());
        let payload_height = payload_height.as_u64() as i64;
        let node_hash = node.hash().as_bytes().to_vec();
        let node = json!(&node).to_string();

        let new_row = NewLeaderProposal {
            shard_id: shard,
            payload_id: payload,
            payload_height,
            node_hash,
            hotstuff_tree_node: node,
        };

        match diesel::insert_into(leader_proposals)
            .values(&new_row)
            .execute(self.transaction.connection())
        {
            Ok(_) => Ok(()),
            Err(e) => match e {
                diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) => {
                    warn!(target: LOG_TARGET, "Leader proposal already exists");
                    Ok(())
                },
                _ => Err(StorageError::QueryError {
                    reason: format!("Save leader proposal: {}", e),
                }),
            },
        }
        // .map_err(|e| StorageError::QueryError {
        //     reason: format!("Save payload vote error: {}", e),
        // })?;
    }

    fn has_vote_for(&self, from: &PublicKey, node_hash: TreeNodeHash, shard: ShardId) -> Result<bool, StorageError> {
        use crate::schema::received_votes::{address, shard_id, tree_node_hash};

        let vote: Option<ReceivedVote> = received_votes
            .filter(
                shard_id
                    .eq(Vec::from(shard.as_bytes()))
                    .and(tree_node_hash.eq(Vec::from(node_hash.as_bytes())))
                    .and(address.eq(Vec::from(from.as_bytes()))),
            )
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Has vote for error: {}", e),
            })?;

        Ok(vote.is_some())
    }

    fn save_received_vote_for(
        &mut self,
        from: PublicKey,
        node_hash: TreeNodeHash,
        shard: ShardId,
        vote_message: VoteMessage,
    ) -> Result<usize, StorageError> {
        use crate::schema::received_votes::{shard_id, tree_node_hash};

        let from = Vec::from(from.as_bytes());
        let node_hash = Vec::from(node_hash.as_bytes());
        let shard = Vec::from(shard.as_bytes());
        let vote_message = json!(&vote_message).to_string();

        let new_row = NewReceivedVote {
            tree_node_hash: node_hash.clone(),
            shard_id: shard.clone(),
            address: from,
            vote_message,
        };

        diesel::insert_into(received_votes)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Save received voted for: {}", e),
            })?;

        let count: i64 = received_votes
            .filter(
                tree_node_hash
                    .eq(Vec::from(node_hash.as_bytes()))
                    .and(shard_id.eq(Vec::from(shard.as_slice()))),
            )
            .count()
            .first(self.transaction.connection())
            .map_err(|source| StorageError::QueryError {
                reason: format!("Save received vote: {0}", source),
            })?;

        // TryFrom could fail for 32-bit environments
        usize::try_from(count as u64).map_err(|_| StorageError::InvalidIntegerCast)
    }

    fn get_received_votes_for(
        &self,
        node_hash: TreeNodeHash,
        shard: ShardId,
    ) -> Result<Vec<VoteMessage>, StorageError> {
        use crate::schema::received_votes::{shard_id, tree_node_hash};

        let filtered_votes: Option<Vec<ReceivedVote>> = received_votes
            .filter(
                shard_id
                    .eq(Vec::from(shard.as_bytes()))
                    .and(tree_node_hash.eq(Vec::from(node_hash.as_bytes()))),
            )
            .get_results(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get received vote for: {}", e),
            })?;

        if let Some(filtered_votes) = filtered_votes {
            let v = filtered_votes
                .iter()
                .map(|v| {
                    serde_json::from_str::<VoteMessage>(&v.vote_message).map_err(|source| StorageError::SerdeJson {
                        source,
                        operation: "get_received_votes_for".to_string(),
                        data: v.vote_message.to_string(),
                    })
                })
                .collect::<Result<_, _>>()?;
            Ok(v)
        } else {
            Ok(vec![])
        }
    }

    fn get_recent_transactions(&self) -> Result<Vec<RecentTransaction>, StorageError> {
        let res = sql_query("select payload_id,timestamp,meta,instructions from payloads")
            .load::<QueryableRecentTransaction>(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get recent transactions: {}", e),
            })?;
        Ok(res
            .into_iter()
            .map(|recent_transaction| recent_transaction.into())
            .collect())
    }

    fn update_payload_result(
        &self,
        requested_payload_id: &PayloadId,
        result: FinalizeResult,
    ) -> Result<(), StorageError> {
        use crate::schema::payloads;

        let result_json = serde_json::to_string(&result).map_err(|_| StorageError::EncodingError)?;

        diesel::update(payloads::table)
            .filter(payloads::payload_id.eq(requested_payload_id.as_bytes()))
            .set(payloads::result.eq(result_json))
            .execute(self.transaction.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "update_payload_result".to_string(),
            })?;

        Ok(())
    }

    fn get_transaction(&self, payload_id: Vec<u8>) -> Result<Vec<SQLTransaction>, StorageError> {
        let res = sql_query(
            "select node_hash, parent_node_hash, shard, height, payload_height, (select count(*) from received_votes \
             v where v.tree_node_hash = node_hash) as total_votes, (select count(*) from leader_proposals lp where \
             lp.payload_id  = n.payload_id and lp.payload_height = n.payload_height and lp.node_hash = n.node_hash) \
             as total_leader_proposals from nodes as n where payload_id = ? order by shard",
        )
        .bind::<Binary, _>(payload_id)
        .load::<QueryableTransaction>(self.transaction.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("Get transaction: {}", e),
        })?;
        Ok(res.into_iter().map(|transaction| transaction.into()).collect())
    }

    fn get_substates_for_payload(
        &self,
        payload_id: Vec<u8>,
        shard_id: Vec<u8>,
    ) -> Result<Vec<SQLSubstate>, StorageError> {
        let res = sql_query(
            "select * from substates where shard_id == ? and (created_by_payload_id == ? or destroyed_by_payload_id \
             == ?);",
        )
        .bind::<Binary, _>(shard_id)
        .bind::<Binary, _>(payload_id.clone())
        .bind::<Binary, _>(payload_id)
        .load::<QueryableSubstate>(self.transaction.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("Get substates: {}", e),
        })?;
        Ok(res.into_iter().map(|transaction| transaction.into()).collect())
    }

    fn complete_pledges(
        &self,
        shard: ShardId,
        payload: PayloadId,
        node_hash: &TreeNodeHash,
    ) -> Result<(), StorageError> {
        use crate::schema::shard_pledges::{
            completed_by_tree_node_hash,
            dsl::shard_pledges,
            is_active,
            pledged_to_payload_id,
            shard_id,
            updated_timestamp,
        };
        let rows_affected = diesel::update(shard_pledges)
            .filter(shard_id.eq(shard.as_bytes()))
            .filter(pledged_to_payload_id.eq(payload.as_bytes()))
            .filter(is_active.eq(true))
            .set((
                is_active.eq(false),
                completed_by_tree_node_hash.eq(node_hash.as_bytes()),
                updated_timestamp.eq(now),
            ))
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Complete pledges: {}", e),
            })?;
        if rows_affected == 0 {
            return Err(StorageError::QueryError {
                reason: "Complete pledges: No rows affected".to_string(),
            });
        }
        Ok(())
    }
}
