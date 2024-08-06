//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    ops::RangeInclusive,
    str::FromStr,
};

use bigdecimal::{BigDecimal, ToPrimitive};
use diesel::{
    dsl,
    query_builder::SqlQuery,
    sql_query,
    sql_types::{BigInt, Text},
    BoolExpressionMethods,
    ExpressionMethods,
    JoinOnDsl,
    NullableExpressionMethods,
    OptionalExtension,
    QueryDsl,
    QueryableByName,
    RunQueryDsl,
    SqliteConnection,
    TextExpressionMethods,
};
use indexmap::IndexMap;
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{shard::Shard, Epoch, NodeAddressable, NodeHeight, ShardGroup, SubstateAddress};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockDiff,
        BlockId,
        Command,
        EpochCheckpoint,
        ForeignProposal,
        ForeignProposalState,
        ForeignReceiveCounters,
        ForeignSendCounters,
        HighQc,
        LastExecuted,
        LastProposed,
        LastSentVote,
        LastVoted,
        LeafBlock,
        LockedBlock,
        LockedSubstate,
        PendingShardStateTreeDiff,
        QcId,
        QuorumCertificate,
        StateTransition,
        StateTransitionId,
        SubstateChange,
        SubstateRecord,
        TransactionExecution,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
        Vote,
    },
    Ordering,
    StateStoreReadTransaction,
    StorageError,
};
use tari_engine_types::substate::SubstateId;
use tari_state_tree::{Node, NodeKey, TreeNode, Version};
use tari_transaction::{SubstateRequirement, TransactionId, VersionedSubstateId};
use tari_utilities::ByteArray;

use crate::{
    error::SqliteStorageError,
    serialization::{deserialize_hex_try_from, deserialize_json, serialize_hex, serialize_json},
    sql_models,
    sqlite_transaction::SqliteTransaction,
};

const LOG_TARGET: &str = "tari::dan::storage::state_store_sqlite::reader";

pub struct SqliteStateStoreReadTransaction<'a, TAddr> {
    transaction: SqliteTransaction<'a>,
    _addr: PhantomData<TAddr>,
}

impl<'a, TAddr> SqliteStateStoreReadTransaction<'a, TAddr> {
    pub(crate) fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self {
            transaction,
            _addr: PhantomData,
        }
    }

    pub(crate) fn connection(&self) -> &mut SqliteConnection {
        self.transaction.connection()
    }

    pub(crate) fn commit(self) -> Result<(), SqliteStorageError> {
        self.transaction.commit()
    }

    pub(crate) fn rollback(self) -> Result<(), SqliteStorageError> {
        self.transaction.rollback()
    }
}

impl<'a, TAddr: NodeAddressable + Serialize + DeserializeOwned + 'a> SqliteStateStoreReadTransaction<'a, TAddr> {
    pub fn get_transaction_atom_state_updates_between_blocks<'i, ITx>(
        &self,
        from_block_id: &BlockId,
        to_block_id: &BlockId,
        transaction_ids: ITx,
    ) -> Result<HashMap<String, sql_models::TransactionPoolStateUpdate>, SqliteStorageError>
    where
        ITx: Iterator<Item = &'i str> + ExactSizeIterator,
    {
        if transaction_ids.len() == 0 {
            return Ok(HashMap::new());
        }

        let applicable_block_ids = self.get_block_ids_that_change_state_between(from_block_id, to_block_id)?;

        debug!(
            target: LOG_TARGET,
            "get_transaction_atom_state_updates_between_blocks: from_block_id={}, to_block_id={}, len(applicable_block_ids)={}",
            from_block_id,
            to_block_id,
            applicable_block_ids.len());

        if applicable_block_ids.is_empty() {
            return Ok(HashMap::new());
        }

        self.create_transaction_atom_updates_query(transaction_ids, applicable_block_ids.iter().map(|s| s.as_str()))
            .load_iter::<sql_models::TransactionPoolStateUpdate, _>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get_many_ready",
                source: e,
            })?
            .map(|update| update.map(|u| (u.transaction_id.clone(), u)))
            .collect::<diesel::QueryResult<HashMap<_, _>>>()
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get_many_ready",
                source: e,
            })
    }

    /// Creates a query to select the latest transaction pool state updates for the given transaction ids and block ids.
    /// WARNING: This method does not protect against SQL-injection, Be sure that the transaction ids and block ids
    /// strings are what they are meant to be.
    fn create_transaction_atom_updates_query<
        'i1,
        'i2,
        IBlk: Iterator<Item = &'i1 str> + ExactSizeIterator,
        ITx: Iterator<Item = &'i2 str> + ExactSizeIterator,
    >(
        &self,
        transaction_ids: ITx,
        block_ids: IBlk,
    ) -> SqlQuery {
        // Unfortunate hack. Binding array types in diesel is only supported for postgres.
        sql_query(format!(
            r#"
                 WITH RankedResults AS (
                    SELECT
                        tpsu.*,
                        ROW_NUMBER() OVER (PARTITION BY tpsu.transaction_id ORDER BY tpsu.block_height DESC) AS `rank`
                    FROM
                        transaction_pool_state_updates AS tpsu
                    WHERE
                        tpsu.block_id in ({})
                    AND tpsu.transaction_id in ({})
                )
                SELECT
                    id,
                    block_id,
                    block_height,
                    transaction_id,
                    stage,
                    evidence,
                    is_ready,
                    local_decision,
                    created_at
                FROM
                    RankedResults
                WHERE
                    rank = 1;
                "#,
            self.sql_frag_for_in_statement(block_ids, BlockId::byte_size() * 2),
            self.sql_frag_for_in_statement(transaction_ids, TransactionId::byte_size() * 2),
        ))
    }

    fn sql_frag_for_in_statement<'i, I: Iterator<Item = &'i str> + ExactSizeIterator>(
        &self,
        values: I,
        item_size: usize,
    ) -> String {
        let len = values.len();
        let mut sql_frag = String::with_capacity((len * item_size + len * 3 + len).saturating_sub(1));
        for (i, value) in values.enumerate() {
            sql_frag.push('"');
            sql_frag.push_str(value);
            sql_frag.push('"');
            if i < len - 1 {
                sql_frag.push(',');
            }
        }
        sql_frag
    }

    /// Returns the blocks from the start_block (inclusive) to the end_block (inclusive).
    fn get_block_ids_between(
        &self,
        start_block: &BlockId,
        end_block: &BlockId,
    ) -> Result<Vec<String>, SqliteStorageError> {
        debug!(target: LOG_TARGET, "get_block_ids_between: start: {start_block}, end: {end_block}");
        let block_ids = sql_query(
            r#"
            WITH RECURSIVE tree(bid, parent) AS (
                SELECT block_id, parent_block_id FROM blocks where block_id = ?
            UNION ALL
                SELECT block_id, parent_block_id
                FROM blocks JOIN tree ON
                    block_id = tree.parent
                    AND tree.bid != ?
                LIMIT 1000
            )
            SELECT bid FROM tree"#,
        )
        .bind::<Text, _>(serialize_hex(end_block))
        .bind::<Text, _>(serialize_hex(start_block))
        .load_iter::<BlockIdSqlValue, _>(self.connection())
        .map_err(|e| SqliteStorageError::DieselError {
            operation: "get_block_ids_that_change_state_between",
            source: e,
        })?;

        block_ids
            .map(|b| {
                b.map(|b| b.bid).map_err(|e| SqliteStorageError::DieselError {
                    operation: "get_block_ids_that_change_state_between",
                    source: e,
                })
            })
            .collect()
    }

    pub(crate) fn get_block_ids_that_change_state_between(
        &self,
        start_block: &BlockId,
        end_block: &BlockId,
    ) -> Result<Vec<String>, SqliteStorageError> {
        let block_ids = sql_query(
            r#"
            WITH RECURSIVE tree(bid, parent, is_dummy, command_count) AS (
                SELECT block_id, parent_block_id, is_dummy, command_count FROM blocks where block_id = ?
            UNION ALL
                SELECT block_id, parent_block_id, blocks.is_dummy, blocks.command_count
                FROM blocks JOIN tree ON
                    block_id = tree.parent
                    AND tree.bid != ?
                LIMIT 1000
            )
            SELECT bid FROM tree where is_dummy = 0 AND command_count > 0"#,
        )
        .bind::<Text, _>(serialize_hex(end_block))
        .bind::<Text, _>(serialize_hex(start_block))
        .load_iter::<BlockIdSqlValue, _>(self.connection())
        .map_err(|e| SqliteStorageError::DieselError {
            operation: "get_block_ids_that_change_state_between",
            source: e,
        })?;

        block_ids
            .map(|b| {
                b.map(|b| b.bid).map_err(|e| SqliteStorageError::DieselError {
                    operation: "get_block_ids_that_change_state_between",
                    source: e,
                })
            })
            .collect()
    }

    /// Used in tests, therefore not used in consensus and not part of the trait
    pub fn transactions_count(&self) -> Result<u64, SqliteStorageError> {
        use crate::schema::transactions;

        let count = transactions::table
            .count()
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_count",
                source: e,
            })?;

        Ok(count as u64)
    }

    pub(crate) fn get_commit_block_id(&self) -> Result<BlockId, StorageError> {
        use crate::schema::blocks;

        let locked = self.locked_block_get()?;

        let block_id = blocks::table
            .select(blocks::parent_block_id)
            .filter(blocks::block_id.eq(serialize_hex(locked.block_id)))
            .first::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "get_commit_block_id",
                source: e,
            })?;

        deserialize_hex_try_from(&block_id)
    }

    pub fn substates_count(&self) -> Result<u64, SqliteStorageError> {
        use crate::schema::substates;

        let count = substates::table
            .count()
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_count",
                source: e,
            })?;

        Ok(count as u64)
    }

    pub fn blocks_get_tip(&self, epoch: Epoch, shard_group: ShardGroup) -> Result<Block, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let (block, qc) = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::epoch.eq(epoch.as_u64() as i64))
            .filter(blocks::shard_group.eq(shard_group.encode_as_u32() as i32))
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
}

