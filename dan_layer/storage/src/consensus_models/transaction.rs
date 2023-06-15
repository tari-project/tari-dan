//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::ShardId;
use tari_engine_types::{commit_result::ExecuteResult, instruction::Instruction};
use tari_transaction::InstructionSignature;

use crate::{
    consensus_models::{Decision, TransactionId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    hash: TransactionId,
    fee_instructions: Vec<Instruction>,
    instructions: Vec<Instruction>,
    signature: InstructionSignature,
    sender_public_key: PublicKey,

    inputs: Vec<ShardId>,
    outputs: Vec<ShardId>,
}

impl Transaction {
    pub fn new(
        hash: TransactionId,
        fee_instructions: Vec<Instruction>,
        instructions: Vec<Instruction>,
        signature: InstructionSignature,
        sender_public_key: PublicKey,
        inputs: Vec<ShardId>,
        outputs: Vec<ShardId>,
    ) -> Self {
        Self {
            hash,
            fee_instructions,
            instructions,
            signature,
            sender_public_key,
            inputs,
            outputs,
        }
    }

    pub fn hash(&self) -> &TransactionId {
        &self.hash
    }

    pub fn fee_instructions(&self) -> &[Instruction] {
        &self.fee_instructions
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub fn signature(&self) -> &InstructionSignature {
        &self.signature
    }

    pub fn sender_public_key(&self) -> &PublicKey {
        &self.sender_public_key
    }

    pub fn involved_shards_iter(&self) -> impl Iterator<Item = &ShardId> + '_ {
        self.inputs().iter().chain(self.outputs())
    }

    pub fn outputs(&self) -> &[ShardId] {
        &self.outputs
    }

    pub fn inputs(&self) -> &[ShardId] {
        &self.inputs
    }
}

pub struct ExecutedTransaction {
    transaction: Transaction,
    result: ExecuteResult,
}

impl ExecutedTransaction {
    pub fn new(transaction: Transaction, result: ExecuteResult) -> Self {
        Self { transaction, result }
    }

    pub fn decision(&self) -> Decision {
        if self.result.finalize.is_accept() {
            Decision::Accept
        } else {
            Decision::Reject
        }
    }

    pub fn transaction_decision(&self) -> Decision {
        if self.result.transaction_failure.is_none() {
            Decision::Accept
        } else {
            Decision::Reject
        }
    }

    pub fn transaction(&self) -> &Transaction {
        &self.transaction
    }

    pub fn result(&self) -> &ExecuteResult {
        &self.result
    }
}

impl ExecutedTransaction {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_insert(self)
    }

    pub fn get<TTx: StateStoreReadTransaction>(tx: &mut TTx, tx_id: &TransactionId) -> Result<Self, StorageError> {
        tx.transactions_get(tx_id)
    }

    pub fn get_many<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        tx_ids: I,
    ) -> Result<Vec<Self>, StorageError> {
        tx.transactions_get_many(tx_ids)
    }

    pub fn get_involved_shards<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
        tx: &mut TTx,
        transactions: I,
    ) -> Result<HashMap<TransactionId, HashSet<ShardId>>, StorageError> {
        let transactions = Self::get_many(tx, transactions)?;
        Ok(transactions
            .into_iter()
            .map(|t| {
                (
                    t.transaction.hash,
                    t.transaction.involved_shards_iter().copied().collect(),
                )
            })
            .collect())
    }
}
