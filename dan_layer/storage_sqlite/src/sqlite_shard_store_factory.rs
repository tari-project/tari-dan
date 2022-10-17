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
    convert::{TryFrom, TryInto},
    path::PathBuf,
};

use diesel::{
    prelude::*,
    result::{DatabaseErrorKind, Error},
    SqliteConnection,
};
use log::{debug, warn};
use serde_json::json;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_dan_common_types::{Epoch, ObjectId, PayloadId, ShardId, SubstateState};
use tari_dan_core::{
    models::{
        vote_message::VoteMessage,
        HotStuffTreeNode,
        NodeHeight,
        ObjectPledge,
        Payload,
        QuorumCertificate,
        TariDanPayload,
        TreeNodeHash,
    },
    storage::{
        deserialize,
        shard_store::{ShardStoreFactory, ShardStoreTransaction},
        StorageError,
    },
};
use tari_dan_engine::transaction::{Transaction, TransactionMeta};
use tari_engine_types::{instruction::Instruction, signature::InstructionSignature};
use tari_utilities::{hex::Hex, ByteArray};

use crate::{
    error::SqliteStorageError,
    models::{
        high_qc::{HighQc, NewHighQc},
        last_executed_height::{LastExecutedHeight, NewLastExecutedHeight},
        last_voted_height::{LastVotedHeight, NewLastVotedHeight},
        leader_proposals::{LeaderProposal, NewLeaderProposal},
        leaf_nodes::{LeafNode, NewLeafNode},
        lock_node_and_height::{LockNodeAndHeight, NewLockNodeAndHeight},
        node::{NewNode, Node},
        objects::{NewObject, Object},
        payload::{NewPayload, Payload as SqlPayload},
        received_votes::{NewReceivedVote, ReceivedVote},
        substate_change::NewSubStateChange,
    },
    schema::{
        high_qcs::{dsl::high_qcs, shard_id},
        last_executed_heights::dsl::last_executed_heights,
        last_voted_heights::dsl::last_voted_heights,
        leader_proposals::dsl::leader_proposals,
        leaf_nodes::dsl::leaf_nodes,
        lock_node_and_heights::dsl::lock_node_and_heights,
        nodes::dsl::nodes as table_nodes,
        objects::dsl::objects,
        payloads::dsl::payloads,
        received_votes::dsl::received_votes,
        substate_changes::dsl::substate_changes,
    },
};

const LOG_TARGET: &str = "tari::dan::storage::sqlite::shard_store";

#[derive(Debug, Clone)]
pub struct SqliteShardStoreFactory {
    url: PathBuf,
}

impl SqliteShardStoreFactory {
    pub fn try_create(url: PathBuf) -> Result<Self, StorageError> {
        let connection =
            SqliteConnection::establish(&url.clone().into_os_string().into_string().unwrap()).map_err(|e| {
                StorageError::ConnectionError {
                    reason: format!("Try create error: {}", e),
                }
            })?;

        embed_migrations!("./migrations");
        embedded_migrations::run(&connection).map_err(|e| StorageError::ConnectionError {
            reason: format!("Try create error: {}", e),
        })?;
        Ok(Self { url })
    }
}
impl ShardStoreFactory for SqliteShardStoreFactory {
    type Addr = PublicKey;
    type Payload = TariDanPayload;
    type Transaction = SqliteShardStoreTransaction;

    fn create_tx(&self) -> Result<Self::Transaction, StorageError> {
        match SqliteConnection::establish(&self.url.clone().into_os_string().into_string().unwrap()) {
            Ok(connection) => {
                connection
                    .execute("PRAGMA foreign_keys = ON;   BEGIN TRANSACTION;")
                    .map_err(|source| SqliteStorageError::DieselError {
                        source,
                        operation: "set pragma".to_string(),
                    })?;
                Ok(SqliteShardStoreTransaction::new(connection))
            },
            Err(err) => Err(SqliteStorageError::from(err).into()),
        }
    }
}

pub struct SqliteShardStoreTransaction {
    connection: SqliteConnection,
}

impl SqliteShardStoreTransaction {
    fn new(connection: SqliteConnection) -> Self {
        Self { connection }
    }
}

impl ShardStoreTransaction<PublicKey, TariDanPayload> for SqliteShardStoreTransaction {
    type Error = StorageError;

    fn commit(&mut self) -> Result<(), Self::Error> {
        self.connection
            .execute("COMMIT TRANSACTION;")
            .map_err(|source| StorageError::QueryError {
                reason: format!("Commit transaction error: {0}", source),
            })?;
        Ok(())
    }

