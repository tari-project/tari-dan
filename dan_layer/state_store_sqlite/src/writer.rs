//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Borrow,
    collections::HashSet,
    ops::{Deref, DerefMut},
};

use diesel::{AsChangeset, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection};
use log::*;
use tari_dan_common_types::{optional::Optional, Epoch, NodeAddressable, NodeHeight, SubstateAddress};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Decision,
        Evidence,
        ForeignProposal,
        ForeignReceiveCounters,
        ForeignSendCounters,
        HighQc,
        LastExecuted,
        LastProposed,
        LastSentVote,
        LastVoted,
        LeafBlock,
        LockedBlock,
        LockedOutput,
        PendingStateTreeDiff,
        QcId,
        QuorumCertificate,
        SubstateLockFlag,
        SubstateLockState,
        SubstateRecord,
        TransactionAtom,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        TransactionRecord,
        Vote,
    },
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};
use tari_transaction::{Transaction, TransactionId};
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

    fn parked_blocks_remove(&mut self, block_id: &str) -> Result<Block, StorageError> {
        use crate::schema::parked_blocks;

        let block = parked_blocks::table
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

        block.try_into()
    }

    fn parked_blocks_insert(&mut self, block: &Block) -> Result<(), StorageError> {
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
            parked_blocks::proposed_by.eq(serialize_hex(block.proposed_by().as_bytes())),
            parked_blocks::command_count.eq(block.commands().len() as i64),
            parked_blocks::commands.eq(serialize_json(block.commands())?),
            parked_blocks::total_leader_fee.eq(block.total_leader_fee() as i64),
            parked_blocks::justify.eq(serialize_json(block.justify())?),
            parked_blocks::foreign_indexes.eq(serialize_json(block.foreign_indexes())?),
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

    fn commit(mut self) -> Result<(), StorageError> {
        // Take so that we mark this transaction as complete in the drop impl
        self.transaction.take().unwrap().commit()?;
        Ok(())
    }

    fn rollback(mut self) -> Result<(), StorageError> {
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
            blocks::proposed_by.eq(serialize_hex(block.proposed_by().as_bytes())),
            blocks::command_count.eq(block.commands().len() as i64),
            blocks::commands.eq(serialize_json(block.commands())?),
            blocks::total_leader_fee.eq(block.total_leader_fee() as i64),
            blocks::qc_id.eq(serialize_hex(block.justify().id())),
            blocks::is_dummy.eq(block.is_dummy()),
            blocks::is_processed.eq(block.is_processed()),
            blocks::signature.eq(block.get_signature().map(serialize_json).transpose()?),
            blocks::foreign_indexes.eq(serialize_json(block.foreign_indexes())?),
        );

        diesel::insert_into(blocks::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_insert",
                source: e,
            })?;

        Ok(())
    }

    fn blocks_set_flags(
        &mut self,
        block_id: &BlockId,
        is_committed: Option<bool>,
        is_processed: Option<bool>,
    ) -> Result<(), StorageError> {
        use crate::schema::blocks;

        #[derive(AsChangeset)]
        #[diesel(table_name = blocks)]
        struct Changes {
            is_committed: Option<bool>,
            is_processed: Option<bool>,
        }
        let changes = Changes {
            is_committed,
            is_processed,
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

    fn quorum_certificates_insert(&mut self, qc: &QuorumCertificate) -> Result<(), StorageError> {
        use crate::schema::quorum_certificates;

        let insert = (
            quorum_certificates::qc_id.eq(serialize_hex(qc.id())),
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
                operation: "last_proposed_set",
                source: e,
            })?;

        Ok(())
    }

    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError> {
        use crate::schema::leaf_blocks;

        let insert = (
            leaf_blocks::block_id.eq(serialize_hex(leaf_node.block_id)),
            leaf_blocks::block_height.eq(leaf_node.height.as_u64() as i64),
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

        if let Some(existing) = self.locked_block_get().optional()? {
            if locked_block.height <= existing.height {
                return Err(StorageError::QueryError {
                    reason: format!(
                        "Locked block height {} is not greater than existing height {}",
                        locked_block.height, existing.height
                    ),
                });
            }
        }

        let insert = (
            locked_block::block_id.eq(serialize_hex(locked_block.block_id)),
            locked_block::height.eq(locked_block.height.as_u64() as i64),
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

    fn foreign_proposal_upsert(&mut self, foreign_proposal: &ForeignProposal) -> Result<(), StorageError> {
        use crate::schema::foreign_proposals;

        let values = (
            foreign_proposals::bucket.eq(foreign_proposal.bucket.as_u32() as i32),
            foreign_proposals::block_id.eq(serialize_hex(foreign_proposal.block_id)),
            foreign_proposals::state.eq(foreign_proposal.state.to_string()),
            foreign_proposals::proposed_height.eq(foreign_proposal.proposed_height.map(|h| h.as_u64() as i64)),
            foreign_proposals::transactions.eq(serialize_json(&foreign_proposal.transactions)?),
        );

        diesel::insert_into(foreign_proposals::table)
            .values(&values)
            .on_conflict((foreign_proposals::bucket, foreign_proposals::block_id))
            .do_update()
            .set(values.clone())
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposal_set",
                source: e,
            })?;
        Ok(())
    }

    fn foreign_proposal_delete(&mut self, foreign_proposal: &ForeignProposal) -> Result<(), StorageError> {
        use crate::schema::foreign_proposals;

        diesel::delete(foreign_proposals::table)
            .filter(foreign_proposals::bucket.eq(foreign_proposal.bucket.as_u32() as i32))
            .filter(foreign_proposals::block_id.eq(serialize_hex(foreign_proposal.block_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposal_delete",
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

    fn transactions_insert(&mut self, transaction: &Transaction) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let insert = (
            transactions::transaction_id.eq(serialize_hex(transaction.id())),
            transactions::fee_instructions.eq(serialize_json(transaction.fee_instructions())?),
            transactions::instructions.eq(serialize_json(transaction.instructions())?),
            transactions::signature.eq(serialize_json(transaction.signature())?),
            transactions::inputs.eq(serialize_json(transaction.inputs())?),
            transactions::input_refs.eq(serialize_json(transaction.input_refs())?),
            transactions::filled_inputs.eq(serialize_json(transaction.filled_inputs())?),
            transactions::resulting_outputs.eq(serialize_json(&serde_json::Value::Array(vec![]))?),
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
            result: Option<Option<String>>,
            filled_inputs: Option<String>,
            resulting_outputs: Option<String>,
            execution_time_ms: Option<Option<i64>>,
            final_decision: Option<Option<String>>,
            finalized_at: Option<Option<PrimitiveDateTime>>,
            abort_details: Option<Option<String>>,
        }

        let change_set = Changes {
            result: Some(transaction_rec.result().map(serialize_json).transpose()?),
            filled_inputs: Some(serialize_json(transaction.filled_inputs())?),
            resulting_outputs: Some(serialize_json(transaction_rec.resulting_outputs())?),
            execution_time_ms: Some(
                transaction_rec
                    .execution_time()
                    .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
            ),
            final_decision: Some(transaction_rec.final_decision().map(|d| d.to_string())),
            finalized_at: Some(transaction_rec.final_decision().map(|_| {
                let now = OffsetDateTime::now_utc();
                PrimitiveDateTime::new(now.date(), now.time())
            })),
            abort_details: Some(transaction_rec.abort_details.clone()),
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
                    transactions::signature.eq(serialize_json(transaction.signature())?),
                    transactions::inputs.eq(serialize_json(transaction.inputs())?),
                    transactions::input_refs.eq(serialize_json(transaction.input_refs())?),
                    transactions::filled_inputs.eq(serialize_json(transaction.filled_inputs())?),
                    transactions::resulting_outputs.eq(serialize_json(rec.resulting_outputs())?),
                    transactions::result.eq(rec.result().map(serialize_json).transpose()?),
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

    fn transaction_pool_insert(
        &mut self,
        transaction: TransactionAtom,
        stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), StorageError> {
        use crate::schema::transaction_pool;

        let insert = (
            transaction_pool::transaction_id.eq(serialize_hex(transaction.id)),
            transaction_pool::original_decision.eq(transaction.decision.to_string()),
            transaction_pool::transaction_fee.eq(transaction.transaction_fee as i64),
            transaction_pool::evidence.eq(serialize_json(&transaction.evidence)?),
            transaction_pool::leader_fee.eq(transaction.leader_fee.as_ref().map(|f| f.fee as i64)),
            transaction_pool::global_exhaust_burn
                .eq(transaction.leader_fee.as_ref().map(|f| f.global_exhaust_burn as i64)),
            transaction_pool::stage.eq(stage.to_string()),
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

    fn transaction_pool_add_pending_update(&mut self, update: TransactionPoolStatusUpdate) -> Result<(), StorageError> {
        use crate::schema::{transaction_pool, transaction_pool_state_updates};

        let transaction_id = serialize_hex(update.transaction_id());
        let block_id = serialize_hex(update.block_id());
        let values = (
            transaction_pool_state_updates::block_id.eq(&block_id),
            transaction_pool_state_updates::block_height.eq(update.block_height().as_u64() as i64),
            transaction_pool_state_updates::transaction_id.eq(&transaction_id),
            transaction_pool_state_updates::evidence.eq(serialize_json(update.evidence())?),
            transaction_pool_state_updates::stage.eq(update.stage().to_string()),
            transaction_pool_state_updates::local_decision.eq(update.local_decision().to_string()),
            transaction_pool_state_updates::is_ready.eq(update.is_ready()),
        );

        // Check if update exists for block and transaction
        let count = transaction_pool_state_updates::table
            .count()
            .filter(transaction_pool_state_updates::block_id.eq(&block_id))
            .filter(transaction_pool_state_updates::transaction_id.eq(&transaction_id))
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_add_pending_update",
                source: e,
            })?;

        if count == 0 {
            diesel::insert_into(transaction_pool_state_updates::table)
                .values(values)
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "transaction_pool_add_pending_update",
                    source: e,
                })?;
        } else {
            diesel::update(transaction_pool_state_updates::table)
                .filter(transaction_pool_state_updates::block_id.eq(&block_id))
                .filter(transaction_pool_state_updates::transaction_id.eq(&transaction_id))
                .set(values)
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "transaction_pool_add_pending_update",
                    source: e,
                })?;
        }

        // Set is_ready to the last value we set here. Bit of a hack to get has_uncommitted_transactions to return a
        // more accurate value without querying the updates table
        diesel::update(transaction_pool::table)
            .filter(transaction_pool::transaction_id.eq(&transaction_id))
            .set((
                transaction_pool::is_ready.eq(update.is_ready()),
                transaction_pool::pending_stage.eq(update.stage().to_string()),
            ))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_add_pending_update",
                source: e,
            })?;

        Ok(())
    }

    fn transaction_pool_update(
        &mut self,
        transaction_id: &TransactionId,
        local_decision: Option<Decision>,
        remote_decision: Option<Decision>,
        remote_evidence: Option<&Evidence>,
    ) -> Result<(), StorageError> {
        use crate::schema::transaction_pool;

        let transaction_id = serialize_hex(transaction_id);

        #[derive(AsChangeset)]
        #[diesel(table_name = transaction_pool)]
        struct Changes {
            remote_evidence: Option<String>,
            local_decision: Option<Option<String>>,
            remote_decision: Option<Option<String>>,
            updated_at: PrimitiveDateTime,
        }

        let change_set = Changes {
            remote_evidence: remote_evidence.map(serialize_json).transpose()?,
            local_decision: local_decision.map(|d| d.to_string()).map(Some),
            remote_decision: remote_decision.map(|d| d.to_string()).map(Some),
            updated_at: now(),
        };

        let num_affected = diesel::update(transaction_pool::table)
            .filter(transaction_pool::transaction_id.eq(&transaction_id))
            .set(change_set)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_set_remote_decision",
                source: e,
            })?;

        if num_affected == 0 {
            return Err(StorageError::NotFound {
                item: "transaction".to_string(),
                key: transaction_id,
            });
        }

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

    fn transaction_pool_set_all_transitions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        locked_block: &LockedBlock,
        new_locked_block: &LockedBlock,
        tx_ids: I,
    ) -> Result<(), StorageError> {
        use crate::schema::{transaction_pool, transaction_pool_state_updates};

        let tx_ids = tx_ids.into_iter().map(serialize_hex).collect::<Vec<_>>();

        let count = transaction_pool::table
            .count()
            .filter(transaction_pool::transaction_id.eq_any(&tx_ids))
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_set_all_transitions",
                source: e,
            })?;

        if count != tx_ids.len() as i64 {
            return Err(SqliteStorageError::NotAllTransactionsFound {
                operation: "transaction_pool_set_all_transitions",
                details: format!("Found {} transactions, but {} were queried", count, tx_ids.len()),
            }
            .into());
        }

        let updates = self.get_transaction_atom_state_updates_between_blocks(
            locked_block.block_id(),
            new_locked_block.block_id(),
            tx_ids.iter().map(|s| s.as_str()),
        )?;

        debug!(
            target: LOG_TARGET,
            "transaction_pool_set_all_transitions: locked_block={}, new_locked_block={}, {} transactions, {} updates", locked_block, new_locked_block, tx_ids.len(), updates.len()
        );

        diesel::delete(transaction_pool_state_updates::table)
            .filter(transaction_pool_state_updates::transaction_id.eq_any(&tx_ids))
            .filter(transaction_pool_state_updates::block_height.le(new_locked_block.height().as_u64() as i64))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_set_all_transitions",
                source: e,
            })?;

        for update in updates.into_values() {
            diesel::update(transaction_pool::table)
                .filter(transaction_pool::transaction_id.eq(&update.transaction_id))
                .set((
                    transaction_pool::stage.eq(update.stage),
                    transaction_pool::local_decision.eq(update.local_decision),
                    transaction_pool::evidence.eq(update.evidence),
                    transaction_pool::is_ready.eq(update.is_ready),
                    transaction_pool::updated_at.eq(now()),
                ))
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "transaction_pool_set_all_transitions",
                    source: e,
                })?;
        }

        Ok(())
    }

    fn missing_transactions_insert<
        'a,
        IMissing: IntoIterator<Item = &'a TransactionId>,
        IAwaiting: IntoIterator<Item = &'a TransactionId>,
    >(
        &mut self,
        block: &Block,
        missing_transaction_ids: IMissing,
        awaiting_transaction_ids: IAwaiting,
    ) -> Result<(), StorageError> {
        use crate::schema::missing_transactions;

        let missing_transaction_ids = missing_transaction_ids
            .into_iter()
            .map(serialize_hex)
            .collect::<Vec<_>>();
        let awaiting_transaction_ids = awaiting_transaction_ids
            .into_iter()
            .map(serialize_hex)
            .collect::<Vec<_>>();
        let block_id_hex = serialize_hex(block.id());

        self.parked_blocks_insert(block)?;

        let values = missing_transaction_ids
            .iter()
            .map(|tx_id| {
                (
                    missing_transactions::block_id.eq(&block_id_hex),
                    missing_transactions::block_height.eq(block.height().as_u64() as i64),
                    missing_transactions::transaction_id.eq(tx_id),
                    missing_transactions::is_awaiting_execution.eq(false),
                )
            })
            .chain(awaiting_transaction_ids.iter().map(|tx_id| {
                (
                    missing_transactions::block_id.eq(&block_id_hex),
                    missing_transactions::block_height.eq(block.height().as_u64() as i64),
                    missing_transactions::transaction_id.eq(tx_id),
                    missing_transactions::is_awaiting_execution.eq(true),
                )
            }))
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
    ) -> Result<Option<Block>, StorageError> {
        use crate::schema::{missing_transactions, transactions};

        // delete all entries that are for previous heights
        diesel::delete(missing_transactions::table)
            .filter(missing_transactions::block_height.lt(current_height.as_u64().saturating_sub(1) as i64))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "missing_transactions_remove",
                source: e,
            })?;

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
        let mut missing_transactions = missing_transactions::table
            .select(missing_transactions::transaction_id)
            .filter(missing_transactions::block_id.eq(&block_id))
            .get_results::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "missing_transactions_remove",
                source: e,
            })?;

        if missing_transactions.is_empty() {
            return self.parked_blocks_remove(&block_id).map(Some);
        }

        // Make double sure that we dont have these transactions due to a race condition between inserting
        // missing transactions and them completing execution.
        let found_transaction_ids = transactions::table
            .select(transactions::transaction_id)
            .filter(transactions::transaction_id.eq_any(&missing_transactions))
            .filter(transactions::result.is_not_null())
            .get_results::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_get_many",
                source: e,
            })?;

        diesel::delete(missing_transactions::table)
            .filter(missing_transactions::transaction_id.eq_any(&found_transaction_ids))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "missing_transactions_remove",
                source: e,
            })?;

        missing_transactions.retain(|id| found_transaction_ids.iter().all(|found| found != id));

        if missing_transactions.is_empty() {
            return self.parked_blocks_remove(&block_id).map(Some);
        }

        Ok(None)
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

    fn substates_try_lock_many<'a, I: IntoIterator<Item = &'a SubstateAddress>>(
        &mut self,
        locked_by_tx: &TransactionId,
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<SubstateLockState, StorageError> {
        use crate::schema::substates;

        // Lock unique shards
        let objects: HashSet<String> = objects.into_iter().map(serialize_hex).collect();

        let locked_details = substates::table
            .select((substates::is_locked_w, substates::destroyed_by_transaction))
            .filter(substates::address.eq_any(&objects))
            .get_results::<(bool, Option<String>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_try_lock_many",
                source: e,
            })?;
        if locked_details.len() < objects.len() {
            return Err(SqliteStorageError::NotAllSubstatesFound {
                operation: "substates_try_lock_all",
                details: format!(
                    "{:?}: Found {} substates, but {} were requested",
                    lock_flag,
                    locked_details.len(),
                    objects.len()
                ),
            }
            .into());
        }

        if locked_details.iter().any(|(w, _)| *w) {
            return Ok(SubstateLockState::SomeAlreadyWriteLocked);
        }

        if locked_details.iter().any(|(_, downed)| downed.is_some()) {
            return Ok(SubstateLockState::SomeDestroyed);
        }

        match lock_flag {
            SubstateLockFlag::Write => {
                diesel::update(substates::table)
                    .filter(substates::address.eq_any(objects))
                    .set((
                        substates::is_locked_w.eq(true),
                        substates::locked_by.eq(serialize_hex(locked_by_tx)),
                    ))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "transactions_try_lock_many(Write)",
                        source: e,
                    })?;
            },
            SubstateLockFlag::Read => {
                diesel::update(substates::table)
                    .filter(substates::address.eq_any(objects))
                    .set(substates::read_locks.eq(substates::read_locks + 1))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "transactions_try_lock_many(Read)",
                        source: e,
                    })?;
            },
        }

        Ok(SubstateLockState::LockAcquired)
    }

    fn substates_try_unlock_many<'a, I: IntoIterator<Item = &'a SubstateAddress>>(
        &mut self,
        locked_by_tx: &TransactionId,
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<(), StorageError> {
        use crate::schema::substates;

        let objects: HashSet<String> = objects.into_iter().map(serialize_hex).collect();

        match lock_flag {
            SubstateLockFlag::Write => {
                // let locked_w = substates::table
                //     .select(substates::is_locked_w)
                //     // Only the locking transaction can unlock the substates for write
                //     .filter(substates::locked_by.eq(serialize_hex(locked_by_tx)))
                //     .filter(substates::shard_id.eq_any(&objects))
                //     .get_results::<bool>(self.connection())
                //     .map_err(|e| SqliteStorageError::DieselError {
                //         operation: "substates_try_unlock_many",
                //         source: e,
                //     })?;
                // if locked_w.len() < objects.len() {
                //     return Err(SqliteStorageError::NotAllSubstatesFound {
                //         operation: "substates_try_unlock_many",
                //         details: format!(
                //             "{:?}: Found {} substates, but {} were requested",
                //             lock_flag,
                //             locked_w.len(),
                //             objects.len()
                //         ),
                //     }
                //     .into());
                // }
                // if locked_w.iter().any(|w| !*w) {
                //     return Err(SqliteStorageError::SubstatesUnlock {
                //         operation: "substates_try_unlock_many",
                //         details: "Not all substates are write locked".to_string(),
                //     }
                //     .into());
                // }

                diesel::update(substates::table)
                    .filter(substates::address.eq_any(objects))
                    .filter(substates::locked_by.eq(serialize_hex(locked_by_tx)))
                    .set((
                        substates::is_locked_w.eq(false),
                        substates::locked_by.eq(None::<String>),
                    ))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "substates_try_unlock_many",
                        source: e,
                    })?;
            },
            SubstateLockFlag::Read => {
                // let locked_r = substates::table
                //     .select(substates::read_locks)
                //     .filter(substates::shard_id.eq_any(&objects))
                //     .get_results::<i32>(self.connection())
                //     .map_err(|e| SqliteStorageError::DieselError {
                //         operation: "substates_try_unlock_many",
                //         source: e,
                //     })?;
                // if locked_r.len() < objects.len() {
                //     return Err(SqliteStorageError::NotAllSubstatesFound {
                //         operation: "substates_try_lock_all",
                //         details: format!(
                //             "Found {} substates, but {} were requested",
                //             locked_r.len(),
                //             objects.len()
                //         ),
                //     }
                //     .into());
                // }
                // if locked_r.iter().any(|r| *r == 0) {
                //     return Err(SqliteStorageError::SubstatesUnlock {
                //         operation: "substates_try_unlock_many",
                //         details: "Not all substates are read locked".to_string(),
                //     }
                //     .into());
                // }
                diesel::update(substates::table)
                    .filter(substates::address.eq_any(objects))
                    .set(substates::read_locks.eq(substates::read_locks - 1))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "substates_try_unlock_many",
                        source: e,
                    })?;
            },
        }

        Ok(())
    }

    fn substate_down_many<I: IntoIterator<Item = SubstateAddress>>(
        &mut self,
        addresses: I,
        epoch: Epoch,
        destroyed_block_id: &BlockId,
        destroyed_transaction_id: &TransactionId,
        destroyed_qc_id: &QcId,
        require_locks: bool,
    ) -> Result<(), StorageError> {
        use crate::schema::substates;

        let addresses = addresses.into_iter().map(serialize_hex).collect::<Vec<_>>();

        let is_writable = substates::table
            .select((substates::substate_id, substates::is_locked_w))
            .filter(substates::address.eq_any(&addresses))
            .get_results::<(String, bool)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_down",
                source: e,
            })?;
        if is_writable.len() != addresses.len() {
            return Err(SqliteStorageError::NotAllSubstatesFound {
                operation: "substate_down",
                details: format!(
                    "Found {} substates, but {} were requested",
                    is_writable.len(),
                    addresses.len()
                ),
            }
            .into());
        }
        if require_locks {
            if let Some((addr, _)) = is_writable.iter().find(|(_, w)| !*w) {
                return Err(SqliteStorageError::SubstatesUnlock {
                    operation: "substate_down",
                    details: format!("Substate {} is not write locked", addr),
                }
                .into());
            }
        }

        let changes = (
            substates::destroyed_at.eq(diesel::dsl::now),
            substates::destroyed_by_transaction.eq(Some(serialize_hex(destroyed_transaction_id))),
            substates::destroyed_by_block.eq(Some(serialize_hex(destroyed_block_id))),
            substates::destroyed_at_epoch.eq(Some(epoch.as_u64() as i64)),
            substates::destroyed_justify.eq(Some(serialize_hex(destroyed_qc_id))),
        );

        diesel::update(substates::table)
            .filter(substates::address.eq_any(addresses))
            .set(changes)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_down",
                source: e,
            })?;

        Ok(())
    }

    fn substates_create(&mut self, substate: SubstateRecord) -> Result<(), StorageError> {
        use crate::schema::substates;

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
            substates::destroyed_by_transaction.eq(substate.destroyed().map(|d| serialize_hex(d.by_transaction))),
            substates::destroyed_justify.eq(substate.destroyed().map(|d| serialize_hex(d.justify))),
            substates::destroyed_by_block.eq(substate.destroyed().map(|d| serialize_hex(d.by_block))),
            substates::destroyed_at_epoch.eq(substate.destroyed().map(|d| d.at_epoch.as_u64() as i64)),
        );

        diesel::insert_into(substates::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_create",
                source: e,
            })?;

        Ok(())
    }

    fn locked_outputs_acquire_all<I, B>(
        &mut self,
        block_id: &BlockId,
        transaction_id: &TransactionId,
        output_addresses: I,
    ) -> Result<SubstateLockState, StorageError>
    where
        I: IntoIterator<Item = B>,
        B: Borrow<SubstateAddress>,
    {
        use crate::schema::locked_outputs;
        let block_id_hex = serialize_hex(block_id);
        let transaction_id_hex = serialize_hex(transaction_id);

        let insert = output_addresses
            .into_iter()
            .map(|address| {
                (
                    locked_outputs::block_id.eq(&block_id_hex),
                    locked_outputs::transaction_id.eq(&transaction_id_hex),
                    locked_outputs::substate_address.eq(serialize_hex(address.borrow())),
                )
            })
            .collect::<Vec<_>>();

        let lock_state = diesel::insert_into(locked_outputs::table)
            .values(insert)
            .execute(self.connection())
            .map(|_| SubstateLockState::LockAcquired)
            .or_else(|e| {
                if let diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::UniqueViolation, _) = e {
                    Ok(SubstateLockState::SomeAlreadyWriteLocked)
                } else {
                    Err(SqliteStorageError::DieselError {
                        operation: "locked_outputs_acquire_all",
                        source: e,
                    })
                }
            })?;

        Ok(lock_state)
    }

    fn locked_outputs_release_all<I, B>(&mut self, output_addresses: I) -> Result<Vec<LockedOutput>, StorageError>
    where
        I: IntoIterator<Item = B>,
        B: Borrow<SubstateAddress>,
    {
        use crate::schema::locked_outputs;

        let output_addresses = output_addresses
            .into_iter()
            .map(|address| serialize_hex(address.borrow()))
            .collect::<Vec<_>>();

        // let locked = locked_outputs::table
        //     .filter(locked_outputs::shard_id.eq_any(&output_shards))
        //     .get_results::<sql_models::LockedOutput>(self.connection())
        //     .map_err(|e| SqliteStorageError::DieselError {
        //         operation: "locked_outputs_release",
        //         source: e,
        //     })?;
        //
        // if locked.len() != output_shards.len() {
        //     return Err(SqliteStorageError::NotAllSubstatesFound {
        //         operation: "locked_outputs_release",
        //         details: format!(
        //             "Found {} locked outputs, but {} were requested",
        //             locked.len(),
        //             output_shards.len()
        //         ),
        //     }
        //     .into());
        // }

        diesel::delete(locked_outputs::table)
            .filter(locked_outputs::substate_address.eq_any(&output_addresses))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "locked_outputs_release",
                source: e,
            })?;

        // locked.into_iter().map(TryInto::try_into).collect()
        Ok(vec![])
    }

    fn pending_state_tree_diffs_remove_by_block(
        &mut self,
        block_id: &BlockId,
    ) -> Result<PendingStateTreeDiff, StorageError> {
        use crate::schema::pending_state_tree_diffs;

        let diff = pending_state_tree_diffs::table
            .filter(pending_state_tree_diffs::block_id.eq(serialize_hex(block_id)))
            .first::<sql_models::PendingStateTreeDiff>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "pending_state_tree_diffs_remove_by_block",
                source: e,
            })?;

        diesel::delete(pending_state_tree_diffs::table)
            .filter(pending_state_tree_diffs::id.eq(diff.id))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "pending_state_tree_diffs_remove_by_block",
                source: e,
            })?;

        diff.try_into()
    }

    fn pending_state_tree_diffs_insert(&mut self, pending_diff: &PendingStateTreeDiff) -> Result<(), StorageError> {
        use crate::schema::pending_state_tree_diffs;

        let insert = (
            pending_state_tree_diffs::block_id.eq(serialize_hex(pending_diff.block_id)),
            pending_state_tree_diffs::block_height.eq(pending_diff.block_height.as_u64() as i64),
            pending_state_tree_diffs::diff_json.eq(serialize_json(&pending_diff.diff)?),
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
}

impl<'a, TAddr> Deref for SqliteStateStoreWriteTransaction<'a, TAddr> {
    type Target = SqliteStateStoreReadTransaction<'a, TAddr>;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref().unwrap()
    }
}

impl<'a, TAddr> DerefMut for SqliteStateStoreWriteTransaction<'a, TAddr> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transaction.as_mut().unwrap()
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
