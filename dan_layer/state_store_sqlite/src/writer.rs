//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashSet,
    ops::{Deref, DerefMut},
};

use diesel::{AsChangeset, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection};
use log::*;
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Decision,
        Evidence,
        ExecutedTransaction,
        HighQc,
        LastExecuted,
        LastProposed,
        LastVoted,
        LeafBlock,
        LockedBlock,
        QuorumCertificate,
        SubstateLockFlag,
        SubstateLockState,
        SubstateRecord,
        TransactionAtom,
        TransactionPoolStage,
        Vote,
    },
    StateStoreWriteTransaction,
    StorageError,
};
use tari_transaction::{Transaction, TransactionId};
use time::PrimitiveDateTime;

use crate::{
    error::SqliteStorageError,
    reader::SqliteStateStoreReadTransaction,
    serialization::{deserialize_hex_try_from, deserialize_json, serialize_hex, serialize_json},
    sqlite_transaction::SqliteTransaction,
};

const LOG_TARGET: &str = "tari::dan::storage";

pub struct SqliteStateStoreWriteTransaction<'a> {
    /// None indicates if the transaction has been explicitly committed/rolled back
    transaction: Option<SqliteStateStoreReadTransaction<'a>>,
}

impl<'a> SqliteStateStoreWriteTransaction<'a> {
    pub fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self {
            transaction: Some(SqliteStateStoreReadTransaction::new(transaction)),
        }
    }

    pub fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.as_mut().unwrap().connection()
    }
}

