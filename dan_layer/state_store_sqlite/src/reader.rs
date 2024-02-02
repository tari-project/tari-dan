//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    ops::RangeInclusive,
};

use bigdecimal::{BigDecimal, ToPrimitive};
use diesel::{
    query_builder::SqlQuery,
    sql_query,
    sql_types::{BigInt, Text},
    BoolExpressionMethods,
    ExpressionMethods,
    JoinOnDsl,
    NullableExpressionMethods,
    QueryDsl,
    QueryableByName,
    RunQueryDsl,
    SqliteConnection,
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::{Epoch, NodeAddressable, NodeHeight, SubstateAddress};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        Command,
        Decision,
        Evidence,
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
        QcId,
        QuorumCertificate,
        SubstateLockFlag,
        SubstateLockState,
        SubstateRecord,
        TransactionPoolRecord,
        TransactionPoolStage,
        TransactionRecord,
        Vote,
    },
    Ordering,
    StateStoreReadTransaction,
    StorageError,
};
use tari_engine_types::lock::LockFlag;
use tari_transaction::TransactionId;
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

impl<'a, TAddr: NodeAddressable + Serialize + DeserializeOwned> SqliteStateStoreReadTransaction<'a, TAddr> {
    pub fn get_transaction_atom_state_updates_between_blocks<'i, ITx>(
        &mut self,
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
        &mut self,
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
        let mut sql_frag = String::with_capacity(len * item_size + len * 3 + len - 1);
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

