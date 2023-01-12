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
    convert::{TryFrom, TryInto},
    fs::create_dir_all,
    ops::Deref,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use chrono::NaiveDateTime;
use diesel::{
    dsl::now,
    prelude::*,
    result::{DatabaseErrorKind, Error},
    sql_query,
    sql_types::{BigInt, Binary, Nullable, Text, Timestamp},
    SqliteConnection,
};
use log::{debug, warn};
use serde_json::json;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_dan_common_types::{
    Epoch,
    NodeHeight,
    ObjectPledge,
    ObjectPledgeInfo,
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
        shard_store::{ShardStore, ShardStoreReadTransaction, ShardStoreWriteTransaction},
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
        pledge::{NewShardPledge, ShardPledge as DbShardPledge},
        received_votes::{NewReceivedVote, ReceivedVote},
        substate::{ImportedSubstate, NewSubstate, Substate},
    },
    SqliteTransaction,
};

const LOG_TARGET: &str = "tari::dan::storage::sqlite::shard_store";

#[derive(Debug, QueryableByName)]
pub struct QueryableRecentTransaction {
    #[sql_type = "Binary"]
    pub payload_id: Vec<u8>,
    #[sql_type = "Timestamp"]
    pub timestamp: NaiveDateTime,
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
    #[sql_type = "Timestamp"]
    pub timestamp: NaiveDateTime,
    #[sql_type = "Text"]
    pub justify: String,
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
            timestamp: transaction.timestamp,
            justify: transaction.justify,
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
    type ReadTransaction<'a> = SqliteShardStoreReadTransaction<'a>;
    type WriteTransaction<'a> = SqliteShardStoreWriteTransaction<'a>;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteShardStoreReadTransaction::new(tx))
    }

    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteShardStoreWriteTransaction::new(tx))
    }
}

pub struct SqliteShardStoreReadTransaction<'a> {
    transaction: SqliteTransaction<'a>,
}

