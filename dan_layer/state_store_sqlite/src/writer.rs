//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{iter::Peekable, ops::Deref};

use diesel::{
    dsl,
    dsl::count_star,
    sql_types::Text,
    AsChangeset,
    ExpressionMethods,
    NullableExpressionMethods,
    OptionalExtension,
    QueryDsl,
    RunQueryDsl,
    SqliteConnection,
};
use indexmap::IndexMap;
use log::*;
use tari_dan_common_types::{
    optional::Optional,
    shard::Shard,
    Epoch,
    NodeAddressable,
    NodeHeight,
    ShardGroup,
    SubstateLockType,
    ToSubstateAddress,
    VersionedSubstateId,
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        BlockTransactionExecution,
        BurntUtxo,
        Decision,
        EpochCheckpoint,
        ForeignParkedProposal,
        ForeignProposal,
        ForeignProposalStatus,
        ForeignReceiveCounters,
        ForeignSendCounters,
        HighQc,
        LastExecuted,
        LastProposed,
        LastSentVote,
        LastVoted,
        LeafBlock,
        LockConflict,
        LockedBlock,
        NoVoteReason,
        PendingShardStateTreeDiff,
        QcId,
        QuorumCertificate,
        SubstateChange,
        SubstateLock,
        SubstatePledge,
        SubstatePledges,
        SubstateRecord,
        TransactionPoolConfirmedStage,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        TransactionRecord,
        VersionedStateHashTreeDiff,
        Vote,
    },
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_engine_types::substate::SubstateId;
use tari_state_tree::{Node, NodeKey, StaleTreeNode, TreeNode, Version};
use tari_transaction::TransactionId;
use tari_utilities::ByteArray;
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::{
    error::SqliteStorageError,
    reader::SqliteStateStoreReadTransaction,
    serialization::{serialize_hex, serialize_json},
    sql_models,
    sqlite_transaction::SqliteTransaction,
};

const LOG_TARGET: &str = "tari::dan::storage";

pub struct SqliteStateStoreWriteTransaction<'a, TAddr> {
    /// None indicates if the transaction has been explicitly committed/rolled back
    transaction: Option<SqliteStateStoreReadTransaction<'a, TAddr>>,
}

impl<'a, TAddr: NodeAddressable> SqliteStateStoreWriteTransaction<'a, TAddr> {
    pub fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self {
            transaction: Some(SqliteStateStoreReadTransaction::new(transaction)),
        }
    }

    pub fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.as_mut().unwrap().connection()
    }

    fn parked_blocks_remove(&mut self, block_id: &str) -> Result<(Block, Vec<ForeignProposal>), StorageError> {
        use crate::schema::parked_blocks;

        let parked_block = parked_blocks::table
            .filter(parked_blocks::block_id.eq(&block_id))
            .first::<sql_models::ParkedBlock>(self.connection())
            .optional()
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "parked_blocks_remove",
                source: e,
            })?
            .ok_or_else(|| StorageError::NotFound {
                item: "parked_blocks".to_string(),
                key: block_id.to_string(),
            })?;

        diesel::delete(parked_blocks::table)
            .filter(parked_blocks::block_id.eq(&block_id))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "parked_blocks_remove",
                source: e,
            })?;

        parked_block.try_into()
    }

    fn parked_blocks_insert(
        &mut self,
        block: &Block,
        foreign_proposals: &[ForeignProposal],
    ) -> Result<(), StorageError> {
        use crate::schema::{blocks, parked_blocks};

        // check if block exists in blocks table using count query
        let block_id = serialize_hex(block.id());

        let block_exists = blocks::table
            .count()
            .filter(blocks::block_id.eq(&block_id))
            .first::<i64>(self.connection())
            .map(|count| count > 0)
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "parked_blocks_insert",
                source: e,
            })?;

        if block_exists {
            return Err(StorageError::QueryError {
                reason: format!("Cannot park block {block_id} that already exists in blocks table"),
            });
        }

        // check if block already exists in parked_blocks
        let already_parked = parked_blocks::table
            .count()
            .filter(parked_blocks::block_id.eq(&block_id))
            .first::<i64>(self.connection())
            .map(|count| count > 0)
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "parked_blocks_insert",
                source: e,
            })?;

        if already_parked {
            return Ok(());
        }

        let insert = (
            parked_blocks::block_id.eq(&block_id),
            parked_blocks::parent_block_id.eq(serialize_hex(block.parent())),
            parked_blocks::network.eq(block.network().to_string()),
            parked_blocks::merkle_root.eq(block.merkle_root().to_string()),
            parked_blocks::height.eq(block.height().as_u64() as i64),
            parked_blocks::epoch.eq(block.epoch().as_u64() as i64),
            parked_blocks::shard_group.eq(block.shard_group().encode_as_u32() as i32),
            parked_blocks::proposed_by.eq(serialize_hex(block.proposed_by().as_bytes())),
            parked_blocks::command_count.eq(block.commands().len() as i64),
            parked_blocks::commands.eq(serialize_json(block.commands())?),
            parked_blocks::total_leader_fee.eq(block.total_leader_fee() as i64),
            parked_blocks::justify.eq(serialize_json(block.justify())?),
            parked_blocks::foreign_indexes.eq(serialize_json(block.foreign_indexes())?),
            parked_blocks::signature.eq(block.signature().map(serialize_json).transpose()?),
            parked_blocks::timestamp.eq(block.timestamp() as i64),
            parked_blocks::base_layer_block_height.eq(block.base_layer_block_height() as i64),
            parked_blocks::base_layer_block_hash.eq(serialize_hex(block.base_layer_block_hash())),
            parked_blocks::foreign_proposals.eq(serialize_json(foreign_proposals)?),
            parked_blocks::extra_data.eq(block.extra_data().map(serialize_json).transpose()?),
        );

        diesel::insert_into(parked_blocks::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "parked_blocks_upsert",
                source: e,
            })?;

        Ok(())
    }
}

