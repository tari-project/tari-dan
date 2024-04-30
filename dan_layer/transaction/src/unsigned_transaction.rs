//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use indexmap::IndexSet;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::Epoch;
use tari_engine_types::instruction::Instruction;

use crate::{SubstateRequirement, Transaction, VersionedSubstateId};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct UnsignedTransaction {
    pub fee_instructions: Vec<Instruction>,
    pub instructions: Vec<Instruction>,
    /// Input objects that may be downed by this transaction
    pub inputs: IndexSet<SubstateRequirement>,
    /// Inputs filled by some authority. These are not part of the transaction hash nor the signature
    pub filled_inputs: IndexSet<VersionedSubstateId>,
    pub min_epoch: Option<Epoch>,
    pub max_epoch: Option<Epoch>,
}

impl From<&Transaction> for UnsignedTransaction {
    fn from(tx: &Transaction) -> Self {
        Self {
            fee_instructions: tx.fee_instructions().to_vec(),
            instructions: tx.instructions().to_vec(),
            inputs: tx.inputs().clone(),
            filled_inputs: tx.filled_inputs().clone(),
            min_epoch: tx.min_epoch(),
            max_epoch: tx.max_epoch(),
        }
    }
}