impl<'a> SqliteShardStoreReadTransaction<'a> {
    fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self { transaction }
    }

    fn connection(&self) -> &SqliteConnection {
        self.transaction.connection()
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

impl ShardStoreReadTransaction<PublicKey, TariDanPayload> for SqliteShardStoreReadTransaction<'_> {
    fn get_high_qc_for(&self, payload_id: PayloadId, shard_id: ShardId) -> Result<QuorumCertificate, StorageError> {
        use crate::schema::high_qcs;

        let qc: Option<HighQc> = high_qcs::table
            .filter(high_qcs::shard_id.eq(shard_id.as_bytes()))
            .filter(high_qcs::payload_id.eq(payload_id.as_bytes()))
            .order_by(high_qcs::height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get high qc error: {}", e),
            })?;

        let qc = qc.ok_or_else(|| StorageError::NotFound {
            item: "high_qc".to_string(),
            key: shard_id.to_string(),
        })?;

        let qc = serde_json::from_str(&qc.qc_json).map_err(|source| StorageError::SerdeJson {
            source,
            operation: "get_high_qc_for".to_string(),
            data: qc.qc_json.to_string(),
        })?;
        Ok(qc)
    }

    fn get_leaf_node(&self, payload_id: &PayloadId, shard: &ShardId) -> Result<LeafNode, StorageError> {
        use crate::schema::leaf_nodes;
        let leaf_node: Option<DbLeafNode> = leaf_nodes::table
            .filter(leaf_nodes::shard_id.eq(shard.as_bytes()))
            .filter(leaf_nodes::payload_id.eq(payload_id.as_bytes()))
            .order_by(leaf_nodes::node_height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get leaf node: {}", e),
            })?;

        match leaf_node {
            Some(leaf_node) => Ok(LeafNode::new(
                TreeNodeHash::try_from(leaf_node.tree_node_hash).unwrap(),
                NodeHeight(leaf_node.node_height as u64),
                NodeHeight(leaf_node.payload_height as u64),
            )),
            None => Err(StorageError::NotFound {
                item: "leaf_node".to_string(),
                key: format!("shard_id={},payload_id={}", shard, payload_id),
            }),
        }
    }

    fn get_payload(&self, id: &PayloadId) -> Result<TariDanPayload, StorageError> {
        use crate::schema::payloads;

        let payload: Option<SqlPayload> = payloads::table
            .filter(payloads::payload_id.eq(Vec::from(id.as_bytes())))
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
            PublicKey::from_vec(&payload.sender_address).map_err(StorageError::InvalidByteArrayConversion)?;
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
        use crate::schema::nodes;

        let node: Option<Node> = nodes::dsl::nodes
            .filter(nodes::node_hash.eq(hash.as_bytes()))
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get node error: {}", e),
            })?;

        let node = node.ok_or_else(|| StorageError::NotFound {
            item: "node".to_string(),
            key: hash.to_hex(),
        })?;

        let parent = {
            let mut parent = [0u8; 32];
            parent.copy_from_slice(node.parent_node_hash.as_slice());
            TreeNodeHash::from(parent)
        };
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
        let proposed_by = PublicKey::from_vec(&node.proposed_by).map_err(StorageError::InvalidByteArrayConversion)?;

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
    }

    fn get_locked_node_hash_and_height(
        &self,
        payload_id: PayloadId,
        shard: ShardId,
    ) -> Result<(TreeNodeHash, NodeHeight), StorageError> {
        use crate::schema::lock_node_and_heights;

        let lock_node_hash_and_height: Option<LockNodeAndHeight> = lock_node_and_heights::table
            .filter(lock_node_and_heights::shard_id.eq(shard.as_bytes()))
            .filter(lock_node_and_heights::payload_id.eq(payload_id.as_bytes()))
            .order_by(lock_node_and_heights::node_height.desc())
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

    fn get_last_executed_height(&self, shard: ShardId, payload_id: PayloadId) -> Result<NodeHeight, StorageError> {
        use crate::schema::last_executed_heights;

        let last_executed_height: Option<LastExecutedHeight> = last_executed_heights::table
            .filter(last_executed_heights::shard_id.eq(shard.as_bytes()))
            .filter(last_executed_heights::payload_id.eq(payload_id.as_bytes()))
            .order_by(last_executed_heights::node_height.desc())
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

    fn get_state_inventory(&self) -> Result<Vec<ShardId>, StorageError> {
        use crate::schema::substates;

        let substate_states: Option<Vec<Substate>> = substates::table
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
        use crate::schema::substates;

        let shard_ids = shard_ids
            .iter()
            .map(|sh| Vec::from(sh.as_bytes()))
            .collect::<Vec<Vec<u8>>>();

        let substate_states: Option<Vec<crate::models::substate::Substate>> = substates::table
            .filter(substates::shard_id.eq_any(shard_ids))
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

        let substate_states: Option<Vec<Substate>> = substates::table
            .filter(
                substates::shard_id
                    .ge(start_shard_id.as_bytes())
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

    fn get_last_voted_height(&self, shard: ShardId, payload_id: PayloadId) -> Result<NodeHeight, StorageError> {
        use crate::schema::last_voted_heights;

        let last_vote: Option<LastVotedHeight> = last_voted_heights::table
            .filter(last_voted_heights::shard_id.eq(shard.as_bytes().to_vec()))
            .filter(last_voted_heights::payload_id.eq(payload_id.as_bytes().to_vec()))
            .order_by(last_voted_heights::node_height.desc())
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

    fn get_leader_proposals(
        &self,
        payload: PayloadId,
        payload_height: NodeHeight,
        shards: &[ShardId],
    ) -> Result<Vec<HotStuffTreeNode<PublicKey, TariDanPayload>>, StorageError> {
        use crate::schema::leader_proposals;

        let shards_bytes = shards.iter().map(|sh| sh.as_bytes()).collect::<Vec<_>>();
        let proposals: Vec<LeaderProposal> = leader_proposals::table
            .filter(
                leader_proposals::payload_id
                    .eq(payload.as_bytes())
                    .and(leader_proposals::shard_id.eq_any(shards_bytes))
                    .and(leader_proposals::payload_height.eq(payload_height.as_u64() as i64)),
            )
            .order_by(leader_proposals::shard_id.asc())
            .get_results(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get payload vote: {}", e),
            })?;

        proposals
            .into_iter()
            .map(|proposal| {
                let hot_stuff_tree_node =
                    serde_json::from_str(&proposal.hotstuff_tree_node).map_err(|source| StorageError::SerdeJson {
                        source,
                        operation: "get_leader_proposals".to_string(),
                        data: proposal.hotstuff_tree_node.to_string(),
                    })?;
                Ok(hot_stuff_tree_node)
            })
            .collect()
    }

    fn has_vote_for(&self, from: &PublicKey, node_hash: TreeNodeHash) -> Result<bool, StorageError> {
        use crate::schema::received_votes;

        let vote: Option<ReceivedVote> = received_votes::table
            .filter(
                received_votes::tree_node_hash
                    .eq(node_hash.as_bytes())
                    .and(received_votes::address.eq(from.as_bytes())),
            )
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Has vote for error: {}", e),
            })?;

        Ok(vote.is_some())
    }

    fn get_received_votes_for(&self, node_hash: TreeNodeHash) -> Result<Vec<VoteMessage>, StorageError> {
        use crate::schema::received_votes;

        let filtered_votes: Option<Vec<ReceivedVote>> = received_votes::table
            .filter(received_votes::tree_node_hash.eq(Vec::from(node_hash.as_bytes())))
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

    fn get_payload_result(&self, payload_id: &PayloadId) -> Result<FinalizeResult, StorageError> {
        use crate::schema::{payloads, payloads::dsl};

        let maybe_payload: Option<SqlPayload> = dsl::payloads
            .filter(payloads::payload_id.eq(payload_id.as_bytes()))
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get received vote for: {}", e),
            })?;

        let payload = maybe_payload.ok_or_else(|| StorageError::NotFound {
            item: "payload".to_string(),
            key: payload_id.to_string(),
        })?;

        let payload_result_json = payload.result.ok_or_else(|| StorageError::NotFound {
            item: "payload result".to_string(),
            key: payload_id.to_string(),
        })?;

        Ok(serde_json::from_str(&payload_result_json).expect("payload result in database corrupt"))
    }

    fn get_transaction(&self, payload_id: Vec<u8>) -> Result<Vec<SQLTransaction>, StorageError> {
        let res = sql_query(
            "select node_hash, parent_node_hash, shard, height, payload_height, (select count(*) from received_votes \
             v where v.tree_node_hash = node_hash) as total_votes, (select count(*) from leader_proposals lp where \
             lp.payload_id  = n.payload_id and lp.payload_height = n.payload_height and lp.node_hash = n.node_hash) \
             as total_leader_proposals, n.timestamp, n.justify from nodes as n where payload_id = ? order by shard",
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

    // -------------------------------- Pledges -------------------------------- //
    fn get_resolved_pledges_for_payload(&self, payload: PayloadId) -> Result<Vec<ObjectPledgeInfo>, StorageError> {
        use crate::schema::shard_pledges;

        let pledges: Vec<DbShardPledge> = shard_pledges::table
            .filter(shard_pledges::pledged_to_payload_id.eq(payload.as_bytes()))
            .filter(
                shard_pledges::completed_by_tree_node_hash
                    .is_not_null()
                    .or(shard_pledges::abandoned_by_tree_node_hash.is_not_null()),
            )
            .filter(shard_pledges::is_active.eq(false))
            .order_by(shard_pledges::created_height.desc())
            .load(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get resolved pledges for payload error: {}", e),
            })?;

        pledges
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, SqliteStorageError>>()
            .map_err(Into::into)
    }
}

pub struct SqliteShardStoreWriteTransaction<'a> {
    transaction: SqliteShardStoreReadTransaction<'a>,
    /// Indicates if the transaction has been explicitly committed/rolled back
    is_complete: bool,
}