impl StateStoreWriteTransaction for SqliteStateStoreWriteTransaction<'_> {
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
            blocks::height.eq(block.height().as_u64() as i64),
            blocks::epoch.eq(block.epoch().as_u64() as i64),
            blocks::leader_round.eq(block.round() as i64),
            blocks::proposed_by.eq(serialize_hex(block.proposed_by())),
            blocks::commands.eq(serialize_json(block.commands())?),
            blocks::qc_id.eq(serialize_hex(block.justify().id())),
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

    fn insert_missing_transactions(
        &mut self,
        block_id: &BlockId,
        transaction_ids: Vec<TransactionId>,
    ) -> Result<(), StorageError> {
        use crate::schema::{block_missing_txs, missing_tx};

        let insert = (
            block_missing_txs::block_id.eq(serialize_hex(block_id)),
            block_missing_txs::transaction_ids.eq(serialize_json(&transaction_ids)?),
        );

        diesel::insert_into(block_missing_txs::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "insert_missing_txs",
                source: e,
            })?;

        for transaction_id in transaction_ids {
            diesel::insert_into(missing_tx::table)
                .values((
                    missing_tx::block_id.eq(serialize_hex(block_id)),
                    missing_tx::transaction_id.eq(serialize_hex(transaction_id)),
                ))
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "insert_missing_txs",
                    source: e,
                })?;
        }
        Ok(())
    }

    fn remove_missing_transaction(&mut self, transaction_id: TransactionId) -> Result<Option<BlockId>, StorageError> {
        use crate::schema::{block_missing_txs, missing_tx};
        let block_id = missing_tx::table
            .select(missing_tx::block_id)
            .filter(missing_tx::transaction_id.eq(serialize_hex(transaction_id)))
            .first::<String>(self.connection())
            .optional()
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "remove_missing_transaction",
                source: e,
            })?;
        if let Some(block_id) = block_id {
            diesel::delete(missing_tx::table)
                .filter(missing_tx::transaction_id.eq(serialize_hex(transaction_id)))
                .execute(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "remove_missing_transaction",
                    source: e,
                })?;
            let missing_transactions = block_missing_txs::table
                .select(block_missing_txs::transaction_ids)
                .filter(block_missing_txs::block_id.eq(block_id.clone()))
                .first::<String>(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "remove_missing_transaction",
                    source: e,
                })?;

            let mut missing_transactions = deserialize_json::<Vec<TransactionId>>(&missing_transactions)?;

            missing_transactions.retain(|&transaction| transaction != transaction_id);

            if missing_transactions.is_empty() {
                diesel::delete(block_missing_txs::table)
                    .filter(block_missing_txs::block_id.eq(block_id.clone()))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "remove_missing_transaction",
                        source: e,
                    })?;
                Ok(Some(deserialize_hex_try_from(&block_id)?))
            } else {
                diesel::update(block_missing_txs::table)
                    .filter(block_missing_txs::block_id.eq(block_id))
                    .set(block_missing_txs::transaction_ids.eq(serialize_json(&missing_transactions)?))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "remove_missing_transaction",
                        source: e,
                    })?;
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    fn quorum_certificates_insert(&mut self, qc: &QuorumCertificate) -> Result<(), StorageError> {
        use crate::schema::quorum_certificates;

        let insert = (
            quorum_certificates::qc_id.eq(serialize_hex(qc.id())),
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

    fn last_voted_set(&mut self, last_voted: &LastVoted) -> Result<(), StorageError> {
        use crate::schema::last_voted;

        let insert = (
            last_voted::epoch.eq(last_voted.epoch.as_u64() as i64),
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

    fn last_executed_set(&mut self, last_exec: &LastExecuted) -> Result<(), StorageError> {
        use crate::schema::last_executed;

        let insert = (
            last_executed::epoch.eq(last_exec.epoch.as_u64() as i64),
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
            last_proposed::epoch.eq(last_proposed.epoch.as_u64() as i64),
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

    fn leaf_block_set(&mut self, leaf_node: &LeafBlock) -> Result<(), StorageError> {
        use crate::schema::leaf_blocks;

        let insert = (
            leaf_blocks::epoch.eq(leaf_node.epoch.as_u64() as i64),
            leaf_blocks::block_id.eq(serialize_hex(leaf_node.block_id)),
            leaf_blocks::block_height.eq(leaf_node.height.as_u64() as i64),
        );

        diesel::insert_into(leaf_blocks::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "leaf_node_set",
                source: e,
            })?;

        Ok(())
    }

    fn locked_block_set(&mut self, locked_block: &LockedBlock) -> Result<(), StorageError> {
        use crate::schema::locked_block;

        let insert = (
            locked_block::epoch.eq(locked_block.epoch.as_u64() as i64),
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
            high_qcs::epoch.eq(high_qc.epoch.as_u64() as i64),
            high_qcs::block_id.eq(serialize_hex(high_qc.block_id)),
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

    fn transactions_insert(&mut self, transaction: &Transaction) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let insert = (
            transactions::transaction_id.eq(serialize_hex(transaction.id())),
            transactions::fee_instructions.eq(serialize_json(transaction.fee_instructions())?),
            transactions::instructions.eq(serialize_json(transaction.instructions())?),
            transactions::signature.eq(serialize_json(transaction.signature())?),
            transactions::inputs.eq(serialize_json(transaction.inputs())?),
            transactions::input_refs.eq(serialize_json(transaction.input_refs())?),
            transactions::outputs.eq(serialize_json(transaction.outputs())?),
            transactions::filled_inputs.eq(serialize_json(transaction.filled_inputs())?),
            transactions::filled_outputs.eq(serialize_json(transaction.filled_outputs())?),
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

    fn executed_transactions_update(&mut self, executed_transaction: &ExecutedTransaction) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let transaction = executed_transaction.transaction();
        let result = executed_transaction.result();

        let update = (
            transactions::result.eq(serialize_json(result)?),
            transactions::filled_inputs.eq(serialize_json(transaction.filled_inputs())?),
            transactions::filled_outputs.eq(serialize_json(transaction.filled_outputs())?),
            transactions::execution_time_ms
                .eq(i64::try_from(executed_transaction.execution_time().as_millis()).unwrap_or(i64::MAX)),
            transactions::final_decision.eq(executed_transaction.final_decision().map(|d| d.to_string())),
        );

        let num_affected = diesel::update(transactions::table)
            .filter(transactions::transaction_id.eq(serialize_hex(transaction.id())))
            .set(update)
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

    fn transaction_pool_insert(
        &mut self,
        transaction: TransactionAtom,
        stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), StorageError> {
        use crate::schema::transaction_pool;

        let insert = (
            transaction_pool::transaction_id.eq(serialize_hex(transaction.id)),
            transaction_pool::involved_shards.eq(serialize_json(
                &transaction.evidence.shards_iter().copied().collect::<Vec<_>>(),
            )?),
            transaction_pool::original_decision.eq(transaction.decision.to_string()),
            transaction_pool::fee.eq(transaction.fee as i64),
            transaction_pool::evidence.eq(serialize_json(&transaction.evidence)?),
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

    fn transaction_pool_update(
        &mut self,
        transaction_id: &TransactionId,
        evidence: Option<&Evidence>,
        stage: Option<TransactionPoolStage>,
        pending_decision: Option<Decision>,
        is_ready: Option<bool>,
    ) -> Result<(), StorageError> {
        use crate::schema::transaction_pool;

        #[derive(AsChangeset)]
        #[diesel(table_name=transaction_pool)]
        struct Changes {
            evidence: Option<String>,
            stage: Option<String>,
            pending_decision: Option<String>,
            is_ready: Option<bool>,
            updated_at: PrimitiveDateTime,
        }

        let now = time::OffsetDateTime::now_utc();

        let change_set = Changes {
            evidence: evidence.map(serialize_json).transpose()?,
            stage: stage.map(|s| s.to_string()),
            pending_decision: pending_decision.map(|d| d.to_string()),
            is_ready,
            updated_at: PrimitiveDateTime::new(now.date(), now.time()),
        };

        diesel::update(transaction_pool::table)
            .filter(transaction_pool::transaction_id.eq(serialize_hex(transaction_id)))
            .set(change_set)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_update",
                source: e,
            })?;

        Ok(())
    }

    fn transaction_pool_remove(&mut self, transaction_id: &TransactionId) -> Result<(), StorageError> {
        use crate::schema::transaction_pool;

        diesel::delete(transaction_pool::table)
            .filter(transaction_pool::transaction_id.eq(serialize_hex(transaction_id)))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_remove",
                source: e,
            })?;

        Ok(())
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
            votes::merkle_proof.eq(serialize_json(&vote.merkle_proof)?),
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

    fn substates_try_lock_many<'a, I: IntoIterator<Item = &'a ShardId>>(
        &mut self,
        locked_by_tx: &TransactionId,
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<SubstateLockState, StorageError> {
        use crate::schema::substates;

        // Lock unique shards
        let objects: HashSet<String> = objects.into_iter().map(serialize_hex).collect();

        let locked_w = substates::table
            .select(substates::is_locked_w)
            .filter(substates::shard_id.eq_any(&objects))
            .get_results::<bool>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_try_lock_many",
                source: e,
            })?;
        if locked_w.len() < objects.len() {
            return Err(SqliteStorageError::NotAllSubstatesFound {
                operation: "substates_try_lock_many",
                details: format!(
                    "{:?}: Found {} substates, but {} were requested",
                    lock_flag,
                    locked_w.len(),
                    objects.len()
                ),
            }
            .into());
        }
        if locked_w.iter().any(|w| *w) {
            return Ok(SubstateLockState::SomeWriteLocked);
        }

        match lock_flag {
            SubstateLockFlag::Write => {
                diesel::update(substates::table)
                    .filter(substates::shard_id.eq_any(objects))
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
                    .filter(substates::shard_id.eq_any(objects))
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

    fn substates_try_unlock_many<'a, I: IntoIterator<Item = &'a ShardId>>(
        &mut self,
        locked_by_tx: &TransactionId,
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<(), StorageError> {
        use crate::schema::substates;

        let objects: HashSet<String> = objects.into_iter().map(serialize_hex).collect();

        match lock_flag {
            SubstateLockFlag::Write => {
                let locked_w = substates::table
                    .select(substates::is_locked_w)
                    // Only the locking transaction can unlock the substates for write
                    .filter(substates::locked_by.eq(serialize_hex(locked_by_tx)))
                    .filter(substates::shard_id.eq_any(&objects))
                    .get_results::<bool>(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "substates_try_unlock_many",
                        source: e,
                    })?;
                if locked_w.len() < objects.len() {
                    return Err(SqliteStorageError::NotAllSubstatesFound {
                        operation: "substates_try_unlock_many",
                        details: format!(
                            "{:?}: Found {} substates, but {} were requested",
                            lock_flag,
                            locked_w.len(),
                            objects.len()
                        ),
                    }
                    .into());
                }
                if locked_w.iter().any(|w| !*w) {
                    return Err(SqliteStorageError::SubstatesUnlock {
                        operation: "substates_try_unlock_many",
                        details: "Not all substates are write locked".to_string(),
                    }
                    .into());
                }

                diesel::update(substates::table)
                    .filter(substates::shard_id.eq_any(objects))
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
                let locked_r = substates::table
                    .select(substates::read_locks)
                    .filter(substates::shard_id.eq_any(&objects))
                    .get_results::<i32>(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "substates_try_unlock_many",
                        source: e,
                    })?;
                if locked_r.len() < objects.len() {
                    return Err(SqliteStorageError::NotAllSubstatesFound {
                        operation: "substates_try_lock_many",
                        details: format!(
                            "Found {} substates, but {} were requested",
                            locked_r.len(),
                            objects.len()
                        ),
                    }
                    .into());
                }
                if locked_r.iter().any(|r| *r == 0) {
                    return Err(SqliteStorageError::SubstatesUnlock {
                        operation: "substates_try_unlock_many",
                        details: "Not all substates are read locked".to_string(),
                    }
                    .into());
                }

                diesel::update(substates::table)
                    .filter(substates::shard_id.eq_any(objects))
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

    fn substate_down_many<I: IntoIterator<Item = ShardId>>(
        &mut self,
        shard_ids: I,
        epoch: Epoch,
        destroyed_block_id: &BlockId,
        destroyed_transaction_id: &TransactionId,
    ) -> Result<(), StorageError> {
        use crate::schema::substates;

        let shard_ids = shard_ids.into_iter().map(serialize_hex).collect::<Vec<_>>();

        let is_writable = substates::table
            .select((substates::address, substates::is_locked_w))
            .filter(substates::shard_id.eq_any(&shard_ids))
            .get_results::<(String, bool)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_down",
                source: e,
            })?;
        if is_writable.len() != shard_ids.len() {
            return Err(SqliteStorageError::NotAllSubstatesFound {
                operation: "substate_down",
                details: format!(
                    "Found {} substates, but {} were requested",
                    is_writable.len(),
                    shard_ids.len()
                ),
            }
            .into());
        }
        if let Some((addr, _)) = is_writable.iter().find(|(_, w)| !*w) {
            return Err(SqliteStorageError::SubstatesUnlock {
                operation: "substate_down",
                details: format!("Substate {} is not write locked", addr),
            }
            .into());
        }

        let changes = (
            substates::destroyed_at.eq(diesel::dsl::now),
            substates::destroyed_by_transaction.eq(Some(serialize_hex(destroyed_transaction_id))),
            substates::destroyed_by_block.eq(Some(serialize_hex(destroyed_block_id))),
            substates::destroyed_at_epoch.eq(Some(epoch.as_u64() as i64)),
        );

        diesel::update(substates::table)
            .filter(substates::shard_id.eq_any(shard_ids))
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
            substates::shard_id.eq(serialize_hex(substate.to_shard_id())),
            substates::address.eq(substate.address.to_string()),
            substates::version.eq(substate.version as i32),
            substates::data.eq(serialize_json(&substate.substate_value)?),
            substates::state_hash.eq(serialize_hex(substate.state_hash)),
            substates::created_by_transaction.eq(serialize_hex(substate.created_by_transaction)),
            substates::created_justify.eq(serialize_hex(substate.created_justify)),
            substates::created_block.eq(serialize_hex(substate.created_block)),
            substates::created_height.eq(substate.created_height.as_u64() as i64),
            substates::destroyed_by_transaction.eq(substate.destroyed_by_transaction.as_ref().map(serialize_hex)),
            substates::destroyed_justify.eq(substate.destroyed_justify.as_ref().map(serialize_hex)),
            substates::destroyed_by_block.eq(substate.destroyed_by_block.as_ref().map(serialize_hex)),
            substates::created_at_epoch.eq(substate.created_at_epoch.as_u64() as i64),
            substates::destroyed_at_epoch.eq(substate.destroyed_at_epoch.map(|e| e.as_u64() as i64)),
        );

        diesel::insert_into(substates::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_up",
                source: e,
            })?;

        Ok(())
    }
}

impl<'a> Deref for SqliteStateStoreWriteTransaction<'a> {
    type Target = SqliteStateStoreReadTransaction<'a>;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref().unwrap()
    }
}

impl<'a> DerefMut for SqliteStateStoreWriteTransaction<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transaction.as_mut().unwrap()
    }
}

impl Drop for SqliteStateStoreWriteTransaction<'_> {
    fn drop(&mut self) {
        if self.transaction.is_some() {
            warn!(
                target: LOG_TARGET,
                "Shard store write transaction was not committed/rolled back"
            );
        }
    }
}
