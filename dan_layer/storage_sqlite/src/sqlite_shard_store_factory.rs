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
    fmt,
    fs::create_dir_all,
    ops::{Deref, DerefMut},
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
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use log::{debug, error, warn};
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
use tari_dan_storage::{
    models::{
        ClaimLeaderFees,
        CurrentLeaderStates,
        HotStuffTreeNode,
        LeafNode,
        Payload,
        PayloadResult,
        RecentTransaction,
        SQLSubstate,
        SQLTransaction,
        SubstateShardData,
        TariDanPayload,
        VoteMessage,
    },
    ShardStore,
    ShardStoreReadTransaction,
    ShardStoreWriteTransaction,
    StorageError,
};
use tari_engine_types::substate::SubstateAddress;
use tari_transaction::{InstructionSignature, Transaction, TransactionMeta};
use tari_utilities::{
    hex::{to_hex, Hex},
    ByteArray,
};

use crate::{
    error::SqliteStorageError,
    models::{
        current_state::{CurrentLeaderState, NewCurrentLeaderState},
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
    #[diesel(sql_type = Binary)]
    pub payload_id: Vec<u8>,
    #[diesel(sql_type = Timestamp)]
    pub timestamp: NaiveDateTime,
    #[diesel(sql_type = Text)]
    pub meta: String,
    #[diesel(sql_type = Text)]
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
    #[diesel(sql_type = Binary)]
    pub node_hash: Vec<u8>,
    #[diesel(sql_type = Binary)]
    pub parent_node_hash: Vec<u8>,
    #[diesel(sql_type = Binary)]
    pub shard: Vec<u8>,
    #[diesel(sql_type = BigInt)]
    pub height: i64,
    #[diesel(sql_type = BigInt)]
    pub payload_height: i64,
    #[diesel(sql_type = BigInt)]
    pub total_votes: i64,
    #[diesel(sql_type = BigInt)]
    pub total_leader_proposals: i64,
    #[diesel(sql_type = Timestamp)]
    pub timestamp: NaiveDateTime,
    #[diesel(sql_type = Text)]
    pub justify: String,
    #[diesel(sql_type = Binary)]
    pub proposed_by: Vec<u8>,
    #[diesel(sql_type = BigInt)]
    pub leader_round: i64,
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
            proposed_by: transaction.proposed_by,
            leader_round: transaction.leader_round,
        }
    }
}

#[derive(Debug, QueryableByName)]
pub struct QueryableSubstate {
    #[diesel(sql_type = Binary)]
    pub shard_id: Vec<u8>,
    #[diesel(sql_type = Text)]
    pub address: String,
    #[diesel(sql_type = BigInt)]
    pub version: i64,
    #[diesel(sql_type = Text)]
    pub data: String,
    #[diesel(sql_type = Text)]
    pub created_justify: String,
    #[diesel(sql_type = Binary)]
    pub created_node_hash: Vec<u8>,
    #[diesel(sql_type = Nullable<Text>)]
    pub destroyed_justify: Option<String>,
    #[diesel(sql_type = Nullable<Binary>)]
    pub destroyed_node_hash: Option<Vec<u8>>,
}

impl From<QueryableSubstate> for SQLSubstate {
    fn from(transaction: QueryableSubstate) -> Self {
        Self {
            shard_id: transaction.shard_id,
            address: transaction.address,
            version: transaction.version,
            data: transaction.data,
            created_justify: transaction.created_justify,
            destroyed_justify: transaction.destroyed_justify,
        }
    }
}

#[derive(Debug, QueryableByName)]
pub struct QueryableClaimFeesSubstate {
    #[diesel(sql_type = Text)]
    pub justify_leader_public_key: String,
    #[diesel(sql_type = BigInt)]
    pub created_at_epoch: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    pub destroyed_at_epoch: Option<i64>,
    #[diesel(sql_type = BigInt)]
    pub fee_paid_for_created_justify: i64,
    #[diesel(sql_type = BigInt)]
    pub fee_paid_for_destroyed_justify: i64,
}