impl<'tx, TAddr: NodeAddressable + 'tx> StateStoreWriteTransaction for SqliteStateStoreWriteTransaction<'tx, TAddr> {
    type Addr = TAddr;

    fn commit(&mut self) -> Result<(), StorageError> {
        // Take so that we mark this transaction as complete in the drop impl
        self.transaction.take().unwrap().commit()?;
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), StorageError> {
        // Take so that we mark this transaction as complete in the drop impl
        self.transaction.take().unwrap().rollback()?;
        Ok(())
    }

    fn blocks_insert(&mut self, block: &Block) -> Result<(), StorageError> {
        use crate::schema::blocks;

        let insert = (
            blocks::block_id.eq(serialize_hex(block.id())),
            blocks::parent_block_id.eq(serialize_hex(block.parent())),
            blocks::merkle_root.eq(block.merkle_root().to_string()),
            blocks::network.eq(block.network().to_string()),
            blocks::height.eq(block.height().as_u64() as i64),
            blocks::epoch.eq(block.epoch().as_u64() as i64),
            blocks::shard_group.eq(block.shard_group().encode_as_u32() as i32),
            blocks::proposed_by.eq(serialize_hex(block.proposed_by().as_bytes())),
            blocks::command_count.eq(block.commands().len() as i64),
            blocks::commands.eq(serialize_json(block.commands())?),
            blocks::total_leader_fee.eq(block.total_leader_fee() as i64),
            blocks::qc_id.eq(serialize_hex(block.justify().id())),
            blocks::is_dummy.eq(block.is_dummy()),
            blocks::is_justified.eq(block.is_justified()),
            blocks::signature.eq(block.signature().map(serialize_json).transpose()?),
            blocks::foreign_indexes.eq(serialize_json(block.foreign_indexes())?),
            blocks::timestamp.eq(block.timestamp() as i64),
            blocks::base_layer_block_height.eq(block.base_layer_block_height() as i64),
            blocks::base_layer_block_hash.eq(serialize_hex(block.base_layer_block_hash())),
            blocks::extra_data.eq(block.extra_data().map(serialize_json).transpose()?),
        );

        diesel::insert_into(blocks::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_insert",
                source: e,
            })?;

        diesel::sql_query(
            r#"
            UPDATE blocks
            SET block_time = timestamp -
                             (SELECT timestamp
                              FROM blocks
                              WHERE block_id == ?)
            WHERE block_id = ?"#,
        )
        .bind::<Text, _>(serialize_hex(block.justify().block_id()))
        .bind::<Text, _>(serialize_hex(block.id()))
        .execute(self.connection())
        .map_err(|e| SqliteStorageError::DieselError {
            operation: "blocks_insert_set_delta_time",
            source: e,
        })?;

        Ok(())
    }

    fn blocks_delete(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        use crate::schema::{blocks, diagnostic_deleted_blocks};

        let block_id = serialize_hex(block_id);

        diesel::insert_into(diagnostic_deleted_blocks::table)
            .values(blocks::table.filter(blocks::block_id.eq(&block_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_delete (insert into diagnostic_deleted_blocks)",
                source: e,
            })?;

        let num_deleted = diesel::delete(blocks::table)
            .filter(blocks::block_id.eq(&block_id))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_delete",
                source: e,
            })?;

        if num_deleted == 0 {
            return Err(StorageError::NotFound {
                item: "blocks".to_string(),
                key: block_id,
            });
        }

        Ok(())
    }

    fn blocks_set_flags(
        &mut self,
        block_id: &BlockId,
        is_committed: Option<bool>,
        is_justified: Option<bool>,
    ) -> Result<(), StorageError> {
        use crate::schema::blocks;

        #[derive(AsChangeset)]
        #[diesel(table_name = blocks)]
        struct Changes {
            is_committed: Option<bool>,
            is_justified: Option<bool>,
        }
        let changes = Changes {
            is_committed,
            is_justified,
        };

        diesel::update(blocks::table)
            .filter(blocks::block_id.eq(serialize_hex(block_id)))
            .set(changes)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_commit",
                source: e,
            })?;

        Ok(())
    }

    fn block_diffs_insert(&mut self, block_id: &BlockId, changes: &[SubstateChange]) -> Result<(), StorageError> {
        use crate::schema::block_diffs;

        let block_id = serialize_hex(block_id);
        // We commit in chunks because we can hit the SQL variable limit
        for chunk in changes.chunks(1000) {
            let values = chunk
                .iter()
                .map(|ch| {
                    Ok((
                        block_diffs::block_id.eq(&block_id),
                        block_diffs::transaction_id.eq(serialize_hex(ch.transaction_id())),
                        block_diffs::substate_id.eq(ch.versioned_substate_id().substate_id().to_string()),
                        block_diffs::version.eq(ch.versioned_substate_id().version() as i32),
                        block_diffs::shard.eq(ch.shard().as_u32() as i32),
                        block_diffs::change.eq(ch.as_change_string()),
                        block_diffs::state.eq(ch.substate().map(serialize_json).transpose()?),
                    ))
                })
                .collect::<Result<Vec<_>, StorageError>>()?;

            diesel::insert_into(block_diffs::table)
                .values(values)
                .execute(self.connection())
                .map(|_| ())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "block_diffs_insert",
                    source: e,
                })?;
        }

        Ok(())
    }

    fn block_diffs_remove(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        use crate::schema::block_diffs;

        diesel::delete(block_diffs::table)
            .filter(block_diffs::block_id.eq(serialize_hex(block_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "block_diffs_remove",
                source: e,
            })?;

        Ok(())
    }

    fn quorum_certificates_insert(&mut self, qc: &QuorumCertificate) -> Result<(), StorageError> {
        use crate::schema::quorum_certificates;

        let insert = (
            quorum_certificates::qc_id.eq(serialize_hex(qc.id())),
            quorum_certificates::shard_group.eq(qc.shard_group().encode_as_u32() as i32),
            quorum_certificates::block_id.eq(serialize_hex(qc.block_id())),
            quorum_certificates::json.eq(serialize_json(qc)?),
        );

        diesel::insert_into(quorum_certificates::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "quorum_certificates_insert",
                source: e,
            })?;

        Ok(())
    }

    fn last_sent_vote_set(&mut self, last_sent_vote: &LastSentVote) -> Result<(), StorageError> {
        use crate::schema::last_sent_vote;

        let insert = (
            last_sent_vote::epoch.eq(last_sent_vote.epoch.as_u64() as i64),
            last_sent_vote::block_id.eq(serialize_hex(last_sent_vote.block_id)),
            last_sent_vote::block_height.eq(last_sent_vote.block_height.as_u64() as i64),
            last_sent_vote::decision.eq(i32::from(last_sent_vote.decision.as_u8())),
            last_sent_vote::signature.eq(serialize_json(&last_sent_vote.signature)?),
        );

        diesel::insert_into(last_sent_vote::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_sent_vote_set",
                source: e,
            })?;

        Ok(())
    }

    fn last_voted_set(&mut self, last_voted: &LastVoted) -> Result<(), StorageError> {
        use crate::schema::last_voted;

        let insert = (
            last_voted::block_id.eq(serialize_hex(last_voted.block_id)),
            last_voted::height.eq(last_voted.height.as_u64() as i64),
            last_voted::epoch.eq(last_voted.epoch.as_u64() as i64),
        );

        diesel::insert_into(last_voted::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_voted_set",
                source: e,
            })?;

        Ok(())
    }

    fn last_votes_unset(&mut self, last_voted: &LastVoted) -> Result<(), StorageError> {
        use crate::schema::last_voted;

        diesel::delete(last_voted::table)
            .filter(last_voted::block_id.eq(serialize_hex(last_voted.block_id)))
            .filter(last_voted::height.eq(last_voted.height.as_u64() as i64))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_votes_unset",
                source: e,
            })?;

        Ok(())
    }

    fn last_executed_set(&mut self, last_exec: &LastExecuted) -> Result<(), StorageError> {
        use crate::schema::last_executed;

        let insert = (
            last_executed::block_id.eq(serialize_hex(last_exec.block_id)),
            last_executed::height.eq(last_exec.height.as_u64() as i64),
            last_executed::epoch.eq(last_exec.epoch.as_u64() as i64),
        );

        diesel::insert_into(last_executed::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_executed_set",
                source: e,
            })?;

        Ok(())
    }

    fn last_proposed_set(&mut self, last_proposed: &LastProposed) -> Result<(), StorageError> {
        use crate::schema::last_proposed;

        let insert = (
            last_proposed::block_id.eq(serialize_hex(last_proposed.block_id)),
            last_proposed::height.eq(last_proposed.height.as_u64() as i64),
            last_proposed::epoch.eq(last_proposed.epoch.as_u64() as i64),
        );

        diesel::insert_into(last_proposed::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_proposed_set",
                source: e,
            })?;

        Ok(())
    }

    fn last_proposed_unset(&mut self, last_proposed: &LastProposed) -> Result<(), StorageError> {
        use crate::schema::last_proposed;

        diesel::delete(last_proposed::table)
            .filter(last_proposed::block_id.eq(serialize_hex(last_proposed.block_id)))
            .filter(last_proposed::height.eq(last_proposed.height.as_u64() as i64))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_proposed_unset",
                source: e,
            })?;

        Ok(())
    }

    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError> {
        use crate::schema::leaf_blocks;

        let insert = (
            leaf_blocks::block_id.eq(serialize_hex(leaf_node.block_id)),
            leaf_blocks::block_height.eq(leaf_node.height.as_u64() as i64),
            leaf_blocks::epoch.eq(leaf_node.epoch.as_u64() as i64),
        );

        diesel::insert_into(leaf_blocks::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "leaf_block_set",
                source: e,
            })?;

        Ok(())
    }

    fn locked_block_set(&mut self, locked_block: &LockedBlock) -> Result<(), StorageError> {
        use crate::schema::locked_block;

        let insert = (
            locked_block::block_id.eq(serialize_hex(locked_block.block_id)),
            locked_block::height.eq(locked_block.height.as_u64() as i64),
            locked_block::epoch.eq(locked_block.epoch.as_u64() as i64),
        );

        diesel::insert_into(locked_block::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "locked_block_set",
                source: e,
            })?;

        Ok(())
    }

    fn high_qc_set(&mut self, high_qc: &HighQc) -> Result<(), StorageError> {
        use crate::schema::high_qcs;

        let insert = (
            high_qcs::block_id.eq(serialize_hex(high_qc.block_id)),
            high_qcs::block_height.eq(high_qc.block_height().as_u64() as i64),
            high_qcs::epoch.eq(high_qc.epoch().as_u64() as i64),
            high_qcs::qc_id.eq(serialize_hex(high_qc.qc_id)),
        );

        diesel::insert_into(high_qcs::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "high_qc_set",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_proposals_upsert(
        &mut self,
        foreign_proposal: &ForeignProposal,
        proposed_in_block: Option<BlockId>,
    ) -> Result<(), StorageError> {
        use crate::schema::foreign_proposals;
        let block = foreign_proposal.block();

        let values = (
            foreign_proposals::block_id.eq(serialize_hex(block.id())),
            foreign_proposals::parent_block_id.eq(serialize_hex(block.parent())),
            foreign_proposals::merkle_root.eq(block.merkle_root().to_string()),
            foreign_proposals::network.eq(block.network().to_string()),
            foreign_proposals::height.eq(block.height().as_u64() as i64),
            foreign_proposals::epoch.eq(block.epoch().as_u64() as i64),
            foreign_proposals::shard_group.eq(block.shard_group().encode_as_u32() as i32),
            foreign_proposals::proposed_by.eq(serialize_hex(block.proposed_by().as_bytes())),
            foreign_proposals::command_count.eq(block.commands().len() as i64),
            foreign_proposals::commands.eq(serialize_json(block.commands())?),
            foreign_proposals::total_leader_fee.eq(block.total_leader_fee() as i64),
            foreign_proposals::qc.eq(serialize_json(block.justify())?),
            foreign_proposals::foreign_indexes.eq(serialize_json(block.foreign_indexes())?),
            foreign_proposals::timestamp.eq(block.timestamp() as i64),
            foreign_proposals::base_layer_block_height.eq(block.base_layer_block_height() as i64),
            foreign_proposals::base_layer_block_hash.eq(serialize_hex(block.base_layer_block_hash())),
            // Extra
            foreign_proposals::justify_qc_id.eq(serialize_hex(foreign_proposal.justify_qc().id())),
            foreign_proposals::block_pledge.eq(serialize_json(foreign_proposal.block_pledge())?),
            foreign_proposals::status.eq(ForeignProposalStatus::New.to_string()),
            foreign_proposals::extra_data.eq(foreign_proposal.block().extra_data().map(serialize_json).transpose()?),
        );

        diesel::insert_into(foreign_proposals::table)
            .values(&values)
            .on_conflict_do_nothing()
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposals_upsert",
                source: e,
            })?;

        if let Some(proposed_in_block) = proposed_in_block {
            self.foreign_proposals_set_proposed_in(block.id(), &proposed_in_block)?;
        }

        Ok(())
    }

    fn foreign_proposals_delete(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        use crate::schema::foreign_proposals;

        diesel::delete(foreign_proposals::table)
            .filter(foreign_proposals::block_id.eq(serialize_hex(block_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposals_delete",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_proposals_set_status(
        &mut self,
        block_id: &BlockId,
        status: ForeignProposalStatus,
    ) -> Result<(), StorageError> {
        use crate::schema::foreign_proposals;

        diesel::update(foreign_proposals::table)
            .filter(foreign_proposals::block_id.eq(serialize_hex(block_id)))
            .set(foreign_proposals::status.eq(status.to_string()))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposals_set_status",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_proposals_set_proposed_in(
        &mut self,
        block_id: &BlockId,
        proposed_in_block: &BlockId,
    ) -> Result<(), StorageError> {
        use crate::schema::{blocks, foreign_proposals};

        diesel::update(foreign_proposals::table)
            .filter(foreign_proposals::block_id.eq(serialize_hex(block_id)))
            .set((
                foreign_proposals::proposed_in_block.eq(serialize_hex(proposed_in_block)),
                foreign_proposals::proposed_in_block_height.eq(blocks::table
                    .select(blocks::height)
                    .filter(blocks::block_id.eq(serialize_hex(proposed_in_block)))
                    .single_value()),
                foreign_proposals::status.eq(ForeignProposalStatus::Proposed.to_string()),
            ))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposals_set_proposed_in",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_proposals_clear_proposed_in(&mut self, proposed_in_block: &BlockId) -> Result<(), StorageError> {
        use crate::schema::foreign_proposals;

        diesel::update(foreign_proposals::table)
            .filter(foreign_proposals::proposed_in_block.eq(serialize_hex(proposed_in_block)))
            .set((
                foreign_proposals::proposed_in_block.eq(None::<String>),
                foreign_proposals::proposed_in_block_height.eq(None::<i64>),
                foreign_proposals::status.eq(ForeignProposalStatus::New.to_string()),
            ))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposals_clear_proposed_in",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_send_counters_set(
        &mut self,
        foreign_send_counter: &ForeignSendCounters,
        block_id: &BlockId,
    ) -> Result<(), StorageError> {
        use crate::schema::foreign_send_counters;

        let insert = (
            foreign_send_counters::block_id.eq(serialize_hex(block_id)),
            foreign_send_counters::counters.eq(serialize_json(&foreign_send_counter.counters)?),
        );

        diesel::insert_into(foreign_send_counters::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_send_counters_set",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_receive_counters_set(
        &mut self,
        foreign_receive_counter: &ForeignReceiveCounters,
    ) -> Result<(), StorageError> {
        use crate::schema::foreign_receive_counters;

        let insert = (foreign_receive_counters::counters.eq(serialize_json(&foreign_receive_counter.counters)?),);

        diesel::insert_into(foreign_receive_counters::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_receive_counters_set",
                source: e,
            })?;

        Ok(())
    }

    fn transactions_insert(&mut self, tx_rec: &TransactionRecord) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let transaction = tx_rec.transaction();
        let insert = (
            transactions::transaction_id.eq(serialize_hex(transaction.id())),
            transactions::fee_instructions.eq(serialize_json(transaction.fee_instructions())?),
            transactions::instructions.eq(serialize_json(transaction.instructions())?),
            transactions::signatures.eq(serialize_json(transaction.signatures())?),
            transactions::inputs.eq(serialize_json(transaction.inputs())?),
            transactions::filled_inputs.eq(serialize_json(transaction.filled_inputs())?),
            transactions::resolved_inputs.eq(tx_rec.resolved_inputs().map(serialize_json).transpose()?),
            transactions::resulting_outputs.eq(tx_rec.resulting_outputs().map(serialize_json).transpose()?),
            transactions::result.eq(tx_rec.execution_result().map(serialize_json).transpose()?),
            transactions::execution_time_ms.eq(tx_rec
                .execution_time()
                .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))),
            transactions::final_decision.eq(tx_rec.final_decision().map(|d| d.to_string())),
            transactions::finalized_at.eq(tx_rec
                .finalized_time()
                .map(|t| {
                    let now = OffsetDateTime::now_utc().saturating_sub(t.try_into()?);
                    Ok(PrimitiveDateTime::new(now.date(), now.time()))
                })
                .transpose()
                .map_err(|e: time::error::ConversionRange| StorageError::QueryError {
                    reason: format!("Cannot convert finalize time into PrimitiveDateTime: {e}"),
                })?),
            transactions::abort_details.eq(tx_rec.abort_reason().map(serialize_json).transpose()?),
            transactions::min_epoch.eq(transaction.min_epoch().map(|e| e.as_u64() as i64)),
            transactions::max_epoch.eq(transaction.max_epoch().map(|e| e.as_u64() as i64)),
        );

        diesel::insert_into(transactions::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_insert",
                source: e,
            })?;

        Ok(())
    }

    fn transactions_update(&mut self, transaction_rec: &TransactionRecord) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let transaction = transaction_rec.transaction();

        #[derive(AsChangeset)]
        #[diesel(table_name = transactions)]
        struct Changes {
            result: Option<String>,
            filled_inputs: String,
            resulting_outputs: Option<String>,
            resolved_inputs: Option<String>,
            execution_time_ms: Option<i64>,
            final_decision: Option<String>,
            finalized_at: Option<PrimitiveDateTime>,
            abort_details: Option<String>,
        }

        let change_set = Changes {
            result: transaction_rec.execution_result().map(serialize_json).transpose()?,
            filled_inputs: serialize_json(transaction.filled_inputs())?,
            resulting_outputs: transaction_rec.resulting_outputs().map(serialize_json).transpose()?,
            resolved_inputs: transaction_rec.resolved_inputs().map(serialize_json).transpose()?,
            execution_time_ms: transaction_rec
                .execution_time()
                .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),

            final_decision: transaction_rec.final_decision().map(|d| d.to_string()),
            finalized_at: transaction_rec.final_decision().map(|_| {
                let now = OffsetDateTime::now_utc();
                PrimitiveDateTime::new(now.date(), now.time())
            }),
            abort_details: transaction_rec.abort_reason.as_ref().map(serialize_json).transpose()?,
        };

        let num_affected = diesel::update(transactions::table)
            .filter(transactions::transaction_id.eq(serialize_hex(transaction.id())))
            .set(change_set)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_update",
                source: e,
            })?;

        if num_affected == 0 {
            return Err(StorageError::NotFound {
                item: "transaction".to_string(),
                key: transaction.id().to_string(),
            });
        }

        Ok(())
    }

    fn transactions_save_all<'a, I: IntoIterator<Item = &'a TransactionRecord>>(
        &mut self,
        txs: I,
    ) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let insert = txs
            .into_iter()
            .map(|rec| {
                let transaction = rec.transaction();
                Ok((
                    transactions::transaction_id.eq(serialize_hex(transaction.id())),
                    transactions::fee_instructions.eq(serialize_json(transaction.fee_instructions())?),
                    transactions::instructions.eq(serialize_json(transaction.instructions())?),
                    transactions::signatures.eq(serialize_json(transaction.signatures())?),
                    transactions::inputs.eq(serialize_json(transaction.inputs())?),
                    transactions::resolved_inputs.eq(rec.resolved_inputs().map(serialize_json).transpose()?),
                    transactions::filled_inputs.eq(serialize_json(transaction.filled_inputs())?),
                    transactions::resulting_outputs.eq(rec.resulting_outputs().map(serialize_json).transpose()?),
                    transactions::result.eq(rec.execution_result().map(serialize_json).transpose()?),
                ))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        diesel::insert_or_ignore_into(transactions::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_insert",
                source: e,
            })?;

        Ok(())
    }

    fn transactions_finalize_all<'a, I: IntoIterator<Item = &'a TransactionPoolRecord>>(
        &mut self,
        block_id: BlockId,
        transactions: I,
    ) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let changes = transactions
            .into_iter()
            .map(|rec| {
                // TODO(perf): 2n queries, query is slow
                let exec = self
                    .transaction_executions_get_pending_for_block(rec.transaction_id(), &block_id)
                    .optional()?
                    .ok_or_else(|| StorageError::DataInconsistency {
                        details: format!(
                            "transactions_finalize_all: No pending execution for transaction {}",
                            rec.transaction_id()
                        ),
                    })?;

                Ok((
                    transactions::transaction_id.eq(serialize_hex(rec.transaction_id())),
                    (
                        transactions::resolved_inputs.eq(serialize_json(&exec.resolved_inputs())?),
                        transactions::resulting_outputs.eq(serialize_json(&exec.resulting_outputs())?),
                        transactions::result.eq(serialize_json(&exec.result())?),
                        transactions::execution_time_ms.eq(exec.execution_time().as_millis() as i64),
                        transactions::final_decision.eq(rec.current_decision().to_string()),
                        transactions::abort_details.eq(exec.abort_reason().map(serialize_json).transpose()?),
                        transactions::outcome.eq(exec.result().finalize.result.to_string()),
                        transactions::finalized_at.eq(now()),
                    ),
                ))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;

        for (predicate, change) in changes {
            diesel::update(transactions::table)
                .filter(predicate)
                .set(change)
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "transactions_finalize_all",
                    source: e,
                })?;
        }

        Ok(())
    }

    fn transaction_executions_insert_or_ignore(
        &mut self,
        transaction_execution: &BlockTransactionExecution,
    ) -> Result<bool, StorageError> {
        use crate::schema::transaction_executions;

        let insert = (
            transaction_executions::block_id.eq(serialize_hex(transaction_execution.block_id())),
            transaction_executions::transaction_id.eq(serialize_hex(transaction_execution.transaction_id())),
            transaction_executions::result.eq(serialize_json(&transaction_execution.result())?),
            transaction_executions::abort_reason
                .eq(transaction_execution.abort_reason().map(serialize_json).transpose()?),
            transaction_executions::resolved_inputs.eq(serialize_json(&transaction_execution.resolved_inputs())?),
            transaction_executions::resulting_outputs.eq(serialize_json(&transaction_execution.resulting_outputs())?),
            transaction_executions::execution_time_ms
                .eq(i64::try_from(transaction_execution.execution_time().as_millis()).unwrap_or(i64::MAX)),
        );

        let num_inserted = diesel::insert_or_ignore_into(transaction_executions::table)
            .values(insert)
            .on_conflict_do_nothing()
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_executions_insert",
                source: e,
            })?;

        Ok(num_inserted > 0)
    }

    fn transaction_executions_remove_any_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        use crate::schema::transaction_executions;

        diesel::delete(transaction_executions::table)
            .filter(transaction_executions::block_id.eq(serialize_hex(block_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_executions_remove_any_by_block_id",
                source: e,
            })?;

        Ok(())
    }

    fn transaction_pool_insert_new(
        &mut self,
        tx_id: TransactionId,
        decision: Decision,
        is_ready: bool,
    ) -> Result<(), StorageError> {
        use crate::schema::transaction_pool;

        let insert = (
            transaction_pool::transaction_id.eq(serialize_hex(tx_id)),
            transaction_pool::original_decision.eq(decision.to_string()),
            transaction_pool::stage.eq(TransactionPoolStage::New.to_string()),
            transaction_pool::is_ready.eq(is_ready),
        );

        diesel::insert_into(transaction_pool::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_insert",
                source: e,
            })?;

        Ok(())
    }

    fn transaction_pool_add_pending_update(
        &mut self,
        block_id: &BlockId,
        update: &TransactionPoolStatusUpdate,
    ) -> Result<(), StorageError> {
        use crate::schema::{blocks, transaction_pool, transaction_pool_state_updates};

        let transaction_id = serialize_hex(update.transaction_id());
        let block_id = serialize_hex(block_id);

        let values = (
            transaction_pool_state_updates::block_id.eq(&block_id),
            transaction_pool_state_updates::block_height.eq(blocks::table
                .select(blocks::height)
                .filter(blocks::block_id.eq(&block_id))
                .single_value()
                .assume_not_null()),
            transaction_pool_state_updates::transaction_id.eq(&transaction_id),
            transaction_pool_state_updates::evidence.eq(serialize_json(update.evidence())?),
            transaction_pool_state_updates::stage.eq(update.stage().to_string()),
            transaction_pool_state_updates::local_decision.eq(update.decision().to_string()),
            transaction_pool_state_updates::remote_decision.eq(update.remote_decision().map(|d| d.to_string())),
            transaction_pool_state_updates::transaction_fee.eq(update.transaction_fee() as i64),
            transaction_pool_state_updates::leader_fee.eq(update.leader_fee().map(serialize_json).transpose()?),
            transaction_pool_state_updates::is_ready.eq(update.is_ready()),
        );

        diesel::insert_into(transaction_pool_state_updates::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_add_pending_update",
                source: e,
            })?;

        // Set is_ready and pending_stage to the updated values. This allows has_uncommitted_transactions to return an
        // accurate value without querying records in the updates table.
        diesel::update(transaction_pool::table)
            .filter(transaction_pool::transaction_id.eq(&transaction_id))
            .set((
                transaction_pool::is_ready.eq(update.is_ready_now()),
                transaction_pool::pending_stage.eq(update.stage().to_string()),
            ))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_add_pending_update",
                source: e,
            })?;

        Ok(())
    }

    fn transaction_pool_remove(&mut self, transaction_id: &TransactionId) -> Result<(), StorageError> {
        use crate::schema::{transaction_pool, transaction_pool_state_updates};

        let transaction_id = serialize_hex(transaction_id);
        let num_affected = diesel::delete(transaction_pool::table)
            .filter(transaction_pool::transaction_id.eq(&transaction_id))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_remove",
                source: e,
            })?;

        if num_affected == 0 {
            return Err(StorageError::NotFound {
                item: "transaction".to_string(),
                key: transaction_id,
            });
        }

        diesel::delete(transaction_pool_state_updates::table)
            .filter(transaction_pool_state_updates::transaction_id.eq(transaction_id))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_remove",
                source: e,
            })?;

        Ok(())
    }

    fn transaction_pool_remove_all<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<Vec<TransactionPoolRecord>, StorageError> {
        use crate::schema::{transaction_pool, transaction_pool_state_updates};

        let transaction_ids = transaction_ids.into_iter().map(serialize_hex).collect::<Vec<_>>();

        let txs = diesel::delete(transaction_pool::table)
            .filter(transaction_pool::transaction_id.eq_any(&transaction_ids))
            .returning(transaction_pool::all_columns)
            .get_results::<sql_models::TransactionPoolRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_remove_all",
                source: e,
            })?;

        if txs.len() != transaction_ids.len() {
            return Err(SqliteStorageError::NotAllTransactionsFound {
                operation: "transaction_pool_remove_all",
                details: format!(
                    "Found {} transactions, but {} were queried",
                    txs.len(),
                    transaction_ids.len()
                ),
            }
            .into());
        }

        diesel::delete(transaction_pool_state_updates::table)
            .filter(transaction_pool_state_updates::transaction_id.eq_any(&transaction_ids))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_remove_all",
                source: e,
            })?;

        txs.into_iter().map(|tx| tx.try_convert(None)).collect()
    }

    fn transaction_pool_confirm_all_transitions(&mut self, new_locked_block: &LockedBlock) -> Result<(), StorageError> {
        use crate::schema::{transaction_pool, transaction_pool_state_updates};

        let updates = transaction_pool_state_updates::table
            .filter(transaction_pool_state_updates::block_id.eq(serialize_hex(new_locked_block.block_id())))
            .filter(transaction_pool_state_updates::is_applied.eq(false))
            .get_results::<sql_models::TransactionPoolStateUpdate>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_confirm_all_transitions",
                source: e,
            })?;

        debug!(
            target: LOG_TARGET,
            "transaction_pool_confirm_all_transitions: new_locked_block={}, {} updates",  new_locked_block, updates.len()
        );

        diesel::update(transaction_pool_state_updates::table)
            .filter(transaction_pool_state_updates::id.eq_any(updates.iter().map(|u| u.id)))
            .filter(transaction_pool_state_updates::block_height.le(new_locked_block.height().as_u64() as i64))
            .set(transaction_pool_state_updates::is_applied.eq(true))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_confirm_all_transitions",
                source: e,
            })?;

        #[derive(AsChangeset, Default)]
        #[diesel(table_name = transaction_pool)]
        struct TransactionPoolChangeSet {
            stage: Option<String>,
            local_decision: Option<String>,
            transaction_fee: Option<i64>,
            leader_fee: Option<Option<String>>,
            evidence: Option<Option<String>>,
            is_ready: Option<bool>,
            confirm_stage: Option<Option<String>>,
            remote_decision: Option<Option<String>>,
            updated_at: Option<PrimitiveDateTime>,
        }

        for update in updates {
            let confirm_stage = match update.stage.as_str() {
                "LocalPrepared" => Some(Some(TransactionPoolConfirmedStage::ConfirmedPrepared.to_string())),
                "LocalAccepted" => Some(Some(TransactionPoolConfirmedStage::ConfirmedAccepted.to_string())),
                _ => None,
            };
            let changeset = TransactionPoolChangeSet {
                stage: Some(update.stage),
                local_decision: Some(update.local_decision),
                transaction_fee: Some(update.transaction_fee),
                // Only update if Some. This isn't technically necessary since leader fee should be in every update, but
                // it does shorten the update query FWIW.
                leader_fee: update.leader_fee.map(Some),
                evidence: Some(Some(update.evidence)),
                is_ready: Some(update.is_ready),
                confirm_stage,
                remote_decision: Some(update.remote_decision),
                updated_at: Some(now()),
            };

            diesel::update(transaction_pool::table)
                .filter(transaction_pool::transaction_id.eq(&update.transaction_id))
                .set(changeset)
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "transaction_pool_confirm_all_transitions",
                    source: e,
                })?;
        }

        Ok(())
    }

    fn transaction_pool_state_updates_remove_any_by_block_id(
        &mut self,
        block_id: &BlockId,
    ) -> Result<(), StorageError> {
        use crate::schema::transaction_pool_state_updates;

        diesel::delete(transaction_pool_state_updates::table)
            .filter(transaction_pool_state_updates::block_id.eq(serialize_hex(block_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_state_updates_remove_any_by_block_id",
                source: e,
            })?;

        Ok(())
    }

    fn missing_transactions_insert<'a, IMissing: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        block: &Block,
        foreign_proposals: &[ForeignProposal],
        missing_transaction_ids: IMissing,
    ) -> Result<(), StorageError> {
        use crate::schema::missing_transactions;

        let missing_transaction_ids = missing_transaction_ids.into_iter().map(serialize_hex);
        let block_id_hex = serialize_hex(block.id());

        self.parked_blocks_insert(block, foreign_proposals)?;

        let values = missing_transaction_ids
            .map(|tx_id| {
                (
                    missing_transactions::block_id.eq(&block_id_hex),
                    missing_transactions::block_height.eq(block.height().as_u64() as i64),
                    missing_transactions::transaction_id.eq(tx_id),
                    missing_transactions::is_awaiting_execution.eq(false),
                )
            })
            .collect::<Vec<_>>();

        diesel::insert_into(missing_transactions::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "missing_transactions_insert",
                source: e,
            })?;

        Ok(())
    }

    fn missing_transactions_remove(
        &mut self,
        current_height: NodeHeight,
        transaction_id: &TransactionId,
    ) -> Result<Option<(Block, Vec<ForeignProposal>)>, StorageError> {
        use crate::schema::missing_transactions;

        let transaction_id = serialize_hex(transaction_id);
        let block_id = missing_transactions::table
            .select(missing_transactions::block_id)
            .filter(missing_transactions::transaction_id.eq(&transaction_id))
            .filter(missing_transactions::block_height.eq(current_height.as_u64() as i64))
            .first::<String>(self.connection())
            .optional()
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "missing_transactions_remove",
                source: e,
            })?;
        let Some(block_id) = block_id else {
            return Ok(None);
        };

        diesel::delete(missing_transactions::table)
            .filter(missing_transactions::transaction_id.eq(&transaction_id))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "missing_transactions_remove",
                source: e,
            })?;
        let num_remaining = missing_transactions::table
            .filter(missing_transactions::block_id.eq(&block_id))
            .count()
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "missing_transactions_remove",
                source: e,
            })?;

        if num_remaining == 0 {
            // delete all entries that are for previous heights
            diesel::delete(missing_transactions::table)
                .filter(missing_transactions::block_height.lt(current_height.as_u64() as i64))
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "missing_transactions_remove",
                    source: e,
                })?;
            let block = self.parked_blocks_remove(&block_id)?;
            return Ok(Some(block));
        }

        Ok(None)
    }

    fn foreign_parked_blocks_insert(&mut self, park_block: &ForeignParkedProposal) -> Result<(), StorageError> {
        use crate::schema::foreign_parked_blocks;

        let values = (
            foreign_parked_blocks::block_id.eq(serialize_hex(park_block.block().id())),
            foreign_parked_blocks::block.eq(serialize_json(park_block.block())?),
            foreign_parked_blocks::block_pledges.eq(serialize_json(park_block.block_pledge())?),
            foreign_parked_blocks::justify_qc.eq(serialize_json(park_block.justify_qc())?),
        );

        diesel::insert_into(foreign_parked_blocks::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_parked_blocks_insert",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_parked_blocks_insert_missing_transactions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        park_block_id: &BlockId,
        missing_transaction_ids: I,
    ) -> Result<(), StorageError> {
        use crate::schema::{foreign_missing_transactions, foreign_parked_blocks};

        let parked_block_id = foreign_parked_blocks::table
            .select(foreign_parked_blocks::id)
            .filter(foreign_parked_blocks::block_id.eq(serialize_hex(park_block_id)))
            .first::<i32>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_parked_blocks_insert_missing_transactions",
                source: e,
            })?;

        let values = missing_transaction_ids
            .into_iter()
            .map(|tx_id| {
                (
                    foreign_missing_transactions::parked_block_id.eq(parked_block_id),
                    foreign_missing_transactions::transaction_id.eq(serialize_hex(tx_id)),
                )
            })
            .collect::<Vec<_>>();

        diesel::insert_into(foreign_missing_transactions::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_parked_blocks_insert_missing_transactions",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_parked_blocks_remove_all_by_transaction(
        &mut self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<ForeignParkedProposal>, StorageError> {
        use crate::schema::{foreign_missing_transactions, foreign_parked_blocks};

        let transaction_id = serialize_hex(transaction_id);

        let removed_ids = diesel::delete(foreign_missing_transactions::table)
            .filter(foreign_missing_transactions::transaction_id.eq(&transaction_id))
            .returning(foreign_missing_transactions::parked_block_id)
            .get_results::<i32>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_parked_blocks_remove_all_by_transaction",
                source: e,
            })?;

        if removed_ids.is_empty() {
            return Ok(vec![]);
        }
        let counts = foreign_parked_blocks::table
            .select((
                foreign_parked_blocks::id,
                foreign_missing_transactions::table
                    .select(count_star())
                    .filter(foreign_missing_transactions::parked_block_id.eq(foreign_parked_blocks::id))
                    .single_value(),
            ))
            .filter(foreign_parked_blocks::id.eq_any(&removed_ids))
            .get_results::<(i32, Option<i64>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_parked_blocks_remove_all_by_transaction",
                source: e,
            })?;

        let mut remaining = counts
            .iter()
            .filter(|(_, count)| count.map_or(true, |c| c == 0))
            .map(|(id, _)| *id)
            .peekable();

        // If there are still missing transactions for ALL parked blocks, then we exit early
        if remaining.peek().is_none() {
            return Ok(vec![]);
        }

        let blocks = diesel::delete(foreign_parked_blocks::table)
            .filter(foreign_parked_blocks::id.eq_any(remaining))
            .get_results::<sql_models::ForeignParkedBlock>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_parked_blocks_remove_all_by_transaction",
                source: e,
            })?;

        blocks.into_iter().map(TryInto::try_into).collect()
    }

    fn votes_insert(&mut self, vote: &Vote) -> Result<(), StorageError> {
        use crate::schema::votes;

        let insert = (
            votes::hash.eq(serialize_hex(vote.calculate_hash())),
            votes::epoch.eq(vote.epoch.as_u64() as i64),
            votes::block_id.eq(serialize_hex(vote.block_id)),
            votes::sender_leaf_hash.eq(serialize_hex(vote.sender_leaf_hash)),
            votes::decision.eq(i32::from(vote.decision.as_u8())),
            votes::signature.eq(serialize_json(&vote.signature)?),
        );

        diesel::insert_into(votes::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "votes_insert",
                source: e,
            })?;

        Ok(())
    }

    fn substate_locks_insert_all<'a, I: IntoIterator<Item = (&'a SubstateId, &'a Vec<SubstateLock>)>>(
        &mut self,
        block_id: &BlockId,
        locks: I,
    ) -> Result<(), StorageError> {
        use crate::schema::substate_locks;

        let mut iter = locks.into_iter();
        const CHUNK_SIZE: usize = 100;
        // We have to break up into multiple queries because we can hit max SQL variable limit
        loop {
            let locks = iter
                .by_ref()
                .take(CHUNK_SIZE)
                .flat_map(|(id, locks)| {
                    let block_id = serialize_hex(block_id);
                    locks.iter().map(move |lock| {
                        (
                            substate_locks::block_id.eq(block_id.clone()),
                            substate_locks::substate_id.eq(id.to_string()),
                            substate_locks::version.eq(lock.version() as i32),
                            substate_locks::transaction_id.eq(serialize_hex(lock.transaction_id())),
                            substate_locks::lock.eq(lock.substate_lock().to_string()),
                            substate_locks::is_local_only.eq(lock.is_local_only()),
                        )
                    })
                })
                .collect::<Vec<_>>();

            let count = locks.len();
            if count == 0 {
                break;
            }

            diesel::insert_into(substate_locks::table)
                .values(locks)
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "substate_locks_insert_all",
                    source: e,
                })?;

            if count < CHUNK_SIZE {
                break;
            }
        }

        Ok(())
    }

    fn substate_locks_remove_many_for_transactions<'a, I: Iterator<Item = &'a TransactionId>>(
        &mut self,
        mut transaction_ids: Peekable<I>,
    ) -> Result<(), StorageError> {
        use crate::schema::substate_locks;

        // NOTE: looked at the diesel code and if the iterator is empty, this executes WHERE 0=1 which is fine, but
        // let's check the peekable iterator to save an OP.
        if transaction_ids.peek().is_none() {
            return Ok(());
        }

        diesel::delete(substate_locks::table)
            .filter(substate_locks::transaction_id.eq_any(transaction_ids.map(serialize_hex)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_locks_release_all_by_substates",
                source: e,
            })?;

        Ok(())
    }

    fn substate_locks_remove_any_by_block_id(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        use crate::schema::substate_locks;

        diesel::delete(substate_locks::table)
            .filter(substate_locks::block_id.eq(serialize_hex(block_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_locks_remove_any_by_block_id",
                source: e,
            })?;

        Ok(())
    }

    fn substates_create(&mut self, substate: &SubstateRecord) -> Result<(), StorageError> {
        use crate::schema::{state_transitions, substates};

        if substate.is_destroyed() {
            return Err(StorageError::QueryError {
                reason: format!(
                    "calling substates_create with a destroyed SubstateRecord is not valid. substate_id = {}",
                    substate.substate_id
                ),
            });
        }

        let values = (
            substates::address.eq(serialize_hex(substate.to_substate_address())),
            substates::substate_id.eq(substate.substate_id.to_string()),
            substates::version.eq(substate.version as i32),
            substates::data.eq(serialize_json(&substate.substate_value)?),
            substates::state_hash.eq(serialize_hex(substate.state_hash)),
            substates::created_by_transaction.eq(serialize_hex(substate.created_by_transaction)),
            substates::created_justify.eq(serialize_hex(substate.created_justify)),
            substates::created_block.eq(serialize_hex(substate.created_block)),
            substates::created_height.eq(substate.created_height.as_u64() as i64),
            substates::created_at_epoch.eq(substate.created_at_epoch.as_u64() as i64),
            substates::created_by_shard.eq(substate.created_by_shard.as_u32() as i32),
        );

        diesel::insert_into(substates::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_create",
                source: e,
            })?;

        let seq = state_transitions::table
            .select(dsl::max(state_transitions::seq))
            .filter(state_transitions::shard.eq(substate.created_by_shard.as_u32() as i32))
            .first::<Option<i64>>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_create",
                source: e,
            })?;
        let next_seq = seq.map(|s| s + 1).unwrap_or(1);

        // This means that we MUST do the state tree updates before inserting substates
        let version = self.state_tree_versions_get_latest(substate.created_by_shard)?;
        let values = (
            state_transitions::seq.eq(next_seq),
            state_transitions::epoch.eq(substate.created_at_epoch.as_u64() as i64),
            state_transitions::shard.eq(substate.created_by_shard.as_u32() as i32),
            state_transitions::substate_address.eq(serialize_hex(substate.to_substate_address())),
            state_transitions::substate_id.eq(substate.substate_id.to_string()),
            state_transitions::version.eq(substate.version as i32),
            state_transitions::transition.eq("UP"),
            state_transitions::state_hash.eq(serialize_hex(substate.state_hash)),
            state_transitions::state_version.eq(version.unwrap_or(0) as i64),
        );

        diesel::insert_into(state_transitions::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_create",
                source: e,
            })?;

        Ok(())
    }

    fn substates_down(
        &mut self,
        versioned_substate_id: VersionedSubstateId,
        shard: Shard,
        epoch: Epoch,
        destroyed_block_height: NodeHeight,
        destroyed_transaction_id: &TransactionId,
        destroyed_qc_id: &QcId,
    ) -> Result<(), StorageError> {
        use crate::schema::{state_transitions, substates};

        let changes = (
            substates::destroyed_at.eq(diesel::dsl::now),
            substates::destroyed_by_transaction.eq(Some(serialize_hex(destroyed_transaction_id))),
            substates::destroyed_by_block.eq(Some(destroyed_block_height.as_u64() as i64)),
            substates::destroyed_at_epoch.eq(Some(epoch.as_u64() as i64)),
            substates::destroyed_by_shard.eq(Some(shard.as_u32() as i32)),
            substates::destroyed_justify.eq(Some(serialize_hex(destroyed_qc_id))),
        );

        let address = versioned_substate_id.to_substate_address();

        diesel::update(substates::table)
            .filter(substates::address.eq(serialize_hex(address)))
            .set(changes)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_down (update substates)",
                source: e,
            })?;

        let seq = state_transitions::table
            .select(dsl::max(state_transitions::seq))
            .filter(state_transitions::shard.eq(shard.as_u32() as i32))
            .first::<Option<i64>>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_down (get max seq)",
                source: e,
            })?;
        let next_seq = seq.map(|s| s + 1).unwrap_or(1);

        let version = self.state_tree_versions_get_latest(shard)?;
        let values = (
            state_transitions::seq.eq(next_seq),
            state_transitions::epoch.eq(epoch.as_u64() as i64),
            state_transitions::shard.eq(shard.as_u32() as i32),
            state_transitions::substate_address.eq(serialize_hex(address)),
            state_transitions::substate_id.eq(versioned_substate_id.substate_id.to_string()),
            state_transitions::version.eq(versioned_substate_id.version as i32),
            state_transitions::transition.eq("DOWN"),
            state_transitions::state_version.eq(version.unwrap_or(0) as i64),
        );

        diesel::insert_into(state_transitions::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_down(insert into state_transitions)",
                source: e,
            })?;

        Ok(())
    }

    fn foreign_substate_pledges_save(
        &mut self,
        transaction_id: &TransactionId,
        shard_group: ShardGroup,
        pledges: &SubstatePledges,
    ) -> Result<(), StorageError> {
        use crate::schema::foreign_substate_pledges;
        let tx_id_hex = serialize_hex(transaction_id);

        let values = pledges.iter().map(|pledge| match pledge {
            SubstatePledge::Input {
                substate_id,
                is_write,
                substate,
            } => {
                let lock_type = if *is_write {
                    SubstateLockType::Write
                } else {
                    SubstateLockType::Read
                };
                Ok::<_, StorageError>((
                    foreign_substate_pledges::transaction_id.eq(&tx_id_hex),
                    foreign_substate_pledges::address.eq(serialize_hex(substate_id.to_substate_address())),
                    foreign_substate_pledges::substate_id.eq(substate_id.substate_id().to_string()),
                    foreign_substate_pledges::version.eq(substate_id.version() as i32),
                    foreign_substate_pledges::shard_group.eq(shard_group.encode_as_u32() as i32),
                    foreign_substate_pledges::lock_type.eq(lock_type.to_string()),
                    foreign_substate_pledges::substate_value.eq(Some(serialize_json(&substate)?)),
                ))
            },
            SubstatePledge::Output { substate_id } => Ok::<_, StorageError>((
                foreign_substate_pledges::transaction_id.eq(&tx_id_hex),
                foreign_substate_pledges::address.eq(serialize_hex(substate_id.to_substate_address())),
                foreign_substate_pledges::substate_id.eq(substate_id.substate_id().to_string()),
                foreign_substate_pledges::version.eq(substate_id.version() as i32),
                foreign_substate_pledges::shard_group.eq(shard_group.encode_as_u32() as i32),
                foreign_substate_pledges::lock_type.eq(SubstateLockType::Output.to_string()),
                foreign_substate_pledges::substate_value.eq(None),
            )),
        });

        for value in values {
            diesel::insert_into(foreign_substate_pledges::table)
                .values(value?)
                // This is not supported for batch inserts, which is why we do multiple inserts
                .on_conflict_do_nothing()
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "foreign_substate_pledges_insert",
                    source: e,
                })?;
        }

        Ok(())
    }

    fn foreign_substate_pledges_remove_many<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        transaction_ids: I,
    ) -> Result<(), StorageError> {
        use crate::schema::foreign_substate_pledges;

        let num_deleted = diesel::delete(foreign_substate_pledges::table)
            .filter(foreign_substate_pledges::transaction_id.eq_any(transaction_ids.into_iter().map(serialize_hex)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_substate_pledges_remove_many",
                source: e,
            })?;

        debug!(
            target: LOG_TARGET,
            "Deleted {num_deleted} foreign substate pledges",
        );

        Ok(())
    }

    fn pending_state_tree_diffs_insert(
        &mut self,
        block_id: BlockId,
        shard: Shard,
        diff: &VersionedStateHashTreeDiff,
    ) -> Result<(), StorageError> {
        use crate::schema::{blocks, pending_state_tree_diffs};

        let insert = (
            pending_state_tree_diffs::block_id.eq(serialize_hex(block_id)),
            pending_state_tree_diffs::shard.eq(shard.as_u32() as i32),
            pending_state_tree_diffs::block_height.eq(blocks::table
                .select(blocks::height)
                .filter(blocks::block_id.eq(serialize_hex(block_id)))
                .single_value()
                .assume_not_null()),
            pending_state_tree_diffs::version.eq(diff.version as i64),
            pending_state_tree_diffs::diff_json.eq(serialize_json(&diff.diff)?),
        );

        diesel::insert_into(pending_state_tree_diffs::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "pending_state_tree_diffs_insert",
                source: e,
            })?;

        Ok(())
    }

    fn pending_state_tree_diffs_remove_by_block(&mut self, block_id: &BlockId) -> Result<(), StorageError> {
        use crate::schema::pending_state_tree_diffs;

        diesel::delete(pending_state_tree_diffs::table)
            .filter(pending_state_tree_diffs::block_id.eq(serialize_hex(block_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "pending_state_tree_diffs_remove_by_block",
                source: e,
            })?;

        Ok(())
    }

    fn pending_state_tree_diffs_remove_and_return_by_block(
        &mut self,
        block_id: &BlockId,
    ) -> Result<IndexMap<Shard, Vec<PendingShardStateTreeDiff>>, StorageError> {
        use crate::schema::pending_state_tree_diffs;

        let diff_recs = pending_state_tree_diffs::table
            .filter(pending_state_tree_diffs::block_id.eq(serialize_hex(block_id)))
            .order_by(pending_state_tree_diffs::block_height.asc())
            .get_results::<sql_models::PendingStateTreeDiff>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "pending_state_tree_diffs_remove_by_block",
                source: e,
            })?;

        diesel::delete(pending_state_tree_diffs::table)
            .filter(pending_state_tree_diffs::id.eq_any(diff_recs.iter().map(|d| d.id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "pending_state_tree_diffs_remove_by_block",
                source: e,
            })?;

        let mut diffs = IndexMap::new();
        for diff in diff_recs {
            let shard = Shard::from(diff.shard as u32);
            let diff = PendingShardStateTreeDiff::try_from(diff)?;
            diffs.entry(shard).or_insert_with(Vec::new).push(diff);
        }

        Ok(diffs)
    }

    fn state_tree_nodes_insert(&mut self, shard: Shard, key: NodeKey, node: Node<Version>) -> Result<(), StorageError> {
        use crate::schema::state_tree;

        let node = TreeNode::new_latest(node);
        let node = serde_json::to_string(&node).map_err(|e| StorageError::QueryError {
            reason: format!("Failed to serialize node: {}", e),
        })?;

        let values = (
            state_tree::shard.eq(shard.as_u32() as i32),
            state_tree::key.eq(key.to_string()),
            state_tree::node.eq(&node),
        );
        diesel::insert_into(state_tree::table)
            .values(&values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "state_tree_nodes_insert",
                source: e,
            })?;

        Ok(())
    }

    fn state_tree_nodes_record_stale_tree_node(
        &mut self,
        shard: Shard,
        node: StaleTreeNode,
    ) -> Result<(), StorageError> {
        use crate::schema::state_tree;

        //   let num_effected = diesel::update(state_tree::table)
        //             .filter(state_tree::shard.eq(shard.as_u32() as i32))
        //             .filter(state_tree::key.eq(key.to_string()))
        //             .set(state_tree::is_stale.eq(true))
        //             .execute(self.connection())
        //             .map_err(|e| SqliteStorageError::DieselError {
        //                 operation: "state_tree_nodes_mark_stale_tree_node",
        //                 source: e,
        //             })?;

        let key = node.as_node_key();
        let num_effected = diesel::delete(state_tree::table)
            .filter(state_tree::shard.eq(shard.as_u32() as i32))
            .filter(state_tree::key.eq(key.to_string()))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "state_tree_nodes_mark_stale_tree_node",
                source: e,
            })?;

        if num_effected == 0 {
            return Err(StorageError::NotFound {
                item: "state_tree_node".to_string(),
                key: key.to_string(),
            });
        }

        Ok(())
    }

    fn state_tree_shard_versions_set(&mut self, shard: Shard, version: Version) -> Result<(), StorageError> {
        use crate::schema::state_tree_shard_versions;

        let values = (
            state_tree_shard_versions::shard.eq(shard.as_u32() as i32),
            state_tree_shard_versions::version.eq(version as i64),
        );

        diesel::insert_into(state_tree_shard_versions::table)
            .values(&values)
            .on_conflict(state_tree_shard_versions::shard)
            .do_update()
            .set(state_tree_shard_versions::version.eq(version as i64))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "state_tree_shard_versions_increment",
                source: e,
            })?;

        Ok(())
    }

    fn epoch_checkpoint_save(&mut self, checkpoint: &EpochCheckpoint) -> Result<(), StorageError> {
        use crate::schema::epoch_checkpoints;

        let values = (
            epoch_checkpoints::epoch.eq(checkpoint.block().epoch().as_u64() as i64),
            epoch_checkpoints::commit_block.eq(serialize_json(checkpoint.block())?),
            epoch_checkpoints::qcs.eq(serialize_json(checkpoint.qcs())?),
            epoch_checkpoints::shard_roots.eq(serialize_json(checkpoint.shard_roots())?),
        );

        diesel::insert_into(epoch_checkpoints::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "epoch_checkpoint_save",
                source: e,
            })?;

        Ok(())
    }

    fn burnt_utxos_insert(&mut self, burnt_utxo: &BurntUtxo) -> Result<(), StorageError> {
        use crate::schema::burnt_utxos;

        let values = (
            burnt_utxos::substate_id.eq(burnt_utxo.substate_id.to_string()),
            burnt_utxos::substate.eq(serialize_json(&burnt_utxo.substate_value)?),
            burnt_utxos::base_layer_block_height.eq(burnt_utxo.base_layer_block_height as i64),
        );

        diesel::insert_into(burnt_utxos::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "burnt_utxos_insert",
                source: e,
            })?;

        Ok(())
    }

    fn burnt_utxos_set_proposed_block(
        &mut self,
        substate_id: &SubstateId,
        proposed_in_block: &BlockId,
    ) -> Result<(), StorageError> {
        use crate::schema::{blocks, burnt_utxos};

        let proposed_in_block_hex = serialize_hex(proposed_in_block);
        let num_affected = diesel::update(burnt_utxos::table)
            .filter(burnt_utxos::substate_id.eq(substate_id.to_string()))
            .set((
                burnt_utxos::proposed_in_block.eq(&proposed_in_block_hex),
                burnt_utxos::proposed_in_block_height.eq(blocks::table
                    .select(blocks::height)
                    .filter(blocks::block_id.eq(&proposed_in_block_hex))
                    .single_value()),
            ))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "burnt_utxos_set_proposed_block",
                source: e,
            })?;

        if num_affected == 0 {
            return Err(StorageError::NotFound {
                item: "burnt_utxo".to_string(),
                key: substate_id.to_string(),
            });
        }

        Ok(())
    }

    fn burnt_utxos_clear_proposed_block(&mut self, proposed_in_block: &BlockId) -> Result<(), StorageError> {
        use crate::schema::burnt_utxos;

        let proposed_in_block_hex = serialize_hex(proposed_in_block);
        diesel::update(burnt_utxos::table)
            .filter(burnt_utxos::proposed_in_block.eq(&proposed_in_block_hex))
            .set((
                burnt_utxos::proposed_in_block.eq(None::<String>),
                burnt_utxos::proposed_in_block_height.eq(None::<i64>),
            ))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "burnt_utxos_clear_proposed_block",
                source: e,
            })?;

        Ok(())
    }

    fn burnt_utxos_delete(&mut self, substate_id: &SubstateId) -> Result<(), StorageError> {
        use crate::schema::burnt_utxos;

        let num_affected = diesel::delete(burnt_utxos::table)
            .filter(burnt_utxos::substate_id.eq(substate_id.to_string()))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "burnt_utxos_delete",
                source: e,
            })?;

        if num_affected == 0 {
            return Err(StorageError::NotFound {
                item: "burnt_utxo".to_string(),
                key: substate_id.to_string(),
            });
        }

        Ok(())
    }

    fn lock_conflicts_insert_all<'a, I: IntoIterator<Item = (&'a TransactionId, &'a Vec<LockConflict>)>>(
        &mut self,
        block_id: &BlockId,
        conflicts: I,
    ) -> Result<(), StorageError> {
        use crate::schema::lock_conflicts;

        let values = conflicts
            .into_iter()
            .flat_map(|(tx_id, conflicts)| {
                conflicts.iter().map(move |conflict| {
                    (
                        lock_conflicts::block_id.eq(serialize_hex(block_id)),
                        lock_conflicts::transaction_id.eq(serialize_hex(tx_id)),
                        lock_conflicts::depends_on_tx.eq(serialize_hex(conflict.transaction_id)),
                        lock_conflicts::lock_type.eq(conflict.requested_lock.to_string()),
                    )
                })
            })
            .collect::<Vec<_>>();

        diesel::insert_into(lock_conflicts::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "lock_conflicts_insert_all",
                source: e,
            })?;

        Ok(())
    }

    fn diagnostics_add_no_vote(&mut self, block_id: BlockId, reason: NoVoteReason) -> Result<(), StorageError> {
        use crate::schema::{blocks, diagnostics_no_votes};
        let block_id = serialize_hex(block_id);

        let values = (
            diagnostics_no_votes::block_id.eq(&block_id),
            diagnostics_no_votes::block_height.eq(blocks::table
                .select(blocks::height)
                .filter(blocks::block_id.eq(&block_id))
                .single_value()
                .assume_not_null()),
            diagnostics_no_votes::reason_code.eq(reason.as_code_str()),
            diagnostics_no_votes::reason_text.eq(reason.to_string()),
        );

        diesel::insert_into(diagnostics_no_votes::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "diagnostics_add_no_vote",
                source: e,
            })?;

        Ok(())
    }
}

impl<'a, TAddr> Deref for SqliteStateStoreWriteTransaction<'a, TAddr> {
    type Target = SqliteStateStoreReadTransaction<'a, TAddr>;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref().unwrap()
    }
}

impl<TAddr> Drop for SqliteStateStoreWriteTransaction<'_, TAddr> {
    fn drop(&mut self) {
        if self.transaction.is_some() {
            warn!(
                target: LOG_TARGET,
                "Shard store write transaction was not committed/rolled back"
            );
        }
    }
}

fn now() -> PrimitiveDateTime {
    let now = time::OffsetDateTime::now_utc();
    PrimitiveDateTime::new(now.date(), now.time())
}