    fn update_high_qc(&mut self, shard: ShardId, qc: QuorumCertificate) -> Result<(), Self::Error> {
        // update all others for this shard to highest == false
        let shard = Vec::from(shard.0);

        let new_row = NewHighQc {
            shard_id: shard,
            height: qc.local_node_height().as_u64() as i64,
            qc_json: json!(qc).to_string(),
        };
        diesel::insert_into(high_qcs)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError {
                reason: format!("Update qc error: {0}", e),
            })?;

        Ok(())
    }

    fn get_leaf_node(&self, shard: ShardId) -> Result<(TreeNodeHash, NodeHeight), Self::Error> {
        use crate::schema::leaf_nodes::{node_height, shard_id};
        let leaf_node: Option<LeafNode> = leaf_nodes
            .filter(shard_id.eq(Vec::from(shard.as_bytes())))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get leaf node: {}", e),
            })?;
        if let Some(leaf_node) = leaf_node {
            Ok((
                TreeNodeHash::try_from(leaf_node.tree_node_hash).unwrap(),
                NodeHeight(leaf_node.node_height as u64),
            ))
        } else {
            // if no leaves, return genesis
            Ok((TreeNodeHash::zero(), NodeHeight(0)))
        }
    }

    fn update_leaf_node(
        &mut self,
        shard: tari_dan_common_types::ShardId,
        node: TreeNodeHash,
        height: NodeHeight,
    ) -> Result<(), Self::Error> {
        let shard = Vec::from(shard.0);
        let tree_node_hash = Vec::from(node.as_bytes());
        let node_height = height
            .0
            .try_into()
            .map_err(|_| Self::Error::InvalidIntegerCast)
            .unwrap();

        let new_row = NewLeafNode {
            shard_id: shard,
            tree_node_hash,
            node_height,
        };

        // TODO: verify that we just need to add a new row to the table, instead
        // of possibly updating an existing row
        diesel::insert_into(leaf_nodes)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError {
                reason: format!("Update leaf node error: {}", e),
            })?;

        Ok(())
    }

    fn get_high_qc_for(&self, shard: ShardId) -> Result<QuorumCertificate, Self::Error> {
        dbg!("get high qc");
        use crate::schema::high_qcs::height;
        let qc: Option<HighQc> = high_qcs
            .filter(shard_id.eq(Vec::from(shard.0)))
            .order_by(height.desc())
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get high qc error: {}", e),
            })?;
        if let Some(qc) = qc {
            Ok(serde_json::from_str(&qc.qc_json)?)
        } else {
            Ok(QuorumCertificate::genesis())
        }
    }

    fn get_payload(&self, id: &PayloadId) -> Result<TariDanPayload, Self::Error> {
        dbg!("get payload");
        use crate::schema::payloads::payload_id;

        let payload: Option<SqlPayload> = payloads
            .filter(payload_id.eq(Vec::from(id.as_slice())))
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get payload error: {}", e),
            })?;

        if let Some(payload) = payload {
            let instructions: Vec<Instruction> = serde_json::from_str(&payload.instructions)?;

            let fee: u64 = payload.fee.try_into().map_err(|_| Self::Error::InvalidIntegerCast)?;

            let public_nonce =
                PublicKey::from_vec(&payload.public_nonce).map_err(Self::Error::InvalidByteArrayConversion)?;
            let signature =
                PrivateKey::from_bytes(payload.scalar.as_slice()).map_err(Self::Error::InvalidByteArrayConversion)?;

            let signature: InstructionSignature =
                InstructionSignature::try_from(Signature::new(public_nonce, signature)).map_err(|e| {
                    Self::Error::InvalidTypeCasting {
                        reason: format!("Get payload error: {}", e),
                    }
                })?;

            let sender_public_key =
                PublicKey::from_vec(&payload.sender_public_key).map_err(Self::Error::InvalidByteArrayConversion)?;
            let meta: TransactionMeta = serde_json::from_str(&payload.meta)?;

            let transaction = Transaction::new(fee, instructions, signature, sender_public_key, meta);

            Ok(TariDanPayload::new(transaction))
        } else {
            Err(Self::Error::NotFound {
                item: "payload".to_string(),
                key: id.to_string(),
            })
        }
    }

    fn set_payload(&mut self, payload: TariDanPayload) -> Result<(), Self::Error> {
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
        };

        match diesel::insert_into(payloads).values(&new_row).execute(&self.connection) {
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
                        return Err(Self::Error::QueryError {
                            reason: format!("Set payload error: {}", err),
                        })
                    },
                }
            },
        }
        Ok(())
    }

    fn get_node(&self, hash: &TreeNodeHash) -> Result<HotStuffTreeNode<PublicKey>, Self::Error> {
        if hash == &TreeNodeHash::zero() {
            return Ok(HotStuffTreeNode::genesis());
        }

        use crate::schema::nodes::node_hash;

        let hash = Vec::from(hash.as_bytes());
        // TODO: Do we need to add an index to the table to order by `height` and `payload_height`
        // more efficiently ?
        let node: Option<Node> = table_nodes
            .filter(node_hash.eq(hash.clone()))
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get node error: {}", e),
            })?;

        if let Some(node) = node {
            let mut parent = [0u8; 32];
            parent.copy_from_slice(node.parent_node_hash.as_slice());

            let parent = TreeNodeHash::from(parent);
            let hgt: u64 = node.height.try_into().map_err(|_| Self::Error::InvalidIntegerCast)?;

            let shard = ShardId::from_bytes(&node.shard)?;
            let payload = PayloadId::try_from(node.payload_id)?;

            let payload_hgt: u64 = node
                .payload_height
                .try_into()
                .map_err(|_| Self::Error::InvalidIntegerCast)?;
            let local_pledges: Vec<ObjectPledge> = serde_json::from_str(&node.local_pledges)?;

            let epoch: u64 = node.epoch.try_into().map_err(|_| Self::Error::InvalidIntegerCast)?;
            let proposed_by =
                PublicKey::from_vec(&node.proposed_by).map_err(Self::Error::InvalidByteArrayConversion)?;

            let justify: QuorumCertificate = serde_json::from_str(&node.justify)?;

            Ok(HotStuffTreeNode::new(
                parent,
                shard,
                NodeHeight(hgt),
                payload,
                NodeHeight(payload_hgt),
                local_pledges,
                Epoch(epoch),
                proposed_by,
                justify,
            ))
        } else {
            Err(Self::Error::NotFound {
                item: "node".to_string(),
                key: hash.to_hex(),
            })
        }
    }

    fn save_node(&mut self, node: HotStuffTreeNode<PublicKey>) -> Result<(), Self::Error> {
        let node_hash = Vec::from(node.hash().as_bytes());
        let parent_node_hash = Vec::from(node.parent().as_bytes());

        let height = node
            .height()
            .0
            .try_into()
            .map_err(|_| Self::Error::InvalidIntegerCast)?;
        let shard = Vec::from(node.shard().as_bytes());

        let payload_id = Vec::from(node.payload().as_bytes());
        let payload_height = node.payload_height().as_u64() as i64;

        let local_pledges = json!(&node.local_pledges()).to_string();

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

        match diesel::insert_into(table_nodes)
            .values(&new_row)
            .execute(&self.connection)
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
                    return Err(Self::Error::QueryError {
                        reason: format!("Save node error: {}", err),
                    })
                },
            },
        }

        Ok(())
    }

    fn get_locked_node_hash_and_height(&self, shard: ShardId) -> Result<(TreeNodeHash, NodeHeight), Self::Error> {
        use crate::schema::lock_node_and_heights::{node_height, shard_id};

        let shard = Vec::from(shard.0);

        let lock_node_hash_and_height: Option<LockNodeAndHeight> = lock_node_and_heights
            .filter(shard_id.eq(shard))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get locked node hash and height error: {}", e),
            })?;

        if let Some(data) = lock_node_hash_and_height {
            let mut tree_node_hash = [0u8; 32];
            tree_node_hash.copy_from_slice(data.tree_node_hash.as_slice());
            let tree_node_hash = TreeNodeHash::from(tree_node_hash);

            let height: u64 = data
                .node_height
                .try_into()
                .map_err(|_| Self::Error::InvalidIntegerCast)?;

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
    ) -> Result<(), Self::Error> {
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
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set locked error: {}", e),
            })?;
        Ok(())
    }

    fn pledge_object(
        &mut self,
        shard: ShardId,
        object: ObjectId,
        payload: PayloadId,
        current_height: NodeHeight,
    ) -> Result<ObjectPledge, Self::Error> {
        use crate::schema::objects::{node_height, object_id, shard_id};

        let shard = Vec::from(shard.as_bytes());
        let f_object = Vec::from(object.as_bytes());
        let f_payload = Vec::from(payload.as_slice());

        let db_object: Option<Object> = objects
            .filter(shard_id.eq(&shard).and(object_id.eq(&f_object)))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Pledge object error: {}", e),
            })?;

        let mut db_current_state = SubstateState::DoesNotExist;
        if let Some(obj) = db_object {
            let object_pledge = obj.object_pledge;

            if object_pledge.is_empty() {
                panic!("Item does not exist");
            }

            let r: ObjectPledge = serde_json::from_str(&object_pledge)?;
            if r.pledged_until > current_height {
                return Ok(r);
            }
            db_current_state = serde_json::from_str(&obj.current_state)?;
        }

        // otherwise save pledge
        let pledge = ObjectPledge {
            object_id: object,
            current_state: db_current_state.clone(),
            pledged_to_payload: payload,
            pledged_until: current_height + NodeHeight(4),
        };

        let new_row = NewObject {
            shard_id: shard,
            object_id: f_object,
            payload_id: f_payload,
            node_height: current_height.as_u64() as i64,
            object_pledge: json!(&pledge).to_string(),
            current_state: json!(&db_current_state).to_string(),
        };
        diesel::insert_into(objects)
            .values(new_row)
            .execute(&self.connection)
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Pledge object error: {}", e),
            })?;

        // entry.1 = Some(pledge.clone());
        Ok(pledge)
    }

    fn set_last_executed_height(&mut self, shard: ShardId, height: NodeHeight) -> Result<(), Self::Error> {
        let shard = Vec::from(shard.as_bytes());
        let node_height = height.as_u64() as i64;

        let new_row = NewLastExecutedHeight {
            shard_id: shard,
            node_height,
        };

        diesel::insert_into(last_executed_heights)
            .values(&new_row)
            .execute(&self.connection)
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set last executed height error: {}", e),
            })?;
        Ok(())
    }

    fn get_last_executed_height(&self, shard: ShardId) -> Result<NodeHeight, Self::Error> {
        use crate::schema::last_executed_heights::{node_height, shard_id};

        let last_executed_height: Option<LastExecutedHeight> = last_executed_heights
            .filter(shard_id.eq(Vec::from(shard.0)))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get last executed height: {}", e),
            })?;

        if let Some(last_exec_height) = last_executed_height {
            let height = last_exec_height
                .node_height
                .try_into()
                .map_err(|_| Self::Error::InvalidIntegerCast)?;
            Ok(NodeHeight(height))
        } else {
            Ok(NodeHeight(0))
        }
    }

    fn save_substate_changes(
        &mut self,
        changes: HashMap<ShardId, Option<SubstateState>>,
        node: TreeNodeHash,
    ) -> Result<(), Self::Error> {
        for (sid, st_ch) in &changes {
            let shard = Vec::from(sid.as_bytes());
            let substate_change = if let Some(st_ch) = st_ch {
                json!(st_ch).to_string()
            } else {
                "".to_string()
            };

            let new_row = NewSubStateChange {
                shard_id: shard,
                tree_node_hash: Vec::from(node.as_bytes()),
                substate_change,
            };

            diesel::insert_into(substate_changes)
                .values(&new_row)
                .execute(&self.connection)
                .map_err(|e| Self::Error::QueryError {
                    reason: format!("Save substate change: {}", e),
                })?;
        }
        Ok(())
    }

    fn get_state_inventory(&self, start_shard: ShardId, end_shard: ShardId) -> Result<Vec<ShardId>, Self::Error> {
        use crate::schema::substate_changes::shard_id;

        let substate_states: Option<Vec<crate::models::substate_change::SubstateChange>> = substate_changes
            .filter(
                shard_id
                    .gt(Vec::from(start_shard.as_bytes()))
                    .and(shard_id.lt(Vec::from(end_shard.as_bytes()))),
            )
            .get_results(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get substate change error: {}", e),
            })
            .unwrap();

        if let Some(substate_states) = substate_states {
            substate_states
                .iter()
                .map(|ss| deserialize::<ShardId>(ss.shard_id.as_slice()))
                .collect::<Result<Vec<_>, _>>()
        } else {
            return Err(Self::Error::NotFound {
                item: "substate".to_string(),
                key: format!("{}, {}", start_shard, end_shard),
            });
        }
    }

    fn get_substates_changes_by_range(
        &self,
        start_shard: ShardId,
        end_shard: ShardId,
    ) -> Result<Vec<SubstateState>, Self::Error> {
        use crate::schema::substate_changes::shard_id;

        let substate_states: Option<Vec<crate::models::substate_change::SubstateChange>> = substate_changes
            .filter(
                shard_id
                    .gt(Vec::from(start_shard.as_bytes()))
                    .and(shard_id.lt(Vec::from(end_shard.as_bytes()))),
            )
            .get_results(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get substate change error: {}", e),
            })
            .unwrap();

        if let Some(substate_states) = substate_states {
            substate_states
                .iter()
                .map(|ss| {
                    serde_json::from_str::<SubstateState>(ss.substate_change.as_str())
                        .map_err(|e| StorageError::SerdeJson(e))
                })
                .collect::<Result<_, _>>()
        } else {
            return Err(Self::Error::NotFound {
                item: "substate".to_string(),
                key: format!("{}, {}", start_shard, end_shard),
            });
        }
    }

    fn get_last_voted_height(&self, shard: ShardId) -> Result<NodeHeight, Self::Error> {
        use crate::schema::last_voted_heights::{node_height, shard_id};

        let last_vote: Option<LastVotedHeight> = last_voted_heights
            .filter(shard_id.eq(Vec::from(shard.as_bytes())))
            .order_by(node_height.desc())
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get last voted height error: {}", e),
            })?;

        if let Some(last_vote_height) = last_vote {
            let height = last_vote_height
                .node_height
                .try_into()
                .map_err(|_| Self::Error::InvalidIntegerCast)?;
            Ok(NodeHeight(height))
        } else {
            Ok(NodeHeight(0))
        }
    }

    fn set_last_voted_height(&mut self, shard: ShardId, height: NodeHeight) -> Result<(), Self::Error> {
        let shard = Vec::from(shard.as_bytes());
        let height = height.as_u64() as i64;

        let new_row = NewLastVotedHeight {
            shard_id: shard,
            node_height: height,
        };

        diesel::insert_into(last_voted_heights)
            .values(&new_row)
            .execute(&self.connection)
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
    ) -> Result<Option<HotStuffTreeNode<PublicKey>>, Self::Error> {
        use crate::schema::leader_proposals::{payload_height as s_payload_height, payload_id, shard_id};

        let payload_vote: Option<LeaderProposal> = leader_proposals
            .filter(
                payload_id
                    .eq(Vec::from(payload.as_slice()))
                    .and(shard_id.eq(Vec::from(shard.as_bytes())))
                    .and(s_payload_height.eq(payload_height.as_u64() as i64)),
            )
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get payload vote: {}", e),
            })?;

        if let Some(payload_vote) = payload_vote {
            let hot_stuff_tree_node: HotStuffTreeNode<PublicKey> =
                serde_json::from_str(&payload_vote.hotstuff_tree_node)?;
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
        node: HotStuffTreeNode<PublicKey>,
    ) -> Result<(), Self::Error> {
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
            .execute(&self.connection)
        {
            Ok(_) => Ok(()),
            Err(e) => match e {
                diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) => {
                    warn!(target: LOG_TARGET, "Leader proposal already exists");
                    Ok(())
                },
                _ => Err(Self::Error::QueryError {
                    reason: format!("Save leader proposal: {}", e),
                }),
            },
        }
        // .map_err(|e| StorageError::QueryError {
        //     reason: format!("Save payload vote error: {}", e),
        // })?;
    }

    fn has_vote_for(&self, from: &PublicKey, node_hash: TreeNodeHash, shard: ShardId) -> Result<bool, Self::Error> {
        use crate::schema::received_votes::{address, shard_id, tree_node_hash};

        let vote: Option<ReceivedVote> = received_votes
            .filter(
                shard_id
                    .eq(Vec::from(shard.as_bytes()))
                    .and(tree_node_hash.eq(Vec::from(node_hash.as_bytes())))
                    .and(address.eq(Vec::from(from.as_bytes()))),
            )
            .first(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
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
    ) -> Result<usize, Self::Error> {
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
            .execute(&self.connection)
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
            .first(&self.connection)
            .map_err(|source| StorageError::QueryError {
                reason: format!("Save received vote: {0}", source),
            })?;

        count.try_into().map_err(|_| Self::Error::InvalidIntegerCast)
    }

    fn get_received_votes_for(&self, node_hash: TreeNodeHash, shard: ShardId) -> Result<Vec<VoteMessage>, Self::Error> {
        use crate::schema::received_votes::{shard_id, tree_node_hash};

        let filtered_votes: Option<Vec<ReceivedVote>> = received_votes
            .filter(
                shard_id
                    .eq(Vec::from(shard.as_bytes()))
                    .and(tree_node_hash.eq(Vec::from(node_hash.as_bytes()))),
            )
            .get_results(&self.connection)
            .optional()
            .map_err(|e| Self::Error::QueryError {
                reason: format!("Get received vote for: {}", e),
            })?;

        if let Some(filtered_votes) = filtered_votes {
            let v = filtered_votes
                .iter()
                .map(|v| serde_json::from_str::<VoteMessage>(&v.vote_message))
                .collect::<Result<_, _>>()?;
            Ok(v)
        } else {
            Ok(vec![])
        }
    }
}
