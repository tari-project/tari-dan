//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Borrow,
    collections::HashSet,
    ops::{Deref, DerefMut},
};

use diesel::{AsChangeset, ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection};
use log::*;
use tari_dan_common_types::{Epoch, NodeAddressable, ShardId};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Decision,
        Evidence,
        HighQc,
        LastExecuted,
        LastProposed,
        LastVoted,
        LeafBlock,
        LockedBlock,
        LockedOutput,
        QuorumCertificate,
        SubstateLockFlag,
        SubstateLockState,
        SubstateRecord,
        TransactionAtom,
        TransactionPoolStage,
        TransactionRecord,
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
    sql_models,
    sqlite_transaction::SqliteTransaction,
};

const LOG_TARGET: &str = "tari::dan::storage";

pub struct SqliteStateStoreWriteTransaction<'a, TAddr> {
    /// None indicates if the transaction has been explicitly committed/rolled back
    transaction: Option<SqliteStateStoreReadTransaction<'a, TAddr>>,
}

impl<'a, TAddr> SqliteStateStoreWriteTransaction<'a, TAddr> {
    pub fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self {
            transaction: Some(SqliteStateStoreReadTransaction::new(transaction)),
        }
    }

    pub fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.as_mut().unwrap().connection()
    }
}

impl<TAddr: NodeAddressable> StateStoreWriteTransaction for SqliteStateStoreWriteTransaction<'_, TAddr> {
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

    fn blocks_insert(&mut self, block: &Block<TAddr>) -> Result<(), StorageError> {
        use crate::schema::blocks;

        let insert = (
            blocks::block_id.eq(serialize_hex(block.id())),
            blocks::parent_block_id.eq(serialize_hex(block.parent())),
            blocks::height.eq(block.height().as_u64() as i64),
            blocks::epoch.eq(block.epoch().as_u64() as i64),
            blocks::proposed_by.eq(serialize_hex(block.proposed_by().as_bytes())),
            blocks::command_count.eq(block.commands().len() as i64),
            blocks::commands.eq(serialize_json(block.commands())?),
            blocks::total_leader_fee.eq(block.total_leader_fee() as i64),
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

    fn insert_missing_transactions<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        block_id: &BlockId,
        transaction_ids: I,
    ) -> Result<(), StorageError> {
        use crate::schema::{block_missing_txs, missing_tx};

        let transaction_ids = transaction_ids.into_iter().map(serialize_hex).collect::<Vec<_>>();
        let block_id_hex = serialize_hex(block_id);
        let insert = (
            block_missing_txs::block_id.eq(&block_id_hex),
            block_missing_txs::transaction_ids.eq(serialize_json(&transaction_ids)?),
        );

        diesel::insert_into(block_missing_txs::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "insert_missing_txs",
                source: e,
            })?;

        let values = transaction_ids
            .iter()
            .map(|tx_id| {
                (
                    missing_tx::block_id.eq(&block_id_hex),
                    missing_tx::transaction_id.eq(tx_id),
                )
            })
            .collect::<Vec<_>>();

        diesel::insert_into(missing_tx::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "insert_missing_txs",
                source: e,
            })?;

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

    fn quorum_certificates_insert(&mut self, qc: &QuorumCertificate<Self::Addr>) -> Result<(), StorageError> {
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
                operation: "leaf_node_set",
                source: e,
            })?;

        Ok(())
    }

    fn locked_block_set(&mut self, locked_block: &LockedBlock) -> Result<(), StorageError> {
        use crate::schema::locked_block;

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
            transaction_pool::transaction_fee.eq(transaction.transaction_fee as i64),
            transaction_pool::leader_fee.eq(transaction.leader_fee as i64),
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

    fn votes_insert(&mut self, vote: &Vote<Self::Addr>) -> Result<(), StorageError> {
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
                operation: "substates_try_lock_all",
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
            return Ok(SubstateLockState::SomeAlreadyWriteLocked);
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
                        operation: "substates_try_lock_all",
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
                operation: "substate_create",
                source: e,
            })?;

        Ok(())
    }

    fn locked_outputs_acquire_all<I, B>(
        &mut self,
        block_id: &BlockId,
        transaction_id: &TransactionId,
        output_shards: I,
    ) -> Result<SubstateLockState, StorageError>
    where
        I: IntoIterator<Item = B>,
        B: Borrow<ShardId>,
    {
        use crate::schema::locked_outputs;
        let block_id_hex = serialize_hex(block_id);
        let transaction_id_hex = serialize_hex(transaction_id);

        let insert = output_shards
            .into_iter()
            .map(|shard_id| {
                (
                    locked_outputs::block_id.eq(&block_id_hex),
                    locked_outputs::transaction_id.eq(&transaction_id_hex),
                    locked_outputs::shard_id.eq(serialize_hex(shard_id.borrow())),
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
                        operation: "locked_outputs_acquire",
                        source: e,
                    })
                }
            })?;

        Ok(lock_state)
    }

    fn locked_outputs_release_all<I, B>(&mut self, output_shards: I) -> Result<Vec<LockedOutput>, StorageError>
    where
        I: IntoIterator<Item = B>,
        B: Borrow<ShardId>,
    {
        use crate::schema::locked_outputs;

        let output_shards = output_shards
            .into_iter()
            .map(|shard_id| serialize_hex(shard_id.borrow()))
            .collect::<Vec<_>>();

        let locked = locked_outputs::table
            .filter(locked_outputs::shard_id.eq_any(&output_shards))
            .get_results::<sql_models::LockedOutput>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "locked_outputs_release",
                source: e,
            })?;

        if locked.len() != output_shards.len() {
            return Err(SqliteStorageError::NotAllSubstatesFound {
                operation: "locked_outputs_release",
                details: format!(
                    "Found {} locked outputs, but {} were requested",
                    locked.len(),
                    output_shards.len()
                ),
            }
            .into());
        }

        diesel::delete(locked_outputs::table)
            .filter(locked_outputs::shard_id.eq_any(&output_shards))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "locked_outputs_release",
                source: e,
            })?;

        locked.into_iter().map(TryInto::try_into).collect()
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
