//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::{Deref, DerefMut};

use diesel::{AsChangeset, ExpressionMethods, QueryDsl, RunQueryDsl, SqliteConnection};
use log::*;
use tari_dan_common_types::ShardId;
use tari_dan_storage::{
    consensus_models::{
        Block,
        Evidence,
        ExecutedTransaction,
        HighQc,
        LastExecuted,
        LastVoted,
        LeafBlock,
        LockedBlock,
        QuorumCertificate,
        SubstateLockFlag,
        TransactionAtom,
        TransactionId,
        TransactionPoolStage,
        Vote,
    },
    StateStoreWriteTransaction,
    StorageError,
};
use tari_utilities::ByteArray;

use crate::{
    error::SqliteStorageError,
    reader::SqliteStateStoreReadTransaction,
    serialization::{serialize_hex, serialize_json},
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

    fn transactions_insert(&mut self, executed_transaction: &ExecutedTransaction) -> Result<(), StorageError> {
        use crate::schema::transactions;

        let transaction = executed_transaction.transaction();
        let result = executed_transaction.result();

        let insert = (
            transactions::transaction_id.eq(serialize_hex(transaction.hash())),
            transactions::fee_instructions.eq(serialize_json(transaction.fee_instructions())?),
            transactions::instructions.eq(serialize_json(transaction.instructions())?),
            transactions::result.eq(serialize_json(result)?),
            transactions::signature.eq(serialize_json(transaction.signature())?),
            transactions::sender_public_key.eq(serialize_hex(transaction.sender_public_key().as_bytes())),
            transactions::inputs.eq(serialize_json(transaction.inputs())?),
            transactions::input_refs.eq(serialize_json(transaction.input_refs())?),
            transactions::outputs.eq(serialize_json(transaction.outputs())?),
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

    fn transaction_pool_insert(
        &mut self,
        transaction: TransactionAtom,
        stage: TransactionPoolStage,
        is_ready: bool,
    ) -> Result<(), StorageError> {
        use crate::schema::transaction_pool;

        let insert = (
            transaction_pool::transaction_id.eq(serialize_hex(transaction.id)),
            transaction_pool::involved_shards.eq(serialize_json(&transaction.involved_shards)?),
            transaction_pool::overall_decision.eq(transaction.decision.to_string()),
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
        is_ready: Option<bool>,
    ) -> Result<(), StorageError> {
        use crate::schema::transaction_pool;

        #[derive(AsChangeset)]
        #[diesel(table_name=transaction_pool)]
        struct Changes {
            evidence: Option<String>,
            stage: Option<String>,
            is_ready: Option<bool>,
        }

        let change_set = Changes {
            evidence: evidence.map(serialize_json).transpose()?,
            stage: stage.map(|s| s.to_string()),
            is_ready,
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
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<(), StorageError> {
        use crate::schema::substates;

        let objects: Vec<String> = objects.into_iter().map(serialize_hex).collect();

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
                    "Found {} substates, but {} were requested",
                    locked_w.len(),
                    objects.len()
                ),
            }
            .into());
        }
        if locked_w.iter().any(|w| *w) {
            return Err(SqliteStorageError::SubstatesWriteLocked {
                operation: "substates_try_lock_many",
            }
            .into());
        }

        match lock_flag {
            SubstateLockFlag::Write => {
                diesel::update(substates::table)
                    .filter(substates::shard_id.eq_any(objects))
                    .set(substates::is_locked_w.eq(true))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "transactions_try_lock_many",
                        source: e,
                    })?;
            },
            SubstateLockFlag::Read => {
                diesel::update(substates::table)
                    .filter(substates::shard_id.eq_any(objects))
                    .set(substates::read_locks.eq(substates::read_locks + 1))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "transactions_try_lock_many",
                        source: e,
                    })?;
            },
        }

        Ok(())
    }

    fn substates_try_unlock_many<'a, I: IntoIterator<Item = &'a ShardId>>(
        &mut self,
        objects: I,
        lock_flag: SubstateLockFlag,
    ) -> Result<(), StorageError> {
        use crate::schema::substates;

        let objects: Vec<String> = objects.into_iter().map(serialize_hex).collect();

        match lock_flag {
            SubstateLockFlag::Write => {
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
                            "Found {} substates, but {} were requested",
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
                    .set(substates::is_locked_w.eq(false))
                    .execute(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "transactions_try_unlock_many",
                        source: e,
                    })?;
            },
            SubstateLockFlag::Read => {
                let locked_r = substates::table
                    .select(substates::read_locks)
                    .filter(substates::shard_id.eq_any(&objects))
                    .get_results::<i32>(self.connection())
                    .map_err(|e| SqliteStorageError::DieselError {
                        operation: "transactions_try_lock_many",
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
                        operation: "transactions_try_unlock_many",
                        source: e,
                    })?;
            },
        }

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
