//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use diesel::{
    sql_query,
    sql_types::Text,
    ExpressionMethods,
    JoinOnDsl,
    NullableExpressionMethods,
    QueryDsl,
    QueryableByName,
    RunQueryDsl,
    SqliteConnection,
};
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        ExecutedTransaction,
        HighQc,
        LastExecuted,
        LastVoted,
        LeafBlock,
        LockedBlock,
        QcId,
        QuorumCertificate,
        TransactionId,
        TransactionPoolRecord,
        TransactionPoolStage,
        Vote,
    },
    StateStoreReadTransaction,
    StorageError,
};

use crate::{
    error::SqliteStorageError,
    serialization::{deserialize_json, serialize_hex},
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
    fn last_voted_get(&mut self, epoch: Epoch) -> Result<LastVoted, StorageError> {
        use crate::schema::last_voted;

        let last_voted = last_voted::table
            .filter(last_voted::epoch.eq(epoch.as_u64() as i64))
            .order_by(last_voted::id.desc())
            .first::<sql_models::LastVoted>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "high_qc_get",
                source: e,
            })?;

        last_voted.try_into()
    }

    fn last_executed_get(&mut self, epoch: Epoch) -> Result<LastExecuted, StorageError> {
        use crate::schema::last_executed;

        let last_executed = last_executed::table
            .filter(last_executed::epoch.eq(epoch.as_u64() as i64))
            .order_by(last_executed::id.desc())
            .first::<sql_models::LastExecuted>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_executed_get",
                source: e,
            })?;

        last_executed.try_into()
    }

    fn locked_block_get(&mut self, epoch: Epoch) -> Result<LockedBlock, StorageError> {
        use crate::schema::locked_block;

        let locked_block = locked_block::table
            .filter(locked_block::epoch.eq(epoch.as_u64() as i64))
            .order_by(locked_block::id.desc())
            .first::<sql_models::LockedBlock>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "locked_block_get",
                source: e,
            })?;

        locked_block.try_into()
    }

    fn leaf_block_get(&mut self, epoch: Epoch) -> Result<LeafBlock, StorageError> {
        use crate::schema::leaf_blocks;

        let leaf_block = leaf_blocks::table
            .filter(leaf_blocks::epoch.eq(epoch.as_u64() as i64))
            .order_by(leaf_blocks::id.desc())
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
            .order_by(high_qcs::id.desc())
            .first::<sql_models::HighQc>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "high_qc_get",
                source: e,
            })?;

        high_qc.try_into()
    }

    fn transactions_get(&mut self, tx_id: &TransactionId) -> Result<ExecutedTransaction, StorageError> {
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

    fn transactions_get_many<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &mut self,
        tx_ids: I,
    ) -> Result<Vec<ExecutedTransaction>, StorageError> {
        use crate::schema::transactions;

        let tx_ids: Vec<String> = tx_ids.into_iter().map(serialize_hex).collect();

        let transactions = transactions::table
            .filter(transactions::transaction_id.eq_any(tx_ids))
            .load::<sql_models::Transaction>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_get_many",
                source: e,
            })?;

        transactions
            .into_iter()
            .map(|transaction| transaction.try_into())
            .collect()
    }

    fn blocks_get(&mut self, block_id: &BlockId) -> Result<Block, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let (block, qc) = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::block_id.eq(serialize_hex(block_id)))
            .first::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get",
                source: e,
            })?;

        let qc = qc.ok_or_else(|| SqliteStorageError::DbInconsistency {
            operation: "blocks_get",
            details: format!(
                "block {} references non-existent quorum certificate {}",
                block_id, block.qc_id
            ),
        })?;

        block.try_convert(qc)
    }

    fn blocks_get_tip(&mut self, epoch: Epoch) -> Result<Block, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let (block, qc) = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::epoch.eq(epoch.as_u64() as i64))
            .order_by(blocks::height.desc())
            .first::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get_tip",
                source: e,
            })?;

        let qc = qc.ok_or_else(|| SqliteStorageError::DbInconsistency {
            operation: "blocks_get_tip",
            details: format!(
                "block {} references non-existent quorum certificate {}",
                block.block_id, block.qc_id
            ),
        })?;

        block.try_convert(qc)
    }

    fn blocks_get_by_parent(&mut self, parent_id: &BlockId) -> Result<Block, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let (block, qc) = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::parent_block_id.eq(serialize_hex(parent_id)))
            .first::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get_by_parent",
                source: e,
            })?;

        let qc = qc.ok_or_else(|| SqliteStorageError::DbInconsistency {
            operation: "blocks_get_by_parent",
            details: format!(
                "block {} references non-existent quorum certificate {}",
                parent_id, block.qc_id
            ),
        })?;

        block.try_convert(qc)
    }

    fn blocks_exists(&mut self, block_id: &BlockId) -> Result<bool, StorageError> {
        use crate::schema::blocks;

        let exists = blocks::table
            .filter(blocks::block_id.eq(serialize_hex(block_id)))
            .count()
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_exists",
                source: e,
            })? >
            0;

        Ok(exists)
    }

    fn blocks_is_ancestor(&mut self, descendant: &BlockId, ancestor: &BlockId) -> Result<bool, StorageError> {
        // TODO: this scans all the way to genesis for every query - can optimise though it's low priority for now
        let is_ancestor = sql_query(
            r#"
            WITH RECURSIVE tree(bid, parent) AS (
                  SELECT block_id, parent_block_id FROM blocks where block_id = ?
                UNION ALL
                  SELECT block_id, parent_block_id
                    FROM blocks JOIN tree ON block_id = tree.parent AND block_id != ?
            )
            SELECT count(1) as "count" FROM tree WHERE bid = ? LIMIT 1
        "#,
        )
        .bind::<Text, _>(serialize_hex(descendant))
        .bind::<Text, _>(serialize_hex(BlockId::genesis())) // stop recursing at zero block
        .bind::<Text, _>(serialize_hex(ancestor))
        .get_result::<Count>(self.connection())
        .map_err(|e| SqliteStorageError::DieselError {
            operation: "blocks_is_ancestor",
            source: e,
        })?;

        Ok(is_ancestor.count > 0)
    }

    fn quorum_certificates_get(&mut self, qc_id: &QcId) -> Result<QuorumCertificate, StorageError> {
        use crate::schema::quorum_certificates;

        let qc_json = quorum_certificates::table
            .select(quorum_certificates::json)
            .filter(quorum_certificates::qc_id.eq(serialize_hex(qc_id)))
            .first::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get",
                source: e,
            })?;

        deserialize_json(&qc_json)
    }

    fn transactions_fetch_involved_shards(
        &mut self,
        transaction_ids: HashSet<TransactionId>,
    ) -> Result<HashSet<ShardId>, StorageError> {
        use crate::schema::transactions;

        let tx_ids = transaction_ids.into_iter().map(serialize_hex).collect::<Vec<_>>();

        let involved_shards = transactions::table
            .select((transactions::inputs, transactions::input_refs, transactions::outputs))
            .filter(transactions::transaction_id.eq_any(tx_ids))
            .load::<(String, String, String)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pools_fetch_involved_shards",
                source: e,
            })?;

        let shards = involved_shards
            .into_iter()
            .map(|(inputs, input_refs, outputs)| {
                (
                    // a Result is very inconvenient with flat_map
                    deserialize_json::<HashSet<ShardId>>(&inputs).unwrap(),
                    deserialize_json::<HashSet<ShardId>>(&input_refs).unwrap(),
                    deserialize_json::<HashSet<ShardId>>(&outputs).unwrap(),
                )
            })
            .flat_map(|(inputs, input_refs, outputs)| {
                inputs
                    .into_iter()
                    .chain(input_refs)
                    .chain(outputs)
                    .collect::<HashSet<_>>()
            })
            .collect();

        Ok(shards)
    }

    fn votes_count_for_block(&mut self, block_id: &BlockId) -> Result<u64, StorageError> {
        use crate::schema::votes;

        let count = votes::table
            .filter(votes::block_id.eq(serialize_hex(block_id)))
            .count()
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "votes_count_for_block",
                source: e,
            })?;

        Ok(count as u64)
    }

    fn votes_get_for_block(&mut self, block_id: &BlockId) -> Result<Vec<Vote>, StorageError> {
        use crate::schema::votes;

        let votes = votes::table
            .filter(votes::block_id.eq(serialize_hex(block_id)))
            .get_results::<sql_models::Vote>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "votes_get_for_block",
                source: e,
            })?;

        votes.into_iter().map(Vote::try_from).collect()
    }

    fn votes_get_by_block_and_sender(
        &mut self,
        block_id: &BlockId,
        sender_leaf_hash: &FixedHash,
    ) -> Result<Vote, StorageError> {
        use crate::schema::votes;

        let vote = votes::table
            .filter(votes::block_id.eq(serialize_hex(block_id)))
            .filter(votes::sender_leaf_hash.eq(serialize_hex(sender_leaf_hash)))
            .first::<sql_models::Vote>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "votes_get",
                source: e,
            })?;

        Vote::try_from(vote)
    }

    fn transaction_pool_get(&mut self, transaction_id: &TransactionId) -> Result<TransactionPoolRecord, StorageError> {
        use crate::schema::transaction_pool;

        transaction_pool::table
            .filter(transaction_pool::transaction_id.eq(serialize_hex(transaction_id)))
            .first::<sql_models::TransactionPoolRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get",
                source: e,
            })?
            .try_into()
    }

    fn transaction_pool_get_many_ready(&mut self, max_txs: usize) -> Result<Vec<TransactionPoolRecord>, StorageError> {
        use crate::schema::transaction_pool;

        transaction_pool::table
            .filter(transaction_pool::is_ready.eq(true))
            .limit(max_txs as i64)
            .get_results::<sql_models::TransactionPoolRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get_many_ready",
                source: e,
            })?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    fn transaction_pool_count(
        &mut self,
        stage: Option<TransactionPoolStage>,
        is_ready: Option<bool>,
    ) -> Result<usize, StorageError> {
        use crate::schema::transaction_pool;
        let mut query = transaction_pool::table.into_boxed();
        if let Some(stage) = stage {
            query = query.filter(transaction_pool::stage.eq(stage.to_string()));
        }
        if let Some(is_ready) = is_ready {
            query = query.filter(transaction_pool::is_ready.eq(is_ready));
        }

        let count =
            query
                .count()
                .get_result::<i64>(self.connection())
                .map_err(|e| SqliteStorageError::DieselError {
                    operation: "transaction_pool_count",
                    source: e,
                })?;

        Ok(count as usize)
    }
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub count: i64,
}