impl<'tx, TAddr: NodeAddressable + Serialize + DeserializeOwned + 'tx> StateStoreReadTransaction
    for SqliteStateStoreReadTransaction<'tx, TAddr>
{
    type Addr = TAddr;

    fn last_sent_vote_get(&self) -> Result<LastSentVote, StorageError> {
        use crate::schema::last_sent_vote;

        let last_voted = last_sent_vote::table
            .order_by(last_sent_vote::id.desc())
            .first::<sql_models::LastSentVote>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_sent_vote_get",
                source: e,
            })?;

        last_voted.try_into()
    }

    fn last_voted_get(&self) -> Result<LastVoted, StorageError> {
        use crate::schema::last_voted;

        let last_voted = last_voted::table
            .order_by(last_voted::id.desc())
            .first::<sql_models::LastVoted>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_voted_get",
                source: e,
            })?;

        last_voted.try_into()
    }

    fn last_executed_get(&self) -> Result<LastExecuted, StorageError> {
        use crate::schema::last_executed;

        let last_executed = last_executed::table
            .order_by(last_executed::id.desc())
            .first::<sql_models::LastExecuted>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_executed_get",
                source: e,
            })?;

        last_executed.try_into()
    }

    fn last_proposed_get(&self) -> Result<LastProposed, StorageError> {
        use crate::schema::last_proposed;

        let last_proposed = last_proposed::table
            .order_by(last_proposed::id.desc())
            .first::<sql_models::LastProposed>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "last_proposed_get",
                source: e,
            })?;

        last_proposed.try_into()
    }

    fn locked_block_get(&self) -> Result<LockedBlock, StorageError> {
        use crate::schema::locked_block;

        let locked_block = locked_block::table
            .order_by(locked_block::id.desc())
            .first::<sql_models::LockedBlock>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "locked_block_get",
                source: e,
            })?;

        locked_block.try_into()
    }

    fn leaf_block_get(&self) -> Result<LeafBlock, StorageError> {
        use crate::schema::leaf_blocks;

        let leaf_block = leaf_blocks::table
            .order_by(leaf_blocks::id.desc())
            .first::<sql_models::LeafBlock>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "leaf_block_get",
                source: e,
            })?;

        leaf_block.try_into()
    }

    fn high_qc_get(&self) -> Result<HighQc, StorageError> {
        use crate::schema::high_qcs;

        let high_qc = high_qcs::table
            .order_by(high_qcs::id.desc())
            .first::<sql_models::HighQc>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "high_qc_get",
                source: e,
            })?;

        high_qc.try_into()
    }

    fn foreign_proposal_exists(&self, foreign_proposal: &ForeignProposal) -> Result<bool, StorageError> {
        use crate::schema::foreign_proposals;

        let foreign_proposals = foreign_proposals::table
            .filter(foreign_proposals::shard_group.eq(foreign_proposal.shard_group.encode_as_u32() as i32))
            .filter(foreign_proposals::block_id.eq(serialize_hex(foreign_proposal.block_id)))
            .filter(foreign_proposals::transactions.eq(serialize_json(&foreign_proposal.transactions)?))
            .filter(foreign_proposals::base_layer_block_height.eq(foreign_proposal.base_layer_block_height as i64))
            .count()
            .limit(1)
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposal_exists",
                source: e,
            })?;

        Ok(foreign_proposals > 0)
    }

    fn foreign_proposal_get_all_new(&self) -> Result<Vec<ForeignProposal>, StorageError> {
        use crate::schema::foreign_proposals;

        let foreign_proposals = foreign_proposals::table
            .filter(foreign_proposals::state.eq(ForeignProposalState::New.to_string()))
            .load::<sql_models::ForeignProposal>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposal_get_all",
                source: e,
            })?;

        foreign_proposals.into_iter().map(|p| p.try_into()).collect()
    }

    fn foreign_proposal_get_all_pending(
        &self,
        from_block_id: &BlockId,
        to_block_id: &BlockId,
    ) -> Result<Vec<ForeignProposal>, StorageError> {
        use crate::schema::blocks;

        let blocks = self.get_block_ids_that_change_state_between(from_block_id, to_block_id)?;

        let all_commands: Vec<String> = blocks::table
            .select(blocks::commands)
            .filter(blocks::command_count.gt(0)) // if there is no command, then there is definitely no foreign proposal command
            .filter(blocks::block_id.eq_any(blocks))
            .load::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposal_get_all",
                source: e,
            })?;
        let all_commands = all_commands
            .into_iter()
            .map(|commands| deserialize_json(commands.as_str()))
            .collect::<Result<Vec<Vec<Command>>, _>>()?;
        let all_commands = all_commands.into_iter().flatten().collect::<Vec<_>>();
        Ok(all_commands
            .into_iter()
            .filter_map(|command| command.foreign_proposal().cloned())
            .collect::<Vec<ForeignProposal>>())
    }

    fn foreign_proposal_get_all_proposed(&self, to_height: NodeHeight) -> Result<Vec<ForeignProposal>, StorageError> {
        use crate::schema::foreign_proposals;

        let foreign_proposals = foreign_proposals::table
            .filter(foreign_proposals::state.eq(ForeignProposalState::Proposed.to_string()))
            .filter(foreign_proposals::proposed_height.le(to_height.0 as i64))
            .load::<sql_models::ForeignProposal>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposal_get_all",
                source: e,
            })?;

        foreign_proposals.into_iter().map(|p| p.try_into()).collect()
    }

    fn foreign_send_counters_get(&self, block_id: &BlockId) -> Result<ForeignSendCounters, StorageError> {
        use crate::schema::foreign_send_counters;

        let counter = foreign_send_counters::table
            .filter(foreign_send_counters::block_id.eq(serialize_hex(block_id)))
            .first::<sql_models::ForeignSendCounters>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_send_counters_get",
                source: e,
            })?;

        counter.try_into()
    }

    fn foreign_receive_counters_get(&self) -> Result<ForeignReceiveCounters, StorageError> {
        use crate::schema::foreign_receive_counters;

        let counter = foreign_receive_counters::table
            .order_by(foreign_receive_counters::id.desc())
            .first::<sql_models::ForeignReceiveCounters>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_receive_counters_get",
                source: e,
            })?;

        counter.try_into()
    }

    fn transactions_get(&self, tx_id: &TransactionId) -> Result<TransactionRecord, StorageError> {
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

    fn transactions_exists(&self, tx_id: &TransactionId) -> Result<bool, StorageError> {
        use crate::schema::transactions;

        let exists = transactions::table
            .count()
            .filter(transactions::transaction_id.eq(serialize_hex(tx_id)))
            .limit(1)
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_exists",
                source: e,
            })?;

        Ok(exists > 0)
    }

    fn transactions_get_any<'a, I: IntoIterator<Item = &'a TransactionId>>(
        &self,
        tx_ids: I,
    ) -> Result<Vec<TransactionRecord>, StorageError> {
        use crate::schema::transactions;

        let tx_ids: Vec<String> = tx_ids.into_iter().map(serialize_hex).collect();

        let transactions = transactions::table
            .filter(transactions::transaction_id.eq_any(tx_ids))
            .load::<sql_models::Transaction>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_get_any",
                source: e,
            })?;

        transactions
            .into_iter()
            .map(|transaction| transaction.try_into())
            .collect()
    }

    fn transactions_get_paginated(
        &self,
        limit: u64,
        offset: u64,
        asc_desc_created_at: Option<Ordering>,
    ) -> Result<Vec<TransactionRecord>, StorageError> {
        use crate::schema::transactions;

        let mut query = transactions::table.into_boxed();

        if let Some(ordering) = asc_desc_created_at {
            match ordering {
                Ordering::Ascending => query = query.order_by(transactions::created_at.asc()),
                Ordering::Descending => query = query.order_by(transactions::created_at.desc()),
            }
        }

        let transactions = query
            .limit(limit as i64)
            .offset(offset as i64)
            .get_results::<sql_models::Transaction>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transactions_get_paginated",
                source: e,
            })?;

        transactions
            .into_iter()
            .map(|transaction| transaction.try_into())
            .collect()
    }

    fn transaction_executions_get(
        &self,
        tx_id: &TransactionId,
        block: &BlockId,
    ) -> Result<TransactionExecution, StorageError> {
        use crate::schema::transaction_executions;

        let execution = transaction_executions::table
            .filter(transaction_executions::transaction_id.eq(serialize_hex(tx_id)))
            .filter(transaction_executions::block_id.eq(serialize_hex(block)))
            .first::<sql_models::TransactionExecution>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_executions_get",
                source: e,
            })?;

        execution.try_into()
    }

    fn transaction_executions_get_pending_for_block(
        &self,
        tx_id: &TransactionId,
        from_block_id: &BlockId,
    ) -> Result<TransactionExecution, StorageError> {
        use crate::schema::transaction_executions;

        // TODO: This gets slower as the chain progresses.
        let block_ids = self.get_block_ids_between(&BlockId::zero(), from_block_id)?;

        let execution = transaction_executions::table
            .filter(transaction_executions::transaction_id.eq(serialize_hex(tx_id)))
            .filter(transaction_executions::block_id.eq_any(block_ids))
            .order_by(transaction_executions::id.desc())
            .first::<sql_models::TransactionExecution>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_executions_get_pending_for_block",
                source: e,
            })?;

        execution.try_into()
    }

    fn blocks_get(&self, block_id: &BlockId) -> Result<Block, StorageError> {
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

    fn blocks_get_last_n_in_epoch(&self, n: usize, epoch: Epoch) -> Result<Vec<Block>, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let blocks = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::epoch.eq(epoch.as_u64() as i64))
            .filter(blocks::is_committed.eq(true))
            .order_by(blocks::height.desc())
            .limit(n as i64)
            .get_results::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get_last_n_in_epoch",
                source: e,
            })?;

        blocks
            .into_iter()
            // Order from lowest to highest height
            .rev()
            .map(|(b, qc)| {
                qc.ok_or_else(|| StorageError::DataInconsistency {
                    details: format!(
                        "blocks_get_last_n_in_epoch: block {} references non-existent quorum certificate {}",
                        b.block_id, b.qc_id
                    ),
                })
                .and_then(|qc| b.try_convert(qc))
            })
            .collect()
    }

    fn blocks_get_all_between(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
        start_block_id_exclusive: &BlockId,
        end_block_id_inclusive: &BlockId,
        include_dummy_blocks: bool,
    ) -> Result<Vec<Block>, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let block_ids = self.get_block_ids_between(start_block_id_exclusive, end_block_id_inclusive)?;
        if block_ids.is_empty() {
            return Ok(vec![]);
        }

        let mut query = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::block_id.eq_any(block_ids))
            .into_boxed();

        if !include_dummy_blocks {
            query = query.filter(blocks::is_dummy.eq(false));
        }

        let results = query
            .filter(blocks::epoch.eq(epoch.as_u64() as i64))
            .filter(blocks::shard_group.eq(shard_group.encode_as_u32() as i32))
            .order_by(blocks::height.asc())
            .get_results::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_all_after_height",
                source: e,
            })?;

        results
            .into_iter()
            .map(|(block, qc)| {
                let qc = qc.ok_or_else(|| SqliteStorageError::DbInconsistency {
                    operation: "blocks_all_after_height",
                    details: format!(
                        "block {} references non-existent quorum certificate {}",
                        block.block_id, block.qc_id
                    ),
                })?;

                block.try_convert(qc)
            })
            .collect()
    }

    fn blocks_exists(&self, block_id: &BlockId) -> Result<bool, StorageError> {
        use crate::schema::blocks;

        let count = blocks::table
            .filter(blocks::block_id.eq(serialize_hex(block_id)))
            .count()
            .limit(1)
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_exists",
                source: e,
            })?;

        Ok(count > 0)
    }

    fn blocks_is_ancestor(&self, descendant: &BlockId, ancestor: &BlockId) -> Result<bool, StorageError> {
        if !self.blocks_exists(descendant)? {
            return Err(StorageError::QueryError {
                reason: format!("blocks_is_ancestor: descendant block {} does not exist", descendant),
            });
        }

        if !self.blocks_exists(ancestor)? {
            return Err(StorageError::QueryError {
                reason: format!("blocks_is_ancestor: ancestor block {} does not exist", ancestor),
            });
        }

        // TODO: this scans all the way to genesis for every query - can optimise though it's low priority for now
        let is_ancestor = sql_query(
            r#"
            WITH RECURSIVE tree(bid, parent) AS (
                  SELECT block_id, parent_block_id FROM blocks where block_id = ?
                UNION ALL
                  SELECT block_id, parent_block_id
                    FROM blocks JOIN tree ON block_id = tree.parent AND tree.bid != tree.parent -- stop recursing at zero block (or any self referencing block)
            )
            SELECT count(1) as "count" FROM tree WHERE bid = ? LIMIT 1
        "#,
        )
            .bind::<Text, _>(serialize_hex(descendant))
            // .bind::<Text, _>(serialize_hex(BlockId::genesis())) // stop recursing at zero block
            .bind::<Text, _>(serialize_hex(ancestor))
            .get_result::<Count>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_is_ancestor",
                source: e,
            })?;

        debug!(target: LOG_TARGET, "blocks_is_ancestor: is_ancestor: {}", is_ancestor.count);

        Ok(is_ancestor.count > 0)
    }

    fn blocks_get_all_by_parent(&self, parent_id: &BlockId) -> Result<Vec<Block>, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let results = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::parent_block_id.eq(serialize_hex(parent_id)))
            .filter(blocks::block_id.ne(blocks::parent_block_id)) // Exclude the genesis block
            .get_results::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get_by_parent",
                source: e,
            })?;

        results
            .into_iter()
            .map(|(block, qc)| {
                let qc = qc.ok_or_else(|| SqliteStorageError::DbInconsistency {
                    operation: "blocks_get_by_parent",
                    details: format!(
                        "block {} references non-existent quorum certificate {}",
                        parent_id, block.qc_id
                    ),
                })?;

                block.try_convert(qc)
            })
            .collect()
    }

    fn blocks_get_parent_chain(&self, block_id: &BlockId, limit: usize) -> Result<Vec<Block>, StorageError> {
        if !self.blocks_exists(block_id)? {
            return Err(StorageError::QueryError {
                reason: format!("blocks_get_parent_chain: descendant block {} does not exist", block_id),
            });
        }
        let blocks = sql_query(
            r#"
            WITH RECURSIVE tree(bid, parent) AS (
                  SELECT block_id, parent_block_id FROM blocks where block_id = ?
                UNION ALL
                  SELECT block_id, parent_block_id
                    FROM blocks JOIN tree ON block_id = tree.parent AND tree.bid != tree.parent
                    LIMIT ?
            )
            SELECT blocks.*, quorum_certificates.* FROM tree
                INNER JOIN blocks ON blocks.block_id = tree.bid
                LEFT JOIN quorum_certificates ON blocks.qc_id = quorum_certificates.qc_id
                ORDER BY height desc
        "#,
        )
        .bind::<Text, _>(serialize_hex(block_id))
        .bind::<BigInt, _>(limit as i64)
        .get_results::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
        .map_err(|e| SqliteStorageError::DieselError {
            operation: "blocks_get_parent_chain",
            source: e,
        })?;

        blocks
            .into_iter()
            .map(|(b, qc)| {
                let qc = qc.ok_or_else(|| SqliteStorageError::DbInconsistency {
                    operation: "blocks_get_by_parent",
                    details: format!(
                        "block {} references non-existent quorum certificate {}",
                        block_id, b.qc_id
                    ),
                })?;

                b.try_convert(qc)
            })
            .collect()
    }

    fn blocks_get_pending_transactions(&self, block_id: &BlockId) -> Result<Vec<TransactionId>, StorageError> {
        use crate::schema::missing_transactions;

        let txs = missing_transactions::table
            .select(missing_transactions::transaction_id)
            .filter(missing_transactions::block_id.eq(serialize_hex(block_id)))
            .get_results::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get_missing_transactions",
                source: e,
            })?;
        txs.into_iter().map(|s| deserialize_hex_try_from(&s)).collect()
    }

    fn blocks_get_total_leader_fee_for_epoch(
        &self,
        epoch: Epoch,
        validator_public_key: &PublicKey,
    ) -> Result<u64, StorageError> {
        use crate::schema::blocks;

        let total_fee = blocks::table
            .select(diesel::dsl::sum(blocks::total_leader_fee))
            .filter(blocks::epoch.eq(epoch.as_u64() as i64))
            .filter(blocks::proposed_by.eq(serialize_hex(validator_public_key.as_bytes())))
            .first::<Option<BigDecimal>>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "validator_fees_get_total_fee_for_epoch",
                source: e,
            })?
            .unwrap_or(BigDecimal::default());

        Ok(total_fee.to_u64().expect("total fee overflows u64"))
    }

    fn blocks_get_any_with_epoch_range(
        &self,
        epoch_range: RangeInclusive<Epoch>,
        validator_public_key: Option<&PublicKey>,
    ) -> Result<Vec<Block>, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let mut query = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::epoch.between(epoch_range.start().as_u64() as i64, epoch_range.end().as_u64() as i64))
            .into_boxed();

        if let Some(vn) = validator_public_key {
            query = query.filter(blocks::proposed_by.eq(serialize_hex(vn.as_bytes())));
        }

        let blocks_and_qcs = query
            .get_results::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "validator_fees_get_any_with_epoch_range_for_validator",
                source: e,
            })?;

        blocks_and_qcs
            .into_iter()
            .map(|(block, qc)| {
                let qc = qc.ok_or_else(|| SqliteStorageError::DbInconsistency {
                    operation: "blocks_get_by_parent",
                    details: format!(
                        "block {} references non-existent quorum certificate {}",
                        block.id, block.qc_id
                    ),
                })?;

                block.try_convert(qc)
            })
            .collect()
    }

    fn blocks_get_paginated(
        &self,
        limit: u64,
        offset: u64,
        filter_index: Option<usize>,
        filter: Option<String>,
        ordering_index: Option<usize>,
        ordering: Option<Ordering>,
    ) -> Result<Vec<Block>, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let mut query = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .into_boxed();

        query = match ordering {
            Some(Ordering::Ascending) => match ordering_index {
                Some(0) => query.order_by(blocks::block_id.asc()),
                Some(1) => query.order_by(blocks::epoch.asc()),
                Some(2) => query.order_by(blocks::epoch.asc()).then_order_by(blocks::height.asc()),
                Some(4) => query.order_by(blocks::command_count.asc()),
                Some(5) => query.order_by(blocks::total_leader_fee.asc()),
                Some(6) => query.order_by(blocks::block_time.asc()),
                Some(7) => query.order_by(blocks::created_at.asc()),
                Some(8) => query.order_by(blocks::proposed_by.asc()),
                _ => query.order_by(blocks::epoch.asc()).then_order_by(blocks::height.asc()),
            },
            _ => match ordering_index {
                Some(0) => query.order_by(blocks::block_id.desc()),
                Some(1) => query.order_by(blocks::epoch.desc()),
                Some(2) => query
                    .order_by(blocks::epoch.desc())
                    .then_order_by(blocks::height.desc()),
                Some(4) => query.order_by(blocks::command_count.desc()),
                Some(5) => query.order_by(blocks::total_leader_fee.desc()),
                Some(6) => query.order_by(blocks::block_time.desc()),
                Some(7) => query.order_by(blocks::created_at.desc()),
                Some(8) => query.order_by(blocks::proposed_by.desc()),
                _ => query
                    .order_by(blocks::epoch.desc())
                    .then_order_by(blocks::height.desc()),
            },
        };

        if let Some(filter) = filter {
            if !filter.is_empty() {
                if let Some(filter_index) = filter_index {
                    match filter_index {
                        0 => query = query.filter(blocks::block_id.like(format!("%{filter}%"))),
                        1 => {
                            query = query.filter(
                                blocks::epoch
                                    .eq(filter.parse::<i64>().map_err(|_| StorageError::InvalidIntegerCast)?),
                            )
                        },
                        2 => {
                            query = query.filter(
                                blocks::height
                                    .eq(filter.parse::<i64>().map_err(|_| StorageError::InvalidIntegerCast)?),
                            )
                        },
                        4 => {
                            query = query.filter(
                                blocks::command_count
                                    .ge(filter.parse::<i64>().map_err(|_| StorageError::InvalidIntegerCast)?),
                            )
                        },
                        5 => {
                            query = query.filter(
                                blocks::total_leader_fee
                                    .ge(filter.parse::<i64>().map_err(|_| StorageError::InvalidIntegerCast)?),
                            )
                        },
                        7 => query = query.filter(blocks::proposed_by.like(format!("%{filter}%"))),
                        _ => (),
                    }
                }
            }
        }

        let blocks = query
            .limit(limit as i64)
            .offset(offset as i64)
            .get_results::<(sql_models::Block, Option<sql_models::QuorumCertificate>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get_paginated",
                source: e,
            })?;

        blocks
            .into_iter()
            .map(|(block, qc)| {
                let qc = qc.ok_or_else(|| SqliteStorageError::DbInconsistency {
                    operation: "blocks_get_paginated",
                    details: format!(
                        "block {} references non-existent quorum certificate {}",
                        block.id, block.qc_id
                    ),
                })?;

                block.try_convert(qc)
            })
            .collect()
    }

    fn blocks_get_count(&self) -> Result<i64, StorageError> {
        use crate::schema::{blocks, quorum_certificates};
        let count = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select(diesel::dsl::count(blocks::id))
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_get_count",
                source: e,
            })?;
        Ok(count)
    }

    fn filtered_blocks_get_count(
        &self,
        filter_index: Option<usize>,
        filter: Option<String>,
    ) -> Result<i64, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let mut query = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .into_boxed();

        if let Some(filter) = filter {
            if !filter.is_empty() {
                if let Some(filter_index) = filter_index {
                    match filter_index {
                        0 => query = query.filter(blocks::block_id.like(format!("%{filter}%"))),
                        1 => {
                            query = query.filter(
                                blocks::epoch
                                    .eq(filter.parse::<i64>().map_err(|_| StorageError::InvalidIntegerCast)?),
                            )
                        },
                        2 => {
                            query = query.filter(
                                blocks::height
                                    .eq(filter.parse::<i64>().map_err(|_| StorageError::InvalidIntegerCast)?),
                            )
                        },
                        4 => {
                            query = query.filter(
                                blocks::command_count
                                    .ge(filter.parse::<i64>().map_err(|_| StorageError::InvalidIntegerCast)?),
                            )
                        },
                        5 => {
                            query = query.filter(
                                blocks::total_leader_fee
                                    .ge(filter.parse::<i64>().map_err(|_| StorageError::InvalidIntegerCast)?),
                            )
                        },
                        7 => query = query.filter(blocks::proposed_by.like(format!("%{filter}%"))),
                        _ => (),
                    }
                }
            }
        }

        let count = query
            .select(diesel::dsl::count(blocks::id))
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "filtered_blocks_get_count",
                source: e,
            })?;
        Ok(count)
    }

    fn blocks_max_height(&self) -> Result<NodeHeight, StorageError> {
        use crate::schema::blocks;

        let height = blocks::table
            .select(diesel::dsl::max(blocks::height))
            .first::<Option<i64>>(self.connection())
            .map(|height| NodeHeight(height.unwrap_or(0) as u64))
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_max_height",
                source: e,
            })?;

        Ok(height)
    }

    fn block_diffs_get(&self, block_id: &BlockId) -> Result<BlockDiff, StorageError> {
        use crate::schema::block_diffs;

        let block_diff = block_diffs::table
            .filter(block_diffs::block_id.eq(serialize_hex(block_id)))
            .get_results::<sql_models::BlockDiff>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "block_diffs_get",
                source: e,
            })?;

        sql_models::BlockDiff::try_load(*block_id, block_diff)
    }

    fn block_diffs_get_last_change_for_substate(
        &self,
        block_id: &BlockId,
        substate_id: &SubstateId,
    ) -> Result<SubstateChange, StorageError> {
        use crate::schema::block_diffs;
        let commit_block = self.get_commit_block_id()?;
        let block_ids = self.get_block_ids_that_change_state_between(&commit_block, block_id)?;

        let diff = block_diffs::table
            .filter(block_diffs::block_id.eq_any(block_ids))
            .filter(block_diffs::substate_id.eq(substate_id.to_string()))
            .order_by(block_diffs::id.desc())
            .first::<sql_models::BlockDiff>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "block_diffs_get_last_change_for_substate",
                source: e,
            })?;

        sql_models::BlockDiff::try_convert_change(diff)
    }

    fn parked_blocks_exists(&self, block_id: &BlockId) -> Result<bool, StorageError> {
        use crate::schema::parked_blocks;

        let block_id = serialize_hex(block_id);

        let count = parked_blocks::table
            .filter(parked_blocks::block_id.eq(&block_id))
            .count()
            .limit(1)
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "blocks_exists_or_parked",
                source: e,
            })?;

        Ok(count > 0)
    }

    fn quorum_certificates_get(&self, qc_id: &QcId) -> Result<QuorumCertificate, StorageError> {
        use crate::schema::quorum_certificates;

        let qc_json = quorum_certificates::table
            .select(quorum_certificates::json)
            .filter(quorum_certificates::qc_id.eq(serialize_hex(qc_id)))
            .first::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "quorum_certificates_get",
                source: e,
            })?;

        deserialize_json(&qc_json)
    }

    fn quorum_certificates_get_all<'a, I: IntoIterator<Item = &'a QcId>>(
        &self,
        qc_ids: I,
    ) -> Result<Vec<QuorumCertificate>, StorageError> {
        use crate::schema::quorum_certificates;

        let qc_ids: Vec<String> = qc_ids.into_iter().map(serialize_hex).collect();

        let qc_json = quorum_certificates::table
            .select(quorum_certificates::json)
            .filter(quorum_certificates::qc_id.eq_any(&qc_ids))
            .get_results::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "quorum_certificates_get_all",
                source: e,
            })?;

        if qc_json.len() != qc_ids.len() {
            return Err(SqliteStorageError::NotAllItemsFound {
                items: "QCs",
                operation: "quorum_certificates_get_all",
                details: format!(
                    "quorum_certificates_get_all: expected {} quorum certificates, got {}",
                    qc_ids.len(),
                    qc_json.len()
                ),
            }
            .into());
        }

        qc_json.iter().map(|j| deserialize_json(j)).collect()
    }

    fn quorum_certificates_get_by_block_id(&self, block_id: &BlockId) -> Result<QuorumCertificate, StorageError> {
        use crate::schema::quorum_certificates;

        let qc_json = quorum_certificates::table
            .select(quorum_certificates::json)
            .filter(quorum_certificates::block_id.eq(serialize_hex(block_id)))
            .first::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "quorum_certificates_get_by_block_id",
                source: e,
            })?;

        deserialize_json(&qc_json)
    }

    fn transaction_pool_get_for_blocks(
        &self,
        from_block_id: &BlockId,
        to_block_id: &BlockId,
        transaction_id: &TransactionId,
    ) -> Result<TransactionPoolRecord, StorageError> {
        use crate::schema::transaction_pool;

        let transaction_id = serialize_hex(transaction_id);
        let mut updates = self.get_transaction_atom_state_updates_between_blocks(
            from_block_id,
            to_block_id,
            std::iter::once(transaction_id.as_str()),
        )?;

        debug!(
            target: LOG_TARGET,
            "transaction_pool_get: from_block_id={}, to_block_id={}, transaction_id={}, updates={}",
            from_block_id,
            to_block_id,
            transaction_id,
            updates.len()
        );

        let rec = transaction_pool::table
            .filter(transaction_pool::transaction_id.eq(&transaction_id))
            .first::<sql_models::TransactionPoolRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get",
                source: e,
            })?;

        rec.try_convert(updates.remove(&transaction_id))
    }

    fn transaction_pool_exists(&self, transaction_id: &TransactionId) -> Result<bool, StorageError> {
        use crate::schema::transaction_pool;

        let count = transaction_pool::table
            .count()
            .filter(transaction_pool::transaction_id.eq(serialize_hex(transaction_id)))
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get",
                source: e,
            })?;

        Ok(count > 0)
    }

    fn transaction_pool_get_all(&self) -> Result<Vec<TransactionPoolRecord>, StorageError> {
        use crate::schema::transaction_pool;
        let txs = transaction_pool::table
            .get_results::<sql_models::TransactionPoolRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get_all",
                source: e,
            })?;
        // TODO: need to get the updates - this is just used in JRPC so it doesnt matter too much
        txs.into_iter().map(|tx| tx.try_convert(None)).collect()
    }

    fn transaction_pool_get_many_ready(&self, max_txs: usize) -> Result<Vec<TransactionPoolRecord>, StorageError> {
        use crate::schema::transaction_pool;

        let ready_txs = transaction_pool::table
            .order_by(transaction_pool::transaction_id.asc())
            .get_results::<sql_models::TransactionPoolRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get_many_ready",
                source: e,
            })?;

        if ready_txs.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch all applicable block ids between the locked block and the given block
        let locked = self.locked_block_get()?;
        let leaf = self.leaf_block_get()?;

        let mut updates = self.get_transaction_atom_state_updates_between_blocks(
            &locked.block_id,
            &leaf.block_id,
            ready_txs.iter().map(|s| s.transaction_id.as_str()),
        )?;

        debug!(
            target: LOG_TARGET,
            "transaction_pool_get: from_block_id={}, to_block_id={}, len(ready_txs)={}, updates={}",
            locked.block_id,
            leaf.block_id,
            ready_txs.len(),
            updates.len()
        );

        ready_txs
            .into_iter()
            .map(|rec| {
                let maybe_update = updates.remove(&rec.transaction_id);
                rec.try_convert(maybe_update)
            })
            // Filter only Ok where is_ready == true (after update) or Err
            .filter(|result| result.as_ref().map_or(true, |rec| rec.is_ready()))
            .take(max_txs)
            .collect()
        // let mut used_substates = HashMap::<SubstateAddress, SubstateLockFlag>::new();
        // let mut processed_substates =
        //     HashMap::<TransactionId, HashSet<(SubstateAddress, _)>>::with_capacity(updates.len());
        // for (tx_id, update) in &updates {
        //     if update.local_decision.as_deref() == Some(Decision::Abort.as_str()) {
        //         // The aborted transaction don't lock any substates
        //         continue;
        //     }
        //     let evidence = deserialize_json::<Evidence>(&update.evidence)?;
        //     let evidence = evidence
        //         .iter()
        //         .map(|(shard, evidence)| (*shard, evidence.lock))
        //         .collect::<HashSet<(SubstateAddress, _)>>();
        //     processed_substates.insert(deserialize_hex_try_from(tx_id)?, evidence);
        // }
        //
        // ready_txs
        //     .into_iter()
        //     .filter_map(|rec| {
        //         let maybe_update = updates.remove(&rec.transaction_id);
        //         match rec.try_convert(maybe_update) {
        //             Ok(rec) => {
        //                 if !rec.is_ready() {
        //                     return None;
        //                 }
        //
        //                 let tx_substates = rec
        //                     .transaction()
        //                     .evidence
        //                     .iter()
        //                     .map(|(shard, evidence)| (*shard, evidence.lock))
        //                     .collect::<HashMap<_, _>>();
        //
        //                 // Are there any conflicts between the currently selected set and this transaction?
        //                 if tx_substates.iter().any(|(shard, lock)| {
        //                     if lock.is_write() {
        //                         // Write lock must have no conflicts
        //                         used_substates.contains_key(shard)
        //                     } else {
        //                         // If there is a Shard conflict, then it must not be a write lock
        //                         used_substates
        //                             .get(shard)
        //                             .map(|tx_lock| tx_lock.is_write())
        //                             .unwrap_or(false)
        //                     }
        //                 }) {
        //                     return None;
        //                 }
        //
        //                 // Are there any conflicts between this transaction and other transactions to be included in
        // the                 // block?
        //                 if processed_substates
        //                     .iter()
        //                     // Check other transactions
        //                     .filter(|(tx_id, _)| *tx_id != rec.transaction_id())
        //                     .all(|(_, evidence)| {
        //                         evidence.iter().all(|(shard, lock)| {
        //                             if lock.is_write() {
        //                                 // Write lock must have no conflicts
        //                                 !tx_substates.contains_key(shard)
        //                             } else {
        //                                 // If there is a Shard conflict, then it must be a read lock
        //                                 tx_substates.get(shard).map(|tx_lock| tx_lock.is_read()).unwrap_or(true)
        //                             }
        //                         })
        //                     })
        //                 {
        //                     used_substates.extend(tx_substates);
        //                     Some(Ok(rec))
        //                 } else {
        //                     // TODO: If we don't switch to "no version" transaction, then we can abort these here.
        //                     // That also requires changes to the on_ready_to_vote_on_local_block
        //                     None
        //                 }
        //             },
        //             Err(e) => Some(Err(e)),
        //         }
        //     })
        //     .take(max_txs)
        //     .collect()
    }

    fn transaction_pool_count(
        &self,
        stage: Option<TransactionPoolStage>,
        is_ready: Option<bool>,
        has_foreign_data: Option<bool>,
    ) -> Result<usize, StorageError> {
        use crate::schema::transaction_pool;

        let mut query = transaction_pool::table.into_boxed();
        if let Some(stage) = stage {
            query = query.filter(
                transaction_pool::pending_stage
                    .eq(stage.to_string())
                    .or(transaction_pool::pending_stage
                        .is_null()
                        .and(transaction_pool::stage.eq(stage.to_string()))),
            );
        }
        if let Some(is_ready) = is_ready {
            query = query.filter(transaction_pool::is_ready.eq(is_ready));
        }

        match has_foreign_data {
            Some(true) => {
                query = query.filter(transaction_pool::remote_evidence.is_not_null());
            },
            Some(false) => {
                query = query.filter(transaction_pool::remote_evidence.is_null());
            },
            None => {},
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

    fn transactions_fetch_involved_shards(
        &self,
        transaction_ids: HashSet<TransactionId>,
    ) -> Result<HashSet<SubstateAddress>, StorageError> {
        use crate::schema::transactions;

        let tx_ids = transaction_ids.into_iter().map(serialize_hex).collect::<Vec<_>>();

        let inputs_per_tx = transactions::table
            .select(transactions::resolved_inputs)
            .filter(transactions::transaction_id.eq_any(&tx_ids))
            .load::<Option<String>>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pools_fetch_involved_shards",
                source: e,
            })?;

        if inputs_per_tx.len() != tx_ids.len() {
            return Err(SqliteStorageError::NotAllItemsFound {
                items: "Transactions",
                operation: "transactions_fetch_involved_shards",
                details: format!(
                    "transactions_fetch_involved_shards: expected {} transactions, got {}",
                    tx_ids.len(),
                    inputs_per_tx.len()
                ),
            }
            .into());
        }

        let shards = inputs_per_tx
            .into_iter()
            .filter_map(|inputs| {
                // a Result is very inconvenient with flat_map
                inputs.map(|inputs| {
                    deserialize_json::<HashSet<SubstateAddress>>(&inputs)
                        .expect("[transactions_fetch_involved_shards] Failed to deserialize involved shards")
                })
            })
            .flatten()
            .collect();

        Ok(shards)
    }

    fn votes_get_by_block_and_sender(
        &self,
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

    fn votes_count_for_block(&self, block_id: &BlockId) -> Result<u64, StorageError> {
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

    fn votes_get_for_block(&self, block_id: &BlockId) -> Result<Vec<Vote>, StorageError> {
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

    fn substates_get(&self, address: &SubstateAddress) -> Result<SubstateRecord, StorageError> {
        use crate::schema::substates;

        let substate = substates::table
            .filter(substates::address.eq(serialize_hex(address)))
            .first::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get",
                source: e,
            })?;

        substate.try_into()
    }

    fn substates_get_any(
        &self,
        substate_ids: &HashSet<SubstateRequirement>,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;

        let mut query = substates::table.into_boxed();

        for id in substate_ids {
            let id_str = id.substate_id.to_string();
            match id.version() {
                Some(v) => {
                    query = query.or_filter(substates::substate_id.eq(id_str).and(substates::version.eq(v as i32)));
                },
                None => {
                    // Select the max known version
                    query = query.or_filter(substates::substate_id.eq(id_str.clone()).and(substates::version.eq(
                        dsl::sql("SELECT MAX(version) FROM substates WHERE substate_id = ?").bind::<Text, _>(id_str),
                    )));
                },
            }
        }

        let results = query
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_any",
                source: e,
            })?;

        results.into_iter().map(TryInto::try_into).collect()
    }

    fn substates_get_any_max_version<'a, I: IntoIterator<Item = &'a SubstateId>>(
        &self,
        substate_ids: I,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;
        #[derive(Debug, QueryableByName)]
        struct MaxVersionAndId {
            #[allow(dead_code)]
            #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
            max_version: Option<i32>,
            #[diesel(sql_type = diesel::sql_types::Integer)]
            id: i32,
        }

        let substate_ids = substate_ids.into_iter().map(ToString::to_string).collect::<Vec<_>>();
        if substate_ids.is_empty() {
            return Ok(Vec::new());
        }
        let frag = self.sql_frag_for_in_statement(substate_ids.iter().map(|s| s.as_str()), 32);
        let max_versions_and_ids = sql_query(format!(
            r#"
                SELECT MAX(version) as max_version, id
                FROM substates
                WHERE substate_id in ({})
                GROUP BY substate_id"#,
            frag
        ))
        .get_results::<MaxVersionAndId>(self.connection())
        .map_err(|e| SqliteStorageError::DieselError {
            operation: "substates_get_any_max_version",
            source: e,
        })?;

        let results = substates::table
            .filter(substates::id.eq_any(max_versions_and_ids.iter().map(|m| m.id)))
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_any_max_version",
                source: e,
            })?;

        // let results = substates::table
        //     .group_by(substates::substate_id)
        //     .select((substates::all_columns, dsl::max(substates::version))
        //     .filter(substates::substate_id.eq_any(substate_ids.into_iter().map(ToString::to_string)))
        //     .get_results::<(sql_models::SubstateRecord, Option<i32>)>(self.connection())
        //     .map_err(|e| SqliteStorageError::DieselError {
        //         operation: "substates_get_any_max_version",
        //         source: e,
        //     })?;

        results.into_iter().map(TryInto::try_into).collect()
    }

    fn substates_any_exist<I: IntoIterator<Item = S>, S: Borrow<VersionedSubstateId>>(
        &self,
        addresses: I,
    ) -> Result<bool, StorageError> {
        use crate::schema::substates;

        let count = substates::table
            .count()
            .filter(
                substates::address.eq_any(
                    addresses
                        .into_iter()
                        .map(|v| v.borrow().to_substate_address())
                        .map(serialize_hex),
                ),
            )
            .limit(1)
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_any",
                source: e,
            })?;

        Ok(count > 0)
    }

    fn substates_exists_for_transaction(&self, transaction_id: &TransactionId) -> Result<bool, StorageError> {
        use crate::schema::substates;

        let transaction_id = serialize_hex(transaction_id);

        let count = substates::table
            .count()
            .filter(substates::created_by_transaction.eq(&transaction_id))
            .or_filter(substates::destroyed_by_transaction.eq(&transaction_id))
            .limit(1)
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_exists_for_transaction",
                source: e,
            })?;

        Ok(count > 0)
    }

    fn substates_get_n_after(&self, n: usize, after: &SubstateAddress) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;

        let start_id = substates::table
            .select(substates::id)
            .filter(substates::address.eq(after.to_string()))
            .get_result::<i32>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_n_after",
                source: e,
            })?;

        let substates = substates::table
            .filter(substates::id.gt(start_id))
            .limit(n as i64)
            .order_by(substates::id.asc())
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_n_after",
                source: e,
            })?;

        substates.into_iter().map(TryInto::try_into).collect()
    }

    fn substates_get_many_within_range(
        &self,
        start: &SubstateAddress,
        end: &SubstateAddress,
        exclude: &[SubstateAddress],
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;

        let substates = substates::table
            .filter(substates::address.between(serialize_hex(start), serialize_hex(end)))
            .filter(substates::address.ne_all(exclude.iter().map(serialize_hex)))
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_many_within_range",
                source: e,
            })?;

        substates.into_iter().map(TryInto::try_into).collect()
    }

    fn substates_get_many_by_created_transaction(
        &self,
        tx_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;

        let substates = substates::table
            .filter(substates::created_by_transaction.eq(serialize_hex(tx_id)))
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_many_by_created_transaction",
                source: e,
            })?;

        substates.into_iter().map(TryInto::try_into).collect()
    }

    fn substates_get_many_by_destroyed_transaction(
        &self,
        tx_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;

        let substates = substates::table
            .filter(substates::destroyed_by_transaction.eq(serialize_hex(tx_id)))
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_many_by_destroyed_transaction",
                source: e,
            })?;

        substates.into_iter().map(TryInto::try_into).collect()
    }

    fn substates_get_all_for_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;

        let transaction_id_hex = serialize_hex(transaction_id);

        let substates = substates::table
            .filter(
                substates::created_by_transaction
                    .eq(&transaction_id_hex)
                    .or(substates::destroyed_by_transaction.eq(Some(&transaction_id_hex))),
            )
            .order_by(substates::id.asc())
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_all_for_transaction",
                source: e,
            })?;

        let substates = substates
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(substates)
    }

    fn substate_locks_get_all_for_block(
        &self,
        block_id: BlockId,
    ) -> Result<IndexMap<SubstateId, Vec<LockedSubstate>>, StorageError> {
        use crate::schema::substate_locks;

        // TODO: we need to exclude locks from blocks that are not referenced by the given block_id chain. Up to the
        // locked block we can use this query, but there may be unused locks from blocks that didn't make
        // it into the chain. If we can ensure that those locks are removed e.g remove all locks with height < commit
        // block, then we just have to exclude locks for the forked chain and load everything else.
        // For now, we just fetch all block ids in the chain relevant to the given block, which will be slower and more
        // memory intensive as the chain progresses.
        let block_ids = self.get_block_ids_that_change_state_between(&BlockId::zero(), &block_id)?;

        let lock_recs = substate_locks::table
            .filter(substate_locks::block_id.eq_any(block_ids))
            .order_by(substate_locks::id.asc())
            .get_results::<sql_models::SubstateLock>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_locks_get_all",
                source: e,
            })?;

        let mut locks = IndexMap::<_, Vec<_>>::with_capacity(lock_recs.len());
        for lock in lock_recs {
            let id = SubstateId::from_str(&lock.substate_id).map_err(|e| SqliteStorageError::MalformedDbData {
                operation: "substate_locks_get_all",
                details: format!("'{}' is not a valid SubstateId: {}", lock.substate_id, e),
            })?;
            locks.entry(id).or_default().push(lock.try_into_substate_lock()?);
        }

        Ok(locks)
    }

    fn substate_locks_get_latest_for_substate(&self, substate_id: &SubstateId) -> Result<LockedSubstate, StorageError> {
        use crate::schema::substate_locks;

        // TODO: this may return an invalid lock if:
        // 1. the proposer links the parent block to the locked block instead of the previous tip
        // 2. if there are any inactive locks that were not removed from previous uncommitted blocks.

        let lock = substate_locks::table
            .filter(substate_locks::substate_id.eq(substate_id.to_string()))
            .order_by(substate_locks::id.desc())
            .first::<sql_models::SubstateLock>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substate_locks_get_latest_for_substate",
                source: e,
            })?;

        lock.try_into_substate_lock()
    }

    fn pending_state_tree_diffs_get_all_up_to_commit_block(
        &self,
        block_id: &BlockId,
    ) -> Result<HashMap<Shard, Vec<PendingShardStateTreeDiff>>, StorageError> {
        use crate::schema::pending_state_tree_diffs;

        // Get the last committed block
        let committed_block_id = self.get_commit_block_id()?;

        let block_ids = self.get_block_ids_that_change_state_between(&committed_block_id, block_id)?;

        if block_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let diff_recs = pending_state_tree_diffs::table
            .filter(pending_state_tree_diffs::block_id.eq_any(block_ids))
            .order_by(pending_state_tree_diffs::block_height.asc())
            .get_results::<sql_models::PendingStateTreeDiff>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "pending_state_tree_diffs_get_all_pending",
                source: e,
            })?;

        let mut diffs = HashMap::new();
        for diff in diff_recs {
            let shard = Shard::from(diff.shard as u32);
            let diff = PendingShardStateTreeDiff::try_from(diff)?;
            diffs
                .entry(shard)
                .or_insert_with(Vec::new)//PendingStateTreeDiff::default)
                .push(diff);
        }
        // diffs
        //     .into_iter()
        //     .map(|diff| Ok((Shard::from(diff.shard as u32), diff.try_into()?)))
        //     .collect()
        Ok(diffs)
    }

    fn state_transitions_get_n_after(
        &self,
        n: usize,
        id: StateTransitionId,
        end_epoch: Epoch,
    ) -> Result<Vec<StateTransition>, StorageError> {
        use crate::schema::{state_transitions, substates};

        // Never return epoch 0 state transitions
        let min_epoch = Some(id.epoch().as_u64()).filter(|e| *e > 0).unwrap_or(1) as i64;
        let transitions = state_transitions::table
            .left_join(substates::table.on(state_transitions::substate_address.eq(substates::address)))
            .select((state_transitions::all_columns, substates::all_columns.nullable()))
            .filter(state_transitions::seq.ge(id.seq() as i64))
            .filter(state_transitions::epoch.ge(min_epoch))
            .filter(state_transitions::epoch.lt(end_epoch.as_u64() as i64))
            .filter(state_transitions::shard.eq(id.shard().as_u32() as i32))
            .limit(n as i64)
            .get_results::<(sql_models::StateTransition, Option<sql_models::SubstateRecord>)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "state_transitions_get_n_after",
                source: e,
            })?;

        transitions
            .into_iter()
            .map(|(t, s)| {
                let s = s.ok_or_else(|| StorageError::DataInconsistency {
                    details: format!("substate entry does not exist for transition {}", t.id),
                })?;

                t.try_convert(s)
            })
            .collect()
    }

    fn state_transitions_get_last_id(&self, shard: Shard) -> Result<StateTransitionId, StorageError> {
        use crate::schema::state_transitions;

        let (seq, epoch) = state_transitions::table
            .select((state_transitions::seq, state_transitions::epoch))
            .filter(state_transitions::shard.eq(shard.as_u32() as i32))
            .order_by(state_transitions::epoch.desc())
            .then_order_by(state_transitions::seq.desc())
            .first::<(i64, i64)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "state_transitions_get_last_id",
                source: e,
            })?;

        let epoch = Epoch(epoch as u64);
        let seq = seq as u64;

        Ok(StateTransitionId::new(epoch, shard, seq))
    }

    fn state_tree_nodes_get(&self, shard: Shard, key: &NodeKey) -> Result<Node<Version>, StorageError> {
        use crate::schema::state_tree;

        let node = state_tree::table
            .select(state_tree::node)
            .filter(state_tree::shard.eq(shard.as_u32() as i32))
            .filter(state_tree::key.eq(key.to_string()))
            .filter(state_tree::is_stale.eq(false))
            .first::<String>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "state_tree_nodes_get",
                source: e,
            })?;

        let node = serde_json::from_str::<TreeNode<Version>>(&node).map_err(|e| StorageError::DataInconsistency {
            details: format!("Failed to deserialize state tree node: {}", e),
        })?;

        Ok(node.into_node())
    }

    fn state_tree_versions_get_latest(&self, shard: Shard) -> Result<Option<Version>, StorageError> {
        use crate::schema::state_tree_shard_versions;

        let version = state_tree_shard_versions::table
            .select(state_tree_shard_versions::version)
            .filter(state_tree_shard_versions::shard.eq(shard.as_u32() as i32))
            .order_by(state_tree_shard_versions::version.desc())
            .first::<i64>(self.connection())
            .optional()
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "state_tree_versions_get_latest",
                source: e,
            })?;

        Ok(version.map(|v| v as Version))
    }

    fn epoch_checkpoint_get(&self, epoch: Epoch) -> Result<EpochCheckpoint, StorageError> {
        use crate::schema::epoch_checkpoints;

        let checkpoint = epoch_checkpoints::table
            .filter(epoch_checkpoints::epoch.eq(epoch.as_u64() as i64))
            .first::<sql_models::EpochCheckpoint>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "epoch_checkpoint_get",
                source: e,
            })?;

        checkpoint.try_into()
    }
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub count: i64,
}

#[derive(QueryableByName)]
struct BlockIdSqlValue {
    #[diesel(sql_type = diesel::sql_types::Text)]
    pub bid: String,
}