    fn get_block_ids_between(
        &mut self,
        start_block: &BlockId,
        end_block: &BlockId,
    ) -> Result<Vec<String>, SqliteStorageError> {
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

    fn get_block_ids_that_change_state_between(
        &mut self,
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
}

impl<TAddr: NodeAddressable + Serialize + DeserializeOwned> StateStoreReadTransaction
    for SqliteStateStoreReadTransaction<'_, TAddr>
{
    type Addr = TAddr;

    fn last_voted_get(&mut self) -> Result<LastVoted, StorageError> {
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

    fn last_sent_vote_get(&mut self) -> Result<LastSentVote, StorageError> {
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

    fn last_executed_get(&mut self) -> Result<LastExecuted, StorageError> {
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

    fn last_proposed_get(&mut self) -> Result<LastProposed, StorageError> {
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

    fn locked_block_get(&mut self) -> Result<LockedBlock, StorageError> {
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

    fn leaf_block_get(&mut self) -> Result<LeafBlock, StorageError> {
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

    fn high_qc_get(&mut self) -> Result<HighQc, StorageError> {
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

    fn foreign_proposal_exists(&mut self, foreign_proposal: &ForeignProposal) -> Result<bool, StorageError> {
        use crate::schema::foreign_proposals;

        let foreign_proposals = foreign_proposals::table
            .filter(foreign_proposals::bucket.eq(foreign_proposal.bucket.as_u32() as i32))
            .filter(foreign_proposals::block_id.eq(serialize_hex(foreign_proposal.block_id)))
            .filter(foreign_proposals::transactions.eq(serialize_json(&foreign_proposal.transactions)?))
            .count()
            .limit(1)
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "foreign_proposal_exists",
                source: e,
            })?;

        Ok(foreign_proposals > 0)
    }

    fn foreign_proposal_get_all_new(&mut self) -> Result<Vec<ForeignProposal>, StorageError> {
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
        &mut self,
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

    fn foreign_proposal_get_all_mined(&mut self, to_height: NodeHeight) -> Result<Vec<ForeignProposal>, StorageError> {
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

    fn foreign_send_counters_get(&mut self, block_id: &BlockId) -> Result<ForeignSendCounters, StorageError> {
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

    fn foreign_receive_counters_get(&mut self) -> Result<ForeignReceiveCounters, StorageError> {
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

    fn transactions_get(&mut self, tx_id: &TransactionId) -> Result<TransactionRecord, StorageError> {
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

    fn transactions_exists(&mut self, tx_id: &TransactionId) -> Result<bool, StorageError> {
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
        &mut self,
        tx_ids: I,
    ) -> Result<Vec<TransactionRecord>, StorageError> {
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

    fn transactions_get_paginated(
        &mut self,
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

    fn blocks_get_tip(&mut self) -> Result<Block, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let (block, qc) = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
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

    fn blocks_get_all_between(
        &mut self,
        start_block_id_exclusive: &BlockId,
        end_block_id_inclusive: &BlockId,
        include_dummy_blocks: bool,
    ) -> Result<Vec<Block>, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let mut block_ids = self.get_block_ids_between(start_block_id_exclusive, end_block_id_inclusive)?;
        if block_ids.is_empty() {
            return Ok(vec![]);
        }

        // Exclude start block
        block_ids.pop();

        let mut query = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .filter(blocks::block_id.eq_any(block_ids))
            .into_boxed();

        if !include_dummy_blocks {
            query = query.filter(blocks::is_dummy.eq(false));
        }

        let results = query
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

    fn blocks_get_all_by_parent(&mut self, parent_id: &BlockId) -> Result<Vec<Block>, StorageError> {
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

    fn blocks_get_parent_chain(&mut self, block_id: &BlockId, limit: usize) -> Result<Vec<Block>, StorageError> {
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

    fn blocks_exists(&mut self, block_id: &BlockId) -> Result<bool, StorageError> {
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

    fn blocks_is_ancestor(&mut self, descendant: &BlockId, ancestor: &BlockId) -> Result<bool, StorageError> {
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

    fn blocks_get_pending_transactions(&mut self, block_id: &BlockId) -> Result<Vec<TransactionId>, StorageError> {
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
        &mut self,
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
        &mut self,
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
        &mut self,
        limit: u64,
        offset: u64,
        asc_desc_created_at: Option<Ordering>,
    ) -> Result<Vec<Block>, StorageError> {
        use crate::schema::{blocks, quorum_certificates};

        let mut query = blocks::table
            .left_join(quorum_certificates::table.on(blocks::qc_id.eq(quorum_certificates::qc_id)))
            .select((blocks::all_columns, quorum_certificates::all_columns.nullable()))
            .into_boxed();

        if let Some(ordering) = asc_desc_created_at {
            match ordering {
                Ordering::Ascending => query = query.order_by(blocks::created_at.asc()),
                Ordering::Descending => query = query.order_by(blocks::created_at.desc()),
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

    fn blocks_get_count(&mut self) -> Result<i64, StorageError> {
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

    fn blocks_max_height(&mut self) -> Result<NodeHeight, StorageError> {
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

    fn parked_blocks_exists(&mut self, block_id: &BlockId) -> Result<bool, StorageError> {
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

    fn quorum_certificates_get(&mut self, qc_id: &QcId) -> Result<QuorumCertificate, StorageError> {
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
        &mut self,
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

    fn quorum_certificates_get_by_block_id(&mut self, block_id: &BlockId) -> Result<QuorumCertificate, StorageError> {
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

    fn transactions_fetch_involved_shards(
        &mut self,
        transaction_ids: HashSet<TransactionId>,
    ) -> Result<HashSet<SubstateAddress>, StorageError> {
        use crate::schema::transactions;

        let tx_ids = transaction_ids.into_iter().map(serialize_hex).collect::<Vec<_>>();

        let involved_shards = transactions::table
            .select((transactions::inputs, transactions::input_refs))
            .filter(transactions::transaction_id.eq_any(tx_ids))
            .load::<(String, String)>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pools_fetch_involved_shards",
                source: e,
            })?;

        let shards = involved_shards
            .into_iter()
            .map(|(inputs, input_refs)| {
                (
                    // a Result is very inconvenient with flat_map
                    deserialize_json::<HashSet<SubstateAddress>>(&inputs).unwrap(),
                    deserialize_json::<HashSet<SubstateAddress>>(&input_refs).unwrap(),
                )
            })
            .flat_map(|(inputs, input_refs)| inputs.into_iter().chain(input_refs).collect::<HashSet<_>>())
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

    fn transaction_pool_get(
        &mut self,
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

    fn transaction_pool_exists(&mut self, transaction_id: &TransactionId) -> Result<bool, StorageError> {
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

    fn transaction_pool_get_all(&mut self) -> Result<Vec<TransactionPoolRecord>, StorageError> {
        use crate::schema::transaction_pool;
        let txs = transaction_pool::table
            .get_results::<sql_models::TransactionPoolRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "transaction_pool_get_all",
                source: e,
            })?;
        txs.into_iter().map(|tx| tx.try_convert(None)).collect()
    }

    fn transaction_pool_get_many_ready(&mut self, max_txs: usize) -> Result<Vec<TransactionPoolRecord>, StorageError> {
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

        let mut used_substates = HashMap::<SubstateAddress, LockFlag>::new();
        let mut processed_substates =
            HashMap::<TransactionId, HashSet<(SubstateAddress, _)>>::with_capacity(updates.len());
        for (tx_id, update) in &updates {
            if update.local_decision.as_deref() == Some(Decision::Abort.as_str()) {
                // The aborted transaction don't lock any substates
                continue;
            }
            let evidence = deserialize_json::<Evidence>(&update.evidence)?;
            let evidence = evidence
                .iter()
                .map(|(shard, evidence)| (*shard, evidence.lock))
                .collect::<HashSet<(SubstateAddress, _)>>();
            processed_substates.insert(deserialize_hex_try_from(tx_id)?, evidence);
        }

        ready_txs
            .into_iter()
            .filter_map(|rec| {
                let maybe_update = updates.remove(&rec.transaction_id);
                match rec.try_convert(maybe_update) {
                    Ok(rec) => {
                        if !rec.is_ready() {
                            return None;
                        }

                        let tx_substates = rec
                            .transaction()
                            .evidence
                            .iter()
                            .map(|(shard, evidence)| (*shard, evidence.lock))
                            .collect::<HashMap<_, _>>();

                        // Are there any conflicts between the currently selected set and this transaction?
                        if tx_substates.iter().any(|(shard, lock)| {
                            if lock.is_write() {
                                // Write lock must have no conflicts
                                used_substates.contains_key(shard)
                            } else {
                                // If there is a Shard conflict, then it must not be a write lock
                                used_substates
                                    .get(shard)
                                    .map(|tx_lock| tx_lock.is_write())
                                    .unwrap_or(false)
                            }
                        }) {
                            return None;
                        }

                        // Are there any conflicts between this transaction and other transactions to be included in the
                        // block?
                        if processed_substates
                                .iter()
                                // Check other transactions
                                .filter(|(tx_id, _)| *tx_id != rec.transaction_id())
                                .all(|(_, evidence)| {
                                    evidence.iter().all(|(shard, lock)| {
                                        if lock.is_write() {
                                            // Write lock must have no conflicts
                                            !tx_substates.contains_key(shard)
                                        } else {
                                            // If there is a Shard conflict, then it must be a read lock
                                            tx_substates.get(shard).map(|tx_lock| tx_lock.is_read()).unwrap_or(true)
                                        }
                                    })
                                })
                        {
                            used_substates.extend(tx_substates);
                            Some(Ok(rec))
                        } else {
                            // TODO: If we don't switch to "no version" transaction, then we can abort these here.
                            // That also requires changes to the on_ready_to_vote_on_local_block
                            None
                        }
                    },
                    Err(e) => Some(Err(e)),
                }
            })
            .take(max_txs)
            .collect()
    }

    fn transaction_pool_count(
        &mut self,
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

    fn substates_get(&mut self, address: &SubstateAddress) -> Result<SubstateRecord, StorageError> {
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

    fn substates_get_any(&mut self, addresses: &HashSet<SubstateAddress>) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;

        let substates = substates::table
            .filter(substates::address.eq_any(addresses.iter().map(serialize_hex)))
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_any",
                source: e,
            })?;

        substates.into_iter().map(TryInto::try_into).collect()
    }

    fn substates_any_exist<I: IntoIterator<Item = S>, S: Borrow<SubstateAddress>>(
        &mut self,
        addresses: I,
    ) -> Result<bool, StorageError> {
        use crate::schema::substates;

        let count = substates::table
            .count()
            .filter(substates::address.eq_any(addresses.into_iter().map(|s| serialize_hex(s.borrow()))))
            .limit(1)
            .get_result::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_any",
                source: e,
            })?;

        Ok(count > 0)
    }

    fn substates_exists_for_transaction(&mut self, transaction_id: &TransactionId) -> Result<bool, StorageError> {
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

    fn substates_get_many_within_range(
        &mut self,
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
        &mut self,
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
        &mut self,
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

    fn substates_get_all_for_block(&mut self, block_id: &BlockId) -> Result<Vec<SubstateRecord>, StorageError> {
        use crate::schema::substates;

        let block_id_hex = serialize_hex(block_id);

        let substates = substates::table
            .filter(
                substates::created_block
                    .eq(&block_id_hex)
                    .or(substates::destroyed_by_block.eq(Some(&block_id_hex))),
            )
            .get_results::<sql_models::SubstateRecord>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "substates_get_all_for_block",
                source: e,
            })?;

        let substates = substates
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(substates)
    }

    fn substates_get_all_for_transaction(
        &mut self,
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

    fn substates_check_lock_many<'a, I: IntoIterator<Item = &'a SubstateAddress>>(
        &mut self,
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

        Ok(SubstateLockState::LockAcquired)
    }

    // -------------------------------- LockedOutputs -------------------------------- //
    fn locked_outputs_check_all<I, B>(&mut self, output_addresses: I) -> Result<SubstateLockState, StorageError>
    where
        I: IntoIterator<Item = B>,
        B: Borrow<SubstateAddress>,
    {
        use crate::schema::locked_outputs;

        let outputs_hex = output_addresses
            .into_iter()
            .map(|address| serialize_hex(address.borrow()))
            .collect::<Vec<_>>();

        let has_conflict = locked_outputs::table
            .count()
            .filter(locked_outputs::substate_address.eq_any(outputs_hex))
            .first::<i64>(self.connection())
            .map_err(|e| SqliteStorageError::DieselError {
                operation: "locked_outputs_check_all",
                source: e,
            })?;

        if has_conflict > 0 {
            Ok(SubstateLockState::SomeAlreadyWriteLocked)
        } else {
            Ok(SubstateLockState::LockAcquired)
        }
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
