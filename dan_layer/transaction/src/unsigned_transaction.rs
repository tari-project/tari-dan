//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_dan_common_types::Epoch;
use tari_engine_types::instruction::Instruction;

use crate::{SubstateRequirement, Transaction};

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
    pub inputs: Vec<SubstateRequirement>,
    /// Input objects that must exist but cannot be downed by this transaction
    pub input_refs: Vec<SubstateRequirement>,
    /// Inputs filled by some authority. These are not part of the transaction hash nor the signature
    pub filled_inputs: Vec<SubstateRequirement>,
    pub min_epoch: Option<Epoch>,
    pub max_epoch: Option<Epoch>,
}

impl From<&Transaction> for UnsignedTransaction {
    fn from(tx: &Transaction) -> Self {
        Self {
            fee_instructions: tx.fee_instructions().to_vec(),
            instructions: tx.instructions().to_vec(),
            inputs: tx.inputs().to_vec(),
            input_refs: tx.input_refs().to_vec(),
            filled_inputs: tx.filled_inputs().to_vec(),
            min_epoch: tx.min_epoch(),
            max_epoch: tx.max_epoch(),
        }
    }
}
