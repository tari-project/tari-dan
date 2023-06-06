//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, ops::DerefMut};

use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::ShardId;
use tari_engine_types::{commit_result::ExecuteResult, instruction::Instruction};
use tari_transaction::{InstructionSignature, TransactionMeta};

use crate::{
    consensus_models::{Decision, TransactionId},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub hash: TransactionId,
    pub fee_instructions: Vec<Instruction>,
    pub instructions: Vec<Instruction>,
    pub signature: InstructionSignature,
    pub sender_public_key: PublicKey,
    pub meta: TransactionMeta,
}

impl Transaction {
    pub fn get<TTx>(tx: &mut TTx, tx_id: &TransactionId) -> Result<Self, StorageError>
    where
        TTx: DerefMut,
        TTx::Target: StateStoreReadTransaction,
    {
        tx.deref_mut().transactions_get(tx_id)
    }
}

impl Transaction {
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

    pub fn involved_shards(&self) -> HashSet<ShardId> {
        self.meta
            .involved_shards()
            .into_iter()
            .chain(
                self.meta
                    .required_inputs_iter()
                    .map(|a| ShardId::from_address(a.address(), a.version().unwrap_or(0))),
            )
            .collect()
    }

    pub fn meta(&self) -> &TransactionMeta {
        &self.meta
    }
}

pub struct ExecutedTransaction {
    pub transaction: Transaction,
    pub result: ExecuteResult,
}

impl ExecutedTransaction {
    pub fn decision(&self) -> Decision {
        if self.result.finalize.is_accept() {
            Decision::Accept
        } else {
            Decision::Reject
        }
    }
}

impl ExecutedTransaction {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.transactions_insert(self)
    }
}