impl From<QueryableClaimFeesSubstate> for ClaimLeaderFees {
    fn from(transaction: QueryableClaimFeesSubstate) -> Self {
        Self {
            justify_leader_public_key: transaction.justify_leader_public_key,
            created_at_epoch: transaction.created_at_epoch,
            destroyed_at_epoch: transaction.destroyed_at_epoch,
            fee_paid_for_created_justify: transaction.fee_paid_for_created_justify,
            fee_paid_for_destroyed_justify: transaction.fee_paid_for_destroyed_justify,
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
        let mut connection = SqliteConnection::establish(&database_url).map_err(SqliteStorageError::from)?;

        const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");
        connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|source| SqliteStorageError::MigrationError { source })?;

        sql_query("PRAGMA foreign_keys = ON;")
            .execute(&mut connection)
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "set pragma".to_string(),
            })?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }
}

// we mock the Debug implementation because "SqliteConnection" does not implement the Debug trait
impl fmt::Debug for SqliteShardStore {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SqliteShardStore")
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

    fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.connection()
    }

    fn map_substate_to_shard_data(ss: &Substate) -> Result<SubstateShardData, StorageError> {
        Ok(SubstateShardData::new(
            ShardId::try_from(ss.shard_id.clone())?,
            ss.address.parse().map_err(|_| StorageError::DecodingError)?,
            ss.version as u32,
            serde_json::from_str(&ss.data).unwrap(),
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
            ss.created_justify
                .as_ref()
                .map(|justify| serde_json::from_str(justify.as_str()).unwrap()),
            ss.destroyed_justify.as_ref().map(|v| serde_json::from_str(v).unwrap()),
            ss.fee_paid_for_created_justify as u64,
            ss.fee_paid_for_deleted_justify as u64,
        ))
    }
}

