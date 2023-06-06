//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{Epoch, NodeHeight, ShardId};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, HighQc, LeafBlock, QuorumCertificate, Transaction, TransactionId},
    StateStoreReadTransaction,
    StorageError,
};

use crate::{
    deser::{deserialize_json, serialize_hex},
    error::SqliteStorageError,
    sql_models,
    sqlite_transaction::SqliteTransaction,
};

pub struct SqliteStateStoreReadTransaction<'a> {
    transaction: SqliteTransaction<'a>,
}

impl<'a> SqliteStateStoreReadTransaction<'a> {
    pub(crate) fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self { transaction }
    }

    pub(crate) fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.connection()
    }

    pub(crate) fn commit(self) -> Result<(), SqliteStorageError> {
        self.transaction.commit()
    }

    pub(crate) fn rollback(self) -> Result<(), SqliteStorageError> {
        self.transaction.rollback()
    }
}

impl StateStoreReadTransaction for SqliteStateStoreReadTransaction<'_> {
    fn last_vote_height_get(&mut self, _epoch: Epoch) -> Result<u64, StorageError> {
        todo!()
    }

    fn locked_block_get(&mut self, _epoch: Epoch) -> Result<(NodeHeight, FixedHash), StorageError> {
        todo!()
    }

    fn leaf_block_get(&mut self, epoch: Epoch) -> Result<LeafBlock, StorageError> {
        use crate::schema::leaf_blocks;

        let leaf_block = leaf_blocks::table
            .filter(leaf_blocks::epoch.eq(epoch.as_u64() as i64))
            .order_by(leaf_blocks::block_height.desc())
            .first::<sql_models::LeafBlock>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "leaf_block_get",
                source: e,
            })?;

        leaf_block.try_into()
    }

    fn high_qc_get(&mut self, epoch: Epoch) -> Result<HighQc, StorageError> {
        use crate::schema::high_qcs;

        let high_qc = high_qcs::table
            .filter(high_qcs::epoch.eq(epoch.as_u64() as i64))
            .order_by(high_qcs::height.desc())
            .first::<sql_models::HighQc>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "high_qc_get",
                source: e,
            })?;

        high_qc.try_into()
    }

    fn transactions_get(&mut self, tx_id: &TransactionId) -> Result<Transaction, StorageError> {
        use crate::schema::transactions;

        let transaction = transactions::table
            .filter(transactions::transaction_id.eq(serialize_hex(tx_id)))
            .first::<sql_models::Transaction>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_get",
                source: e,
            })?;

        transaction.try_into()
    }

    fn blocks_get(&mut self, block_id: &BlockId) -> Result<Block, StorageError> {
        use crate::schema::blocks;

        let block = blocks::table
            .filter(blocks::block_id.eq(serialize_hex(block_id)))
            .first::<sql_models::Block>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get",
                source: e,
            })?;

        block.try_into()
    }

    fn quorum_certificates_get(&mut self, block_id: &BlockId) -> Result<QuorumCertificate, StorageError> {
        use crate::schema::blocks;
        // TODO: keep QCs in a separate table?

        let qc_json = blocks::table
            .select(blocks::justify)
            .filter(blocks::block_id.eq(serialize_hex(block_id)))
            .first::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get",
                source: e,
            })?;

        deserialize_json(&qc_json)
    }

    fn transaction_pools_ready_transaction_count(&mut self) -> Result<usize, StorageError> {
        use crate::schema::{
            committed_transaction_pool,
            new_transaction_pool,
            precommitted_transaction_pool,
            prepared_transaction_pool,
        };

        let new_count = new_transaction_pool::table
            .count()
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pools_ready_transaction_count",
                source: e,
            })? as usize;

        let prepared_count = prepared_transaction_pool::table
            .filter(prepared_transaction_pool::is_ready.eq(true))
            .count()
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pools_ready_transaction_count",
                source: e,
            })? as usize;

        let precommitted_count = precommitted_transaction_pool::table
            .filter(precommitted_transaction_pool::is_ready.eq(true))
            .count()
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pools_ready_transaction_count",
                source: e,
            })? as usize;

        let committed_count = committed_transaction_pool::table
            .filter(committed_transaction_pool::is_ready.eq(true))
            .count()
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pools_ready_transaction_count",
                source: e,
            })? as usize;

        Ok(new_count + prepared_count + precommitted_count + committed_count)
    }

    fn transaction_pools_fetch_involved_shards(
        &mut self,
        transaction_ids: HashSet<TransactionId>,
    ) -> Result<HashSet<ShardId>, StorageError> {
        use crate::schema::transactions;

        let tx_ids = transaction_ids.into_iter().map(serialize_hex).collect::<Vec<_>>();

        let involved_shards = transactions::table
            .select(transactions::involved_shards)
            .filter(transactions::transaction_id.eq_any(tx_ids))
            .load::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pools_fetch_involved_shards",
                source: e,
            })?;

        let shards = involved_shards
            .into_iter()
            .flat_map(
                |s| deserialize_json::<HashSet<ShardId>>(&s).unwrap(), // a Result is very inconvenient with flat_map
            )
            .collect();

        Ok(shards)
    }
}
