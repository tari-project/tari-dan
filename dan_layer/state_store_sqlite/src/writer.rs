//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::BTreeSet,
    ops::{Deref, DerefMut},
};

use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection};
use log::*;
use tari_dan_storage::{
    consensus_models::{
        Block,
        ExecutedTransaction,
        HighQc,
        LastExecuted,
        LastVoted,
        LeafBlock,
        LockedBlock,
        TransactionDecision,
    },
    StateStoreWriteTransaction,
    StorageError,
};
use tari_utilities::ByteArray;

use crate::{
    error::SqliteStorageError,
    reader::SqliteStateStoreReadTransaction,
    serialization::{serialize_hex, serialize_json},
    sql_models,
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
            blocks::prepared.eq(serialize_json(block.prepared())?),
            blocks::precommitted.eq(serialize_json(block.precommitted())?),
            blocks::committed.eq(serialize_json(block.committed())?),
            blocks::justify.eq(serialize_json(block.justify())?),
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
            high_qcs::height.eq(high_qc.height.as_u64() as i64),
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

    fn transactions_insert(&mut self, executed_transaction: &ExecutedTransaction) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let ExecutedTransaction { transaction, result } = executed_transaction;

        let insert = (
            transactions::transaction_id.eq(serialize_hex(transaction.hash())),
            transactions::fee_instructions.eq(serialize_json(&transaction.fee_instructions())?),
            transactions::instructions.eq(serialize_json(&transaction.instructions())?),
            transactions::result.eq(serialize_json(&result)?),
            transactions::signature.eq(serialize_json(transaction.signature())?),
            transactions::sender_public_key.eq(serialize_hex(transaction.sender_public_key().as_bytes())),
            transactions::involved_shards.eq(serialize_json(&transaction.involved_shards())?),
            transactions::is_finalized.eq(false),
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

    fn transactions_mark_many_finalized(
        &mut self,
        transaction_ids: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let tx_ids = transaction_ids.iter().map(|id| serialize_hex(id.transaction_id));

        diesel::update(transactions::table)
            .filter(transactions::transaction_id.eq_any(tx_ids))
            .set(transactions::is_finalized.eq(true))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_mark_many_finalized",
                source: e,
            })?;

        Ok(())
    }

    fn new_transaction_pool_insert(&mut self, transaction: TransactionDecision) -> Result<(), StorageError> {
        use crate::schema::new_transaction_pool;

        let insert = (
            new_transaction_pool::transaction_id.eq(serialize_hex(transaction.transaction_id)),
            new_transaction_pool::decision.eq(transaction.decision.to_string()),
            new_transaction_pool::fee.eq(transaction.per_shard_validator_fee as i64),
        );

        diesel::insert_into(new_transaction_pool::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "new_transaction_pool_insert",
                source: e,
            })?;

        Ok(())
    }

    fn new_transaction_pool_remove_many_ready(
        &mut self,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        use crate::schema::new_transaction_pool;

        let rows = new_transaction_pool::table
            .limit(max_txs as i64)
            .load::<sql_models::TransactionDecision>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "new_transaction_pool_remove_many_ready",
                source: e,
            })?;

        let txs = rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, StorageError>>()?;

        Ok(txs)
    }

    fn new_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        use crate::schema::new_transaction_pool;

        let tx_ids = transactions
            .iter()
            .map(|tx| serialize_hex(tx.transaction_id))
            .collect::<Vec<_>>();

        let sql_transactions = new_transaction_pool::table
            .filter(new_transaction_pool::transaction_id.eq_any(&tx_ids))
            .load::<sql_models::TransactionDecision>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "new_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        if sql_transactions.len() != tx_ids.len() {
            return Err(SqliteStorageError::MalformedDbData {
                operation: "new_transaction_pool_remove_specific_ready",
                details: format!(
                    "{} transactions were given to remove but only {} were found",
                    transactions.len(),
                    sql_transactions.len()
                ),
            }
            .into());
        }

        diesel::delete(new_transaction_pool::table)
            .filter(new_transaction_pool::transaction_id.eq_any(&tx_ids))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "new_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        let txs = sql_transactions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(txs)
    }

    fn prepared_transaction_pool_insert_pending(
        &mut self,
        transaction_decisions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        use crate::schema::prepared_transaction_pool;

        let insert = transaction_decisions
            .iter()
            .map(|tx| {
                (
                    prepared_transaction_pool::transaction_id.eq(serialize_hex(tx.transaction_id)),
                    prepared_transaction_pool::decision.eq(tx.decision.to_string()),
                    prepared_transaction_pool::fee.eq(tx.per_shard_validator_fee as i64),
                    prepared_transaction_pool::is_ready.eq(false),
                )
            })
            .collect::<Vec<_>>();

        diesel::insert_into(prepared_transaction_pool::table)
            .values(insert)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "prepared_transaction_pool_insert_pending",
                source: e,
            })?;

        Ok(())
    }

    fn prepared_transaction_pool_mark_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        use crate::schema::prepared_transaction_pool;

        let tx_ids = transactions
            .iter()
            .map(|tx| serialize_hex(tx.transaction_id))
            .collect::<Vec<_>>();

        // Check if all transactions exist in the pool - it does not matter if they are already ready, which is why we
        // cant use rows_affected from the update to check this.
        let num_found = prepared_transaction_pool::table
            .filter(prepared_transaction_pool::transaction_id.eq_any(&tx_ids))
            .count()
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "prepared_transaction_pool_mark_specific_ready",
                source: e,
            })?;

        // If there are duplicates in tx_ids (which there should never be) this will unexpectedly fail
        if num_found as usize != tx_ids.len() {
            return Err(SqliteStorageError::MalformedDbData {
                operation: "prepared_transaction_pool_mark_specific_ready",
                details: format!(
                    "{} transactions were given to mark as ready but only {} were found",
                    transactions.len(),
                    num_found
                ),
            }
            .into());
        }

        diesel::update(prepared_transaction_pool::table)
            .filter(prepared_transaction_pool::transaction_id.eq_any(&tx_ids))
            .set(prepared_transaction_pool::is_ready.eq(true))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "prepared_transaction_pool_mark_many_ready",
                source: e,
            })?;

        Ok(())
    }

    fn prepared_transaction_pool_remove_many_ready(
        &mut self,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        use crate::schema::prepared_transaction_pool;

        let sql_transactions = prepared_transaction_pool::table
            .select((
                prepared_transaction_pool::id,
                prepared_transaction_pool::transaction_id,
                prepared_transaction_pool::decision,
                prepared_transaction_pool::fee,
                prepared_transaction_pool::created_at,
            ))
            .filter(prepared_transaction_pool::is_ready.eq(true))
            .order_by(prepared_transaction_pool::id.asc())
            .limit(max_txs as i64)
            .load::<sql_models::TransactionDecision>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "prepared_transaction_pool_remove_many_ready",
                source: e,
            })?;

        if sql_transactions.is_empty() {
            return Ok(BTreeSet::new());
        }

        let row_affected = diesel::delete(prepared_transaction_pool::table)
            .filter(prepared_transaction_pool::is_ready.eq(true))
            .filter(prepared_transaction_pool::id.le(sql_transactions.last().unwrap().id))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "prepared_transaction_pool_remove_many_ready",
                source: e,
            })?;

        assert_eq!(row_affected, sql_transactions.len());

        let txs = sql_transactions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(txs)
    }

    fn prepared_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        use crate::schema::prepared_transaction_pool;

        let tx_ids = transactions
            .iter()
            .map(|tx| serialize_hex(tx.transaction_id))
            .collect::<Vec<_>>();

        // Check if all transactions exist in the pool - it does not matter if they are already ready, which is why we
        // cant use rows_affected from the update to check this.
        let num_found = prepared_transaction_pool::table
            .filter(prepared_transaction_pool::transaction_id.eq_any(&tx_ids))
            .filter(prepared_transaction_pool::is_ready.eq(true))
            .count()
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "prepared_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        // If there are duplicates in tx_ids (which there should never be) this will unexpectedly fail
        if num_found as usize != tx_ids.len() {
            return Err(SqliteStorageError::MalformedDbData {
                operation: "prepared_transaction_pool_remove_specific_ready",
                details: format!(
                    "{} transactions were given to remove but only {} were found",
                    transactions.len(),
                    num_found
                ),
            }
            .into());
        }

        let sql_transactions = prepared_transaction_pool::table
            .select((
                prepared_transaction_pool::id,
                prepared_transaction_pool::transaction_id,
                prepared_transaction_pool::decision,
                prepared_transaction_pool::fee,
                prepared_transaction_pool::created_at,
            ))
            .filter(prepared_transaction_pool::transaction_id.eq_any(&tx_ids))
            .filter(prepared_transaction_pool::is_ready.eq(true))
            .load::<sql_models::TransactionDecision>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "prepared_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        diesel::delete(prepared_transaction_pool::table)
            .filter(prepared_transaction_pool::is_ready.eq(true))
            .filter(prepared_transaction_pool::transaction_id.eq_any(&tx_ids))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "prepared_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        let txs = sql_transactions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(txs)
    }

    fn precommitted_transaction_pool_insert_pending(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        use crate::schema::precommitted_transaction_pool;

        let values = transactions
            .iter()
            .map(|tx| {
                (
                    precommitted_transaction_pool::transaction_id.eq(serialize_hex(tx.transaction_id)),
                    precommitted_transaction_pool::decision.eq(tx.decision.to_string()),
                    precommitted_transaction_pool::fee.eq(tx.per_shard_validator_fee as i64),
                    precommitted_transaction_pool::is_ready.eq(false),
                )
            })
            .collect::<Vec<_>>();

        diesel::insert_into(precommitted_transaction_pool::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "precommitted_transaction_pool_insert_pending",
                source: e,
            })?;

        Ok(())
    }

    fn precommitted_transaction_pool_mark_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        use crate::schema::precommitted_transaction_pool;

        let tx_ids = transactions
            .iter()
            .map(|tx| serialize_hex(tx.transaction_id))
            .collect::<Vec<_>>();

        let row_affected = diesel::update(precommitted_transaction_pool::table)
            .filter(precommitted_transaction_pool::transaction_id.eq_any(&tx_ids))
            .set(precommitted_transaction_pool::is_ready.eq(true))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "precommitted_transaction_pool_insert_pending",
                source: e,
            })?;

        if row_affected != tx_ids.len() {
            return Err(SqliteStorageError::MalformedDbData {
                operation: "precommitted_transaction_pool_mark_specific_ready",
                details: format!(
                    "{} transactions were given to mark as ready but only {} were found",
                    transactions.len(),
                    row_affected
                ),
            }
            .into());
        }

        Ok(())
    }

    fn precommitted_transaction_pool_remove_many_ready(
        &mut self,
        max_txs: usize,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        use crate::schema::precommitted_transaction_pool;

        let sql_transactions = precommitted_transaction_pool::table
            .select((
                precommitted_transaction_pool::id,
                precommitted_transaction_pool::transaction_id,
                precommitted_transaction_pool::decision,
                precommitted_transaction_pool::fee,
                precommitted_transaction_pool::created_at,
            ))
            .filter(precommitted_transaction_pool::is_ready.eq(true))
            .order(precommitted_transaction_pool::created_at.asc())
            .limit(max_txs as i64)
            .load::<sql_models::TransactionDecision>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "precommitted_transaction_pool_remove_many_ready",
                source: e,
            })?;

        let tx_ids = sql_transactions
            .iter()
            .map(|tx| serialize_hex(&tx.transaction_id))
            .collect::<Vec<_>>();

        diesel::delete(precommitted_transaction_pool::table)
            .filter(precommitted_transaction_pool::transaction_id.eq_any(&tx_ids))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "precommitted_transaction_pool_remove_many_ready",
                source: e,
            })?;

        let txs = sql_transactions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(txs)
    }

    fn precommitted_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        use crate::schema::precommitted_transaction_pool;

        let tx_ids = transactions
            .iter()
            .map(|tx| serialize_hex(tx.transaction_id))
            .collect::<Vec<_>>();

        let sql_transactions = precommitted_transaction_pool::table
            .select((
                precommitted_transaction_pool::id,
                precommitted_transaction_pool::transaction_id,
                precommitted_transaction_pool::decision,
                precommitted_transaction_pool::fee,
                precommitted_transaction_pool::created_at,
            ))
            .filter(precommitted_transaction_pool::transaction_id.eq_any(&tx_ids))
            .filter(precommitted_transaction_pool::is_ready.eq(true))
            .load::<sql_models::TransactionDecision>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "precommitted_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        diesel::delete(precommitted_transaction_pool::table)
            .filter(precommitted_transaction_pool::transaction_id.eq_any(&tx_ids))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "precommitted_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        let txs = sql_transactions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(txs)
    }

    fn committed_transaction_pool_insert_pending(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        use crate::schema::committed_transaction_pool;

        let values = transactions
            .iter()
            .map(|tx| {
                (
                    committed_transaction_pool::transaction_id.eq(serialize_hex(tx.transaction_id)),
                    committed_transaction_pool::decision.eq(tx.decision.to_string()),
                    committed_transaction_pool::fee.eq(tx.per_shard_validator_fee as i64),
                    committed_transaction_pool::is_ready.eq(false),
                )
            })
            .collect::<Vec<_>>();

        diesel::insert_into(committed_transaction_pool::table)
            .values(values)
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "precommitted_transaction_pool_insert_pending",
                source: e,
            })?;

        Ok(())
    }

    fn committed_transaction_pool_mark_many_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<(), StorageError> {
        use crate::schema::committed_transaction_pool;

        let tx_ids = transactions
            .iter()
            .map(|tx| serialize_hex(tx.transaction_id))
            .collect::<Vec<_>>();

        let row_affected = diesel::update(committed_transaction_pool::table)
            .filter(committed_transaction_pool::transaction_id.eq_any(&tx_ids))
            .set(committed_transaction_pool::is_ready.eq(true))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "committed_transaction_pool_mark_many_ready",
                source: e,
            })?;

        if row_affected != tx_ids.len() {
            return Err(SqliteStorageError::MalformedDbData {
                operation: "committed_transaction_pool_mark_many_ready",
                details: format!(
                    "{} transactions were given to mark as ready but only {} were found",
                    transactions.len(),
                    row_affected
                ),
            }
            .into());
        }

        Ok(())
    }

    fn committed_transaction_pool_remove_specific_ready(
        &mut self,
        transactions: &BTreeSet<TransactionDecision>,
    ) -> Result<BTreeSet<TransactionDecision>, StorageError> {
        use crate::schema::committed_transaction_pool;

        let tx_ids = transactions
            .iter()
            .map(|tx| serialize_hex(tx.transaction_id))
            .collect::<Vec<_>>();

        let sql_transactions = committed_transaction_pool::table
            .select((
                committed_transaction_pool::id,
                committed_transaction_pool::transaction_id,
                committed_transaction_pool::decision,
                committed_transaction_pool::fee,
                committed_transaction_pool::created_at,
            ))
            .filter(committed_transaction_pool::transaction_id.eq_any(&tx_ids))
            .filter(committed_transaction_pool::is_ready.eq(true))
            .load::<sql_models::TransactionDecision>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "committed_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        diesel::delete(committed_transaction_pool::table)
            .filter(committed_transaction_pool::transaction_id.eq_any(&tx_ids))
            .filter(committed_transaction_pool::is_ready.eq(true))
            .execute(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "committed_transaction_pool_remove_specific_ready",
                source: e,
            })?;

        let txs = sql_transactions
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        Ok(txs)
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