impl ShardStoreReadTransaction<PublicKey, TariDanPayload> for SqliteShardStoreReadTransaction<'_> {
    fn get_high_qc_for(
        &mut self,
        payload_id: PayloadId,
        shard_id: ShardId,
    ) -> Result<QuorumCertificate<PublicKey>, StorageError> {
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

        let qc = serde_json::from_str(&qc.qc_json).unwrap();
        Ok(qc)
    }

    fn get_high_qcs(&mut self, payload_id: PayloadId) -> Result<Vec<QuorumCertificate<PublicKey>>, StorageError> {
        use crate::schema::high_qcs;

        let qcs: Vec<HighQc> = high_qcs::table
            .filter(high_qcs::payload_id.eq(payload_id.as_bytes()))
            .order_by(high_qcs::height.desc())
            .get_results(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get high qc error: {}", e),
            })?;

        let qcs = qcs
            .into_iter()
            .map(|qc| serde_json::from_str(&qc.qc_json).unwrap())
            .collect();
        Ok(qcs)
    }

    fn get_leaf_node(&mut self, payload_id: &PayloadId, shard: &ShardId) -> Result<LeafNode, StorageError> {
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

    fn get_current_leaders_states(&mut self, payload_id: &PayloadId) -> Result<Vec<CurrentLeaderStates>, StorageError> {
        use crate::schema::current_leader_states;

        let states: Vec<CurrentLeaderState> = current_leader_states::table
            .filter(current_leader_states::payload_id.eq(payload_id.as_bytes()))
            .get_results(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get current state error: {}", e),
            })?;

        let states = states
            .into_iter()
            .map(|state| CurrentLeaderStates {
                payload_id: state.payload_id,
                shard_id: state.shard_id,
                leader_round: state.leader_round,
                leader: state.leader,
                timestamp: state.timestamp,
            })
            .collect::<Vec<_>>();
        Ok(states)
    }

    fn get_payload(&mut self, id: &PayloadId) -> Result<TariDanPayload, StorageError> {
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

        let instructions = serde_json::from_str(&payload.instructions).unwrap();
        let fee_instructions = serde_json::from_str(&payload.fee_instructions).unwrap();

        let public_nonce =
            PublicKey::from_vec(&payload.public_nonce).map_err(StorageError::InvalidByteArrayConversion)?;
        let signature =
            PrivateKey::from_bytes(payload.scalar.as_slice()).map_err(StorageError::InvalidByteArrayConversion)?;

        let signature = InstructionSignature::try_from(Signature::new(public_nonce, signature)).map_err(|e| {
            StorageError::InvalidTypeCasting {
                reason: format!("Get payload error: {}", e),
            }
        })?;

        let sender_public_key =
            PublicKey::from_vec(&payload.sender_address).map_err(StorageError::InvalidByteArrayConversion)?;
        let meta: TransactionMeta = serde_json::from_str(&payload.meta).unwrap();

        let transaction = Transaction::new(fee_instructions, instructions, signature, sender_public_key, meta);
        let mut tari_dan_payload = TariDanPayload::new(transaction);

        if payload.is_finalized {
            // deserialize the transaction result
            let result_field: Option<PayloadResult> = match payload.result {
                Some(result_json) => {
                    let result: PayloadResult =
                        serde_json::from_str(&result_json).map_err(|_| StorageError::DecodingError)?;
                    Some(result)
                },
                None => None,
            };
            if let Some(result) = result_field {
                tari_dan_payload.set_result(result.exec_result);
            }
        }

        Ok(tari_dan_payload)
    }

    fn get_node(&mut self, hash: &TreeNodeHash) -> Result<HotStuffTreeNode<PublicKey, TariDanPayload>, StorageError> {
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

        let parent = TreeNodeHash::try_from(node.parent_node_hash)?;
        let node_height = node.height as u64;

        let shard = ShardId::from_bytes(&node.shard)?;
        let payload = PayloadId::try_from(node.payload_id)?;

        let payload_height = node.payload_height as u64;
        let leader_round = node.leader_round as u32;
        let local_pledge: Option<ObjectPledge> = serde_json::from_str(&node.local_pledges).unwrap();

        let epoch = node.epoch as u64;
        let proposed_by = PublicKey::from_bytes(&node.proposed_by).map_err(StorageError::InvalidByteArrayConversion)?;

        let justify: QuorumCertificate<PublicKey> = serde_json::from_str(&node.justify).unwrap();

        let node = HotStuffTreeNode::new(
            parent,
            shard,
            NodeHeight(node_height),
            payload,
            None,
            NodeHeight(payload_height),
            leader_round,
            local_pledge,
            Epoch(epoch),
            proposed_by,
            justify,
        );

        if node.hash() != hash {
            error!(
                target: LOG_TARGET,
                "Hash mismatch node in DB: {}, rehashed: {}. If you're seeing this, it is likely that a HashMap is \
                 used somewhere in the ObjectPledge causing the hash to differ",
                hash,
                node.hash()
            );
        }
        Ok(node)
    }

    fn get_locked_node_hash_and_height(
        &mut self,
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

    fn get_last_executed_height(&mut self, shard: ShardId, payload_id: PayloadId) -> Result<NodeHeight, StorageError> {
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

    fn get_state_inventory(&mut self) -> Result<Vec<ShardId>, StorageError> {
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

    fn get_substate_states(&mut self, shard_ids: &[ShardId]) -> Result<Vec<SubstateShardData>, StorageError> {
        use crate::schema::substates;

        let shard_ids = shard_ids
            .iter()
            .map(|sh| Vec::from(sh.as_bytes()))
            .collect::<Vec<Vec<u8>>>();

        let substate_states: Vec<crate::models::substate::Substate> = substates::table
            .filter(substates::shard_id.eq_any(shard_ids))
            .get_results(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get substate change error: {}", e),
            })?;

        substate_states
            .iter()
            .map(Self::map_substate_to_shard_data)
            .collect::<Result<_, _>>()
    }

    fn get_substate_states_by_range(
        &mut self,
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
                    .and(substates::shard_id.le(end_shard_id.as_bytes()))
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

    fn get_last_voted_height(
        &mut self,
        shard: ShardId,
        payload_id: PayloadId,
    ) -> Result<(NodeHeight, u32), StorageError> {
        use crate::schema::last_voted_heights;

        let last_vote: Option<LastVotedHeight> = last_voted_heights::table
            .filter(last_voted_heights::shard_id.eq(shard.as_bytes().to_vec()))
            .filter(last_voted_heights::payload_id.eq(payload_id.as_bytes().to_vec()))
            .order_by(last_voted_heights::node_height.desc())
            .then_order_by(last_voted_heights::leader_round.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get last voted height error: {}", e),
            })?;

        if let Some(last_vote_height) = last_vote {
            let height = last_vote_height.node_height as u64;
            let leader_round = last_vote_height.leader_round as u32;
            Ok((NodeHeight(height), leader_round))
        } else {
            Ok((NodeHeight(0), 0))
        }
    }

    fn get_leader_proposals(
        &mut self,
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
                let hot_stuff_tree_node = serde_json::from_str(&proposal.hotstuff_tree_node).unwrap();
                Ok(hot_stuff_tree_node)
            })
            .collect()
    }

    // Get leader proposal with highest payload height for a particular shard.
    fn get_last_payload_height_for_leader_proposal(
        &mut self,
        payload: PayloadId,
        shard: ShardId,
    ) -> Result<NodeHeight, StorageError> {
        use crate::schema::leader_proposals;
        let shard_bytes = shard.as_bytes();
        let proposal: Option<LeaderProposal> = leader_proposals::table
            .filter(
                leader_proposals::payload_id
                    .eq(payload.as_bytes())
                    .and(leader_proposals::shard_id.eq(shard_bytes)),
            )
            .order_by(leader_proposals::payload_height.desc())
            .first(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get payload vote: {}", e),
            })?;
        if let Some(proposal) = proposal {
            Ok(NodeHeight(proposal.payload_height as u64))
        } else {
            Ok(NodeHeight(0))
        }
    }

    fn has_vote_for(&mut self, from: &PublicKey, node_hash: TreeNodeHash) -> Result<bool, StorageError> {
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

    fn get_received_votes_for(&mut self, node_hash: TreeNodeHash) -> Result<Vec<VoteMessage>, StorageError> {
        use crate::schema::received_votes;

        let filtered_votes: Option<Vec<ReceivedVote>> = received_votes::table
            .filter(received_votes::tree_node_hash.eq(node_hash.as_bytes()))
            .get_results(self.transaction.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get received vote for: {}", e),
            })?;

        if let Some(filtered_votes) = filtered_votes {
            let v = filtered_votes
                .iter()
                .map(|v| serde_json::from_str::<VoteMessage>(&v.vote_message).unwrap())
                .collect();
            Ok(v)
        } else {
            Ok(vec![])
        }
    }

    fn get_recent_transactions(&mut self) -> Result<Vec<RecentTransaction>, StorageError> {
        let res = sql_query("select payload_id,timestamp,meta,instructions from payloads order by timestamp desc")
            .load::<QueryableRecentTransaction>(self.transaction.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Get recent transactions: {}", e),
            })?;
        Ok(res
            .into_iter()
            .map(|recent_transaction| recent_transaction.into())
            .collect())
    }

    fn get_payload_result(&mut self, payload_id: &PayloadId) -> Result<PayloadResult, StorageError> {
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

    fn get_transaction(&mut self, payload_id: Vec<u8>) -> Result<Vec<SQLTransaction>, StorageError> {
        let res = sql_query(
            "select node_hash, parent_node_hash, shard, height, payload_height, (select count(*) from received_votes \
             v where v.tree_node_hash = node_hash) as total_votes, (select count(*) from leader_proposals lp where \
             lp.payload_id  = n.payload_id and lp.payload_height = n.payload_height and lp.node_hash = n.node_hash) \
             as total_leader_proposals, n.timestamp, n.justify, n.proposed_by, n.leader_round from nodes as n where \
             payload_id = ? order by shard",
        )
        .bind::<Binary, _>(payload_id)
        .load::<QueryableTransaction>(self.transaction.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("Get transaction: {}", e),
        })?;
        let res = res.into_iter().map(|transaction| transaction.into()).collect();
        Ok(res)
    }

    fn get_substates_for_payload(
        &mut self,
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

    fn get_fees_by_epoch(
        &mut self,
        epoch: u64,
        claim_leader_public_key: Vec<u8>,
    ) -> Result<Vec<ClaimLeaderFees>, StorageError> {
        let res = sql_query(
            "select COALESCE(NULLIF(created_justify_leader, ''), destroyed_justify_leader) as \
             justify_leader_public_key, created_at_epoch, destroyed_at_epoch, fee_paid_for_created_justify, \
             fee_paid_for_deleted_justify as fee_paid_for_destroyed_justify from substates where \
             (created_justify_leader == ? and created_at_epoch == ?) or
                (destroyed_justify_leader == ? and destroyed_at_epoch == ?);",
        )
        .bind::<Text, _>(claim_leader_public_key.to_hex())
        .bind::<BigInt, _>(epoch as i64)
        .bind::<Text, _>(claim_leader_public_key.to_hex())
        .bind::<BigInt, _>(epoch as i64)
        .load::<QueryableClaimFeesSubstate>(self.transaction.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("Get claimable substates fees: {}", e),
        })?;
        Ok(res.into_iter().map(|transaction| transaction.into()).collect())
    }

    // -------------------------------- Pledges -------------------------------- //
    fn get_resolved_pledges_for_payload(&mut self, payload: PayloadId) -> Result<Vec<ObjectPledgeInfo>, StorageError> {
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
    /// None indicates if the transaction has been explicitly committed/rolled back
    transaction: Option<SqliteShardStoreReadTransaction<'a>>,
}