impl<'a> SqliteShardStoreWriteTransaction<'a> {
    pub fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self {
            transaction: SqliteShardStoreReadTransaction::new(transaction),
            is_complete: false,
        }
    }

    fn create_pledge(&mut self, shard: ShardId, obj: DbShardPledge) -> Result<ObjectPledge, StorageError> {
        use crate::schema::substates;
        let current_state: Option<Substate> = substates::table
            .filter(substates::shard_id.eq(shard.as_bytes()))
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
}

impl ShardStoreWriteTransaction<PublicKey, TariDanPayload> for SqliteShardStoreWriteTransaction<'_> {
    fn commit(mut self) -> Result<(), StorageError> {
        self.transaction.transaction.commit()?;
        self.is_complete = true;
        Ok(())
    }

    fn rollback(mut self) -> Result<(), StorageError> {
        self.transaction.transaction.rollback()?;
        self.is_complete = true;
        Ok(())
    }

    fn insert_high_qc(
        &mut self,
        identity: PublicKey,
        shard: ShardId,
        qc: QuorumCertificate,
    ) -> Result<(), StorageError> {
        use crate::schema::high_qcs;
        // update all others for this shard to highest == false
        let new_row = NewHighQc {
            shard_id: shard.as_bytes().to_vec(),
            payload_id: qc.payload_id().as_bytes().to_vec(),
            height: qc.node_height().as_u64() as i64,
            qc_json: serde_json::to_string_pretty(&qc).unwrap(),
            identity: identity.to_vec(),
        };
        match diesel::insert_into(high_qcs::table)
            .values(&new_row)
            .execute(self.transaction.connection())
        {
            Ok(_) => Ok(()),
            // (shard_id, payload_id, height) is a unique index
            // Err(Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _)) => {
            //     // diesel::update(high_qcs::table)
            //     //     .filter(high_qcs::shard_id.eq(shard.as_bytes()))
            //     //     .filter(high_qcs::payload_id.eq(qc.payload_id().as_bytes()))
            //     //     .filter(high_qcs::height.eq(qc.node_height().as_u64() as i64))
            //     //     .set((
            //     //         high_qcs::qc_json.eq(new_row.qc_json),
            //     //         high_qcs::identity.eq(new_row.identity),
            //     //     ))
            //     //     .execute(self.transaction.connection())
            //     //     .map_err(|e| StorageError::QueryError {
            //     //         reason: format!("Update high QC: {}", e),
            //     //     })?;
            //
            //     warn!(
            //         target: LOG_TARGET,
            //         "High QC for shard {}, payload {}, height {} exists",
            //         shard,
            //         qc.payload_id(),
            //         qc.node_height()
            //     );
            //     Ok(())
            // },
            Err(err) => Err(StorageError::QueryError {
                reason: format!("update high QC error: {}", err),
            }),
        }
    }

    fn save_payload(&mut self, payload: TariDanPayload) -> Result<(), StorageError> {
        use crate::schema::payloads;

        let transaction = payload.transaction();
        let instructions = json!(&transaction.instructions()).to_string();

        let signature = transaction.signature();

        let public_nonce = Vec::from(signature.signature().get_public_nonce().as_bytes());
        let scalar = Vec::from(signature.signature().get_signature().as_bytes());

        let fee = transaction.fee() as i64;
        let sender_public_key = Vec::from(transaction.sender_public_key().as_bytes());

        let meta = json!(transaction.meta()).to_string();

        let payload_id = Vec::from(payload.to_id().as_bytes());

        let new_row = NewPayload {
            payload_id,
            instructions,
            public_nonce,
            scalar,
            fee,
            sender_address: sender_public_key,
            meta,
            result: None,
        };

        match diesel::insert_into(payloads::table)
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

    fn set_leaf_node(
        &mut self,
        payload_id: PayloadId,
        shard_id: ShardId,
        node: TreeNodeHash,
        payload_height: NodeHeight,
        height: NodeHeight,
    ) -> Result<(), StorageError> {
        use crate::schema::leaf_nodes;

        let leaf_node: Option<DbLeafNode> = leaf_nodes::table
            .filter(leaf_nodes::shard_id.eq(shard_id.as_bytes()))
            .filter(leaf_nodes::payload_id.eq(payload_id.as_bytes()))
            .order_by(leaf_nodes::node_height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get leaf node: {}", e),
            })?;

        match leaf_node {
            Some(leaf_node) => {
                diesel::update(leaf_nodes::table)
                    .set((
                        leaf_nodes::tree_node_hash.eq(node.as_bytes()),
                        leaf_nodes::node_height.eq(height.as_u64() as i64),
                        leaf_nodes::payload_height.eq(payload_height.as_u64() as i64),
                        leaf_nodes::shard_id.eq(shard_id.as_bytes()),
                        leaf_nodes::payload_id.eq(payload_id.as_bytes()),
                    ))
                    .filter(leaf_nodes::id.eq(leaf_node.id))
                    .execute(self.transaction.connection())
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("Update leaf node: {}", e),
                    })?;
            },
            None => {
                let new_row = NewLeafNode {
                    shard_id: shard_id.as_bytes().to_vec(),
                    payload_id: payload_id.as_bytes().to_vec(),
                    tree_node_hash: node.as_bytes().to_vec(),
                    payload_height: payload_height.as_u64() as i64,
                    node_height: height.as_u64() as i64,
                };

                // TODO: verify that we just need to add a new row to the table, instead
                // of possibly updating an existing row
                diesel::insert_into(leaf_nodes::table)
                    .values(&new_row)
                    .execute(self.transaction.connection())
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("Update leaf node error: {}", e),
                    })?;
            },
        }

        Ok(())
    }

    fn save_node(&mut self, node: HotStuffTreeNode<PublicKey, TariDanPayload>) -> Result<(), StorageError> {
        use crate::schema::nodes;

        let node_hash = Vec::from(node.hash().as_bytes());
        let parent_node_hash = Vec::from(node.parent().as_bytes());

        let height = node.height().as_u64() as i64;
        let shard = Vec::from(node.shard().as_bytes());

        let payload_id = Vec::from(node.payload_id().as_bytes());
        let payload_height = node.payload_height().as_u64() as i64;

        let local_pledges = json!(&node.local_pledge()).to_string();

        let epoch = node.epoch().as_u64() as i64;
        let proposed_by = Vec::from(node.proposed_by().as_bytes());

        let justify = serde_json::to_string_pretty(node.justify()).unwrap();

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

    fn set_locked(
        &mut self,
        payload_id: PayloadId,
        shard: ShardId,
        node_hash: TreeNodeHash,
        node_height: NodeHeight,
    ) -> Result<(), StorageError> {
        use crate::schema::lock_node_and_heights;

        let new_row = NewLockNodeAndHeight {
            payload_id: payload_id.as_bytes().to_vec(),
            shard_id: shard.as_bytes().to_vec(),
            tree_node_hash: node_hash.as_bytes().to_vec(),
            node_height: node_height.as_u64() as i64,
        };

        diesel::insert_into(lock_node_and_heights::table)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set locked error: {}", e),
            })?;
        Ok(())
    }

    fn set_last_executed_height(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
        height: NodeHeight,
    ) -> Result<(), StorageError> {
        use crate::schema::last_executed_heights;

        let new_row = NewLastExecutedHeight {
            shard_id: shard.as_bytes().to_vec(),
            payload_id: payload_id.as_bytes().to_vec(),
            node_height: height.as_u64() as i64,
        };

        diesel::insert_into(last_executed_heights::table)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set last executed height error: {}", e),
            })?;
        Ok(())
    }

    fn save_substate_changes(
        &mut self,
        node: HotStuffTreeNode<PublicKey, TariDanPayload>,
        changes: &[SubstateState],
    ) -> Result<(), StorageError> {
        use crate::schema::substates;

        let current_state = substates::table
            .filter(substates::shard_id.eq(node.shard().as_bytes()))
            .first::<Substate>(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Save substate changes error. Could not retrieve current state: {}", e),
            })?;

        for st_change in changes {
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

                    let pretty_data = serde_json::to_string_pretty(d).map_err(|source| StorageError::SerdeJson {
                        source,
                        operation: "save_substate_changes".to_string(),
                        data: "substate data".to_string(),
                    })?;
                    let new_row = NewSubstate {
                        shard_id: node.shard().as_bytes().to_vec(),
                        version: d.version().into(),
                        data: pretty_data,
                        created_by_payload_id: node.payload_id().as_bytes().to_vec(),
                        created_justify: "".to_string(),
                        created_node_hash: node.hash().as_bytes().to_vec(),
                        created_height: node.height().as_u64() as i64,
                    };
                    diesel::insert_into(substates::table)
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
                                reason: "Save substate changes error. Cannot destroy a substate that has already been \
                                         destroyed"
                                    .to_string(),
                            });
                        }

                        let rows_affected = diesel::update(substates::table.filter(substates::id.eq(s.id)))
                            .set((
                                substates::destroyed_by_payload_id.eq(node.payload_id().as_bytes()),
                                substates::destroyed_justify.eq(serde_json::to_string_pretty(node.justify()).unwrap()),
                                substates::destroyed_height.eq(node.height().as_u64() as i64),
                                substates::destroyed_node_hash.eq(node.hash().as_bytes()),
                                substates::destroyed_timestamp.eq(now),
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

        Ok(())
    }

    fn insert_substates(&mut self, substate_data: SubstateShardData) -> Result<(), StorageError> {
        use crate::schema::substates;

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

        diesel::insert_into(substates::table)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Insert substates: {}", e),
            })?;

        Ok(())
    }

    fn set_last_voted_height(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
        height: NodeHeight,
    ) -> Result<(), StorageError> {
        use crate::schema::last_voted_heights;

        let new_row = NewLastVotedHeight {
            shard_id: shard.as_bytes().to_vec(),
            payload_id: payload_id.as_bytes().to_vec(),
            node_height: height.as_u64() as i64,
        };

        diesel::insert_into(last_voted_heights::table)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set last voted height: {}", e),
            })?;
        Ok(())
    }

    fn save_leader_proposals(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        payload_height: NodeHeight,
        node: HotStuffTreeNode<PublicKey, TariDanPayload>,
    ) -> Result<(), StorageError> {
        use crate::schema::leader_proposals;

        let shard = Vec::from(shard.as_bytes());
        let payload = Vec::from(payload.as_bytes());
        let payload_height = payload_height.as_u64() as i64;
        let node_hash = node.hash().as_bytes().to_vec();
        let node = serde_json::to_string_pretty(&node).unwrap();

        let new_row = NewLeaderProposal {
            shard_id: shard,
            payload_id: payload,
            payload_height,
            node_hash,
            hotstuff_tree_node: node,
        };

        match diesel::insert_into(leader_proposals::table)
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

    fn save_received_vote_for(
        &mut self,
        from: PublicKey,
        node_hash: TreeNodeHash,
        vote_message: VoteMessage,
    ) -> Result<(), StorageError> {
        use crate::schema::received_votes;

        let vote_message = serde_json::to_string_pretty(&vote_message).unwrap();

        let new_row = NewReceivedVote {
            tree_node_hash: node_hash.as_bytes().to_vec(),
            address: from.as_bytes().to_vec(),
            vote_message,
        };

        diesel::insert_into(received_votes::table)
            .values(&new_row)
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Save received voted for: {}", e),
            })?;

        Ok(())
    }

    fn update_payload_result(
        &self,
        requested_payload_id: &PayloadId,
        result: FinalizeResult,
    ) -> Result<(), StorageError> {
        use crate::schema::payloads;

        let result_json = serde_json::to_string_pretty(&result).map_err(|_| StorageError::EncodingError)?;

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

    // -------------------------------- Pledges -------------------------------- //

    fn pledge_object(
        &mut self,
        shard: ShardId,
        payload: PayloadId,
        current_height: NodeHeight,
    ) -> Result<ObjectPledge, StorageError> {
        use crate::schema::shard_pledges;

        let existing_pledge: Option<DbShardPledge> = shard_pledges::table
            .filter(shard_pledges::shard_id.eq(shard.as_bytes()))
            .filter(shard_pledges::is_active.eq(true))
            .order_by(shard_pledges::created_height.desc())
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
            shard_id: shard.as_bytes().to_vec(),
            created_height: current_height.as_u64() as i64,
            pledged_to_payload_id: payload.as_bytes().to_vec(),
            is_active: true,
        };
        let num_affected = diesel::insert_into(shard_pledges::table)
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

        let new_pledge: DbShardPledge = shard_pledges::table
            .filter(
                shard_pledges::shard_id
                    .eq(shard.as_bytes())
                    .and(shard_pledges::is_active.eq(true))
                    .and(shard_pledges::pledged_to_payload_id.eq(payload.as_bytes())),
            )
            .order_by(shard_pledges::id.desc())
            .first(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Pledge object error: {}", e),
            })?;

        self.create_pledge(shard, new_pledge)
    }

    fn complete_pledges(
        &self,
        shard: ShardId,
        payload: PayloadId,
        node_hash: &TreeNodeHash,
    ) -> Result<(), StorageError> {
        use crate::schema::shard_pledges;
        let rows_affected = diesel::update(shard_pledges::table)
            .filter(shard_pledges::shard_id.eq(shard.as_bytes()))
            .filter(shard_pledges::pledged_to_payload_id.eq(payload.as_bytes()))
            .filter(shard_pledges::is_active.eq(true))
            .set((
                shard_pledges::is_active.eq(false),
                shard_pledges::completed_by_tree_node_hash.eq(node_hash.as_bytes()),
                shard_pledges::updated_timestamp.eq(now),
            ))
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Complete pledges: {}", e),
            })?;
        if rows_affected == 0 {
            return Err(StorageError::QueryError {
                reason: format!(
                    "Complete pledges: No pledges found to complete for shard {}, payload_id: {}, is_active: true",
                    shard, payload
                ),
            });
        }
        Ok(())
    }

    fn abandon_pledges(
        &self,
        shard: ShardId,
        payload_id: PayloadId,
        node_hash: &TreeNodeHash,
    ) -> Result<(), StorageError> {
        use crate::schema::shard_pledges;
        let rows_affected = diesel::update(shard_pledges::table)
            .filter(shard_pledges::shard_id.eq(shard.as_bytes()))
            .filter(shard_pledges::pledged_to_payload_id.eq(payload_id.as_bytes()))
            .filter(shard_pledges::is_active.eq(true))
            .set((
                shard_pledges::is_active.eq(false),
                shard_pledges::abandoned_by_tree_node_hash.eq(node_hash.as_bytes()),
                shard_pledges::updated_timestamp.eq(now),
            ))
            .execute(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Complete pledges: {}", e),
            })?;
        if rows_affected == 0 {
            return Err(StorageError::QueryError {
                reason: format!(
                    "Abandon pledges: No pledges found to abandon for shard {}, payload_id: {}, is_active: true",
                    shard, payload_id
                ),
            });
        }
        Ok(())
    }
}

impl<'a> Deref for SqliteShardStoreWriteTransaction<'a> {
    type Target = SqliteShardStoreReadTransaction<'a>;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

impl Drop for SqliteShardStoreWriteTransaction<'_> {
    fn drop(&mut self) {
        if !self.is_complete {
            warn!(
                target: LOG_TARGET,
                "Shard store write transaction was not committed/rolled back"
            );
        }
    }
}