impl<'a> SqliteShardStoreWriteTransaction<'a> {
    pub fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self {
            transaction: Some(SqliteShardStoreReadTransaction::new(transaction)),
        }
    }

    pub fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.as_mut().unwrap().connection()
    }

    fn create_pledge(&mut self, shard: ShardId, obj: DbShardPledge) -> Result<ObjectPledge, StorageError> {
        use crate::schema::substates;
        let current_state: Option<Substate> = substates::table
            .filter(substates::shard_id.eq(shard.as_bytes()))
            .first(self.connection())
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
                        fees_accrued: current_state.fee_paid_for_deleted_justify as u64,
                    }
                } else {
                    SubstateState::Up {
                        address: current_state.address.parse().map_err(|_| StorageError::DecodingError)?,
                        created_by: PayloadId::try_from(current_state.created_by_payload_id)?,
                        data: serde_json::from_str::<tari_engine_types::substate::Substate>(&current_state.data)
                            .unwrap(),
                        fees_accrued: current_state.fee_paid_for_created_justify as u64,
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
        self.transaction.take().unwrap().transaction.commit()?;
        Ok(())
    }

    fn rollback(mut self) -> Result<(), StorageError> {
        self.transaction.take().unwrap().transaction.rollback()?;
        Ok(())
    }

    fn insert_high_qc(
        &mut self,
        identity: PublicKey,
        shard: ShardId,
        qc: QuorumCertificate<PublicKey>,
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
            .execute(self.connection())
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
            //     //     .execute(self.connection())
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
        let instructions = serde_json::to_string_pretty(transaction.instructions()).unwrap();
        let fee_instructions = serde_json::to_string_pretty(transaction.fee_instructions()).unwrap();

        let signature = transaction.signature();

        let public_nonce = Vec::from(signature.signature().get_public_nonce().as_bytes());
        let scalar = Vec::from(signature.signature().get_signature().as_bytes());

        let sender_public_key = Vec::from(transaction.sender_public_key().as_bytes());

        let meta = json!(transaction.meta()).to_string();

        let payload_id = Vec::from(payload.to_id().as_bytes());

        let new_row = NewPayload {
            payload_id,
            fee_instructions,
            instructions,
            public_nonce,
            scalar,
            sender_address: sender_public_key,
            meta,
            result: None,
        };

        match diesel::insert_into(payloads::table)
            .values(&new_row)
            .execute(self.connection())
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

    fn save_current_leader_state(
        &mut self,
        payload: PayloadId,
        shard_id: ShardId,
        leader_round: u32,
        leader: PublicKey,
    ) -> Result<(), StorageError> {
        use crate::schema::current_leader_states;

        let payload_id = payload.as_bytes().to_vec();

        let new_row = NewCurrentLeaderState {
            payload_id: payload_id.clone(),
            shard_id: shard_id.as_bytes().to_vec(),
            leader_round: i64::from(leader_round),
            leader: leader.as_bytes().to_vec(),
        };

        match diesel::insert_into(current_leader_states::table)
            .values(&new_row)
            .execute(self.connection())
        {
            Ok(_) => {},
            Err(err) => match err {
                Error::DatabaseError(DatabaseErrorKind::UniqueViolation, _) => {
                    match diesel::update(current_leader_states::table)
                        .set(current_leader_states::leader_round.eq(i64::from(leader_round)))
                        .filter(current_leader_states::payload_id.eq(payload_id))
                        .execute(self.connection())
                    {
                        Ok(_) => {},
                        Err(err) => {
                            return Err(StorageError::QueryError {
                                reason: format!("Set current state error: {}", err),
                            })
                        },
                    }
                },
                _ => {
                    return Err(StorageError::QueryError {
                        reason: format!("Set current state error: {}", err),
                    })
                },
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
            .first(self.connection())
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
                    .execute(self.connection())
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
                    .execute(self.connection())
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("Update leaf node error: {}", e),
                    })?;
            },
        }

        Ok(())
    }

    fn save_node(&mut self, node: HotStuffTreeNode<PublicKey, TariDanPayload>) -> Result<(), StorageError> {
        use crate::schema::nodes;

        let node_hash = Vec::from(node.calculate_hash().as_bytes());
        let parent_node_hash = Vec::from(node.parent().as_bytes());

        let height = node.height().as_u64() as i64;
        let shard = Vec::from(node.shard().as_bytes());

        let payload_id = Vec::from(node.payload_id().as_bytes());
        let payload_height = node.payload_height().as_u64() as i64;
        let leader_round = i64::from(node.leader_round());

        let local_pledges = serde_json::to_string_pretty(&node.local_pledge()).unwrap();

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
            leader_round,
            local_pledges,
            epoch,
            proposed_by,
            justify,
        };

        match diesel::insert_into(nodes::dsl::nodes)
            .values(&new_row)
            .execute(self.connection())
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
            .execute(self.connection())
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
            .execute(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Set last executed height error: {}", e),
            })?;
        Ok(())
    }

    fn commit_substate_changes(
        &mut self,
        node: HotStuffTreeNode<PublicKey, TariDanPayload>,
        changes: &[SubstateState],
    ) -> Result<(), StorageError> {
        use crate::schema::substates;

        let current_state = substates::table
            .filter(substates::shard_id.eq(node.shard().as_bytes()))
            .first::<Substate>(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Save substate changes error. Could not retrieve current state: {}", e),
            })?;

        for st_change in changes {
            match st_change {
                SubstateState::DoesNotExist => (),
                SubstateState::Up {
                    address,
                    data: d,
                    fees_accrued,
                    ..
                } => {
                    if let Some(s) = &current_state {
                        if !s.is_destroyed() {
                            return Err(StorageError::QueryError {
                                reason: "Save substate changes error. Cannot create a substate that has not been \
                                         destroyed"
                                    .to_string(),
                            });
                        }
                    }

                    let pretty_data = serde_json::to_string_pretty(d).unwrap();
                    let new_row = NewSubstate {
                        shard_id: node.shard().as_bytes().to_vec(),
                        address: address.to_string(),
                        version: d.version().into(),
                        data: pretty_data,
                        created_by_payload_id: node.payload_id().as_bytes().to_vec(),
                        created_justify: Some(serde_json::to_string_pretty(node.justify()).unwrap()),
                        created_node_hash: node.hash().as_bytes().to_vec(),
                        created_height: node.height().as_u64() as i64,
                        fee_paid_for_created_justify: *fees_accrued as i64, // 0
                        fee_paid_for_deleted_justify: 0,
                        created_justify_leader: Some(node.justify().proposed_by().to_hex()),
                        destroyed_justify_leader: None,
                        created_at_epoch: Some(node.epoch().as_u64() as i64),
                        destroyed_at_epoch: None,
                    };
                    diesel::insert_into(substates::table)
                        .values(&new_row)
                        .execute(self.connection())
                        .map_err(|e| StorageError::QueryError {
                            reason: format!("Save substate changes error. Could not insert new substate: {}", e),
                        })?;
                },
                SubstateState::Down { fees_accrued, .. } => {
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
                                substates::destroyed_at_epoch.eq(node.epoch().as_u64() as i64),
                                substates::fee_paid_for_deleted_justify.eq(*fees_accrued as i64),
                                substates::destroyed_justify_leader.eq(node.justify().proposed_by().to_hex()),
                            ))
                            .execute(self.connection())
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
            address: substate_data.substate_address().to_string(),
            version: i64::from(substate_data.version()),
            data: serde_json::to_string_pretty(substate_data.substate()).unwrap(),
            created_by_payload_id: substate_data.created_payload_id().as_bytes().to_vec(),
            created_justify: substate_data
                .created_justify()
                .as_ref()
                .map(|justify| serde_json::to_string_pretty(&justify).unwrap()),
            created_height: substate_data.created_height().as_u64() as i64,
            created_node_hash: substate_data.created_node_hash().as_bytes().to_vec(),
            destroyed_by_payload_id: substate_data.destroyed_payload_id().map(|v| v.as_bytes().to_vec()),
            destroyed_justify: substate_data
                .destroyed_justify()
                .as_ref()
                .map(|v| serde_json::to_string_pretty(v).unwrap()),
            destroyed_height: substate_data.destroyed_height().map(|v| v.as_u64() as i64),
            destroyed_node_hash: substate_data.destroyed_node_hash().map(|v| v.as_bytes().to_vec()),
            fee_paid_for_created_justify: substate_data.created_fee_accrued() as i64,
            fee_paid_for_deleted_justify: substate_data.created_fee_accrued() as i64,
            created_justify_leader: substate_data
                .created_justify()
                .as_ref()
                .map(|justify| justify.proposed_by().to_hex()),
            destroyed_justify_leader: substate_data
                .destroyed_justify()
                .as_ref()
                .map(|justify| justify.proposed_by().to_hex()),
            created_at_epoch: substate_data.created_justify().map(|j| j.epoch().as_u64() as i64),
            destroyed_at_epoch: substate_data
                .destroyed_justify()
                .as_ref()
                .map(|j| j.epoch().as_u64() as i64),
        };

        diesel::insert_into(substates::table)
            .values(&new_row)
            .execute(self.connection())
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
        leader_round: u32,
    ) -> Result<(), StorageError> {
        use crate::schema::last_voted_heights;

        let new_row = NewLastVotedHeight {
            shard_id: shard.as_bytes().to_vec(),
            payload_id: payload_id.as_bytes().to_vec(),
            node_height: height.as_u64() as i64,
            leader_round: i64::from(leader_round),
        };

        diesel::insert_into(last_voted_heights::table)
            .values(&new_row)
            .execute(self.connection())
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
        leader_round: u32,
        node: HotStuffTreeNode<PublicKey, TariDanPayload>,
    ) -> Result<(), StorageError> {
        use crate::schema::leader_proposals;

        let shard = Vec::from(shard.as_bytes());
        let payload = Vec::from(payload.as_bytes());
        let payload_height = payload_height.as_u64() as i64;
        let node_hash = node.hash().as_bytes().to_vec();
        let node = serde_json::to_string_pretty(&node).unwrap();
        let leader_round = i64::from(leader_round);

        let new_row = NewLeaderProposal {
            shard_id: shard,
            payload_id: payload,
            payload_height,
            leader_round,
            node_hash,
            hotstuff_tree_node: node,
        };

        match diesel::insert_into(leader_proposals::table)
            .values(&new_row)
            .execute(self.connection())
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
            .execute(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Save received voted for: {}", e),
            })?;

        Ok(())
    }

    fn update_payload_result(
        &mut self,
        requested_payload_id: &PayloadId,
        result: PayloadResult,
    ) -> Result<(), StorageError> {
        use crate::schema::payloads;

        let result_json = serde_json::to_string_pretty(&result).map_err(|_| StorageError::EncodingError)?;

        diesel::update(payloads::table)
            .filter(payloads::payload_id.eq(requested_payload_id.as_bytes()))
            .set(payloads::result.eq(result_json))
            .execute(self.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "update_payload_result".to_string(),
            })?;

        Ok(())
    }

    fn mark_payload_finalized(&mut self, payload_id: &PayloadId) -> Result<(), StorageError> {
        use crate::schema::payloads;

        let num_rows = diesel::update(payloads::table)
            .filter(payloads::payload_id.eq(payload_id.as_bytes()))
            .set(payloads::is_finalized.eq(true))
            .execute(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Update payload: {}", e),
            })?;

        if num_rows == 0 {
            return Err(StorageError::NotFound {
                item: "payload".to_string(),
                key: payload_id.to_string(),
            });
        }

        Ok(())
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
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("Pledge object error: {}", e),
            })?;

        if let Some(pledge) = existing_pledge {
            // Emit a warning in this case, consensus should handle this correctly (by rejecting)
            if pledge.pledged_to_payload_id != payload.as_bytes() {
                warn!(
                    target: LOG_TARGET,
                    "[pledge_object]: Attempted to create a pledge for shard {}/payload{} that already exists for \
                     another payload {}",
                    shard,
                    payload,
                    to_hex(&pledge.pledged_to_payload_id)
                );
            }

            return self.create_pledge(shard, pledge);
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
            .execute(self.connection())
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
            .first(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Pledge object error: {}", e),
            })?;

        self.create_pledge(shard, new_pledge)
    }

    fn complete_pledges(
        &mut self,
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
            .execute(self.connection())
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
        &mut self,
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
            .execute(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Abandon pledges: {}", e),
            })?;

        if rows_affected == 0 {
            return Err(StorageError::NotFound {
                item: "Abandon pledges".to_string(),
                key: format!("payload={}, shard={}", payload_id, shard),
            });
        }
        Ok(())
    }

    fn save_burnt_utxo(
        &mut self,
        substate: &tari_engine_types::substate::Substate,
        commitment_address: SubstateAddress,
        shard_id: ShardId,
    ) -> Result<(), StorageError> {
        let new_row = NewSubstate {
            shard_id: shard_id.as_bytes().to_vec(),
            address: commitment_address.to_string(),
            version: i64::from(substate.version()),
            data: serde_json::to_string_pretty(substate).unwrap(),
            created_by_payload_id: vec![0; 32],
            created_justify: None,
            created_node_hash: TreeNodeHash::zero().as_bytes().to_vec(),
            created_height: 0,
            fee_paid_for_created_justify: 0,
            fee_paid_for_deleted_justify: 0,
            created_justify_leader: None,
            destroyed_justify_leader: None,
            created_at_epoch: None,
            destroyed_at_epoch: None,
        };
        use crate::schema::substates;
        diesel::insert_into(substates::table)
            .values(new_row)
            .execute(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("Burnt commitment insert error: {}", e),
            })?;
        Ok(())
    }
}

impl<'a> Deref for SqliteShardStoreWriteTransaction<'a> {
    type Target = SqliteShardStoreReadTransaction<'a>;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref().unwrap()
    }
}

impl<'a> DerefMut for SqliteShardStoreWriteTransaction<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transaction.as_mut().unwrap()
    }
}

impl Drop for SqliteShardStoreWriteTransaction<'_> {
    fn drop(&mut self) {
        if self.transaction.is_some() {
            warn!(
                target: LOG_TARGET,
                "Shard store write transaction was not committed/rolled back"
            );
        }
    }
}
