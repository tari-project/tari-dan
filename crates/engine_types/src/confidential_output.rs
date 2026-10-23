//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{Deserialize, Serialize};

use crate::crypto::OutputBody;

/// A confidential output stored as its own substate, referenced by a vault's commitment list.
///
/// Spend authorization is entirely controlled by the owning vault's access rules (unlike a stealth
/// [`crate::utxo::Utxo`], which carries its own spend authorization), so no per-output spend key is held here.
/// Spending or burning downs this substate; the only in-place mutation is freezing.
#[derive(
    Debug, Clone, minicbor::Encode, minicbor::Decode, minicbor::CborLen, Serialize, Deserialize, borsh::BorshSerialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ConfidentialOutput {
    #[n(0)]
    pub output: OutputBody,
    #[n(1)]
    pub is_frozen: bool,
}

impl ConfidentialOutput {
    pub fn new(output: OutputBody) -> Self {
        Self {
            output,
            is_frozen: false,
        }
    }

    pub fn output(&self) -> &OutputBody {
        &self.output
    }

    pub fn into_output(self) -> OutputBody {
        self.output
    }

    pub fn freeze(&mut self) {
        self.is_frozen = true;
    }

    pub fn unfreeze(&mut self) {
        self.is_frozen = false;
    }

    pub fn is_frozen(&self) -> bool {
        self.is_frozen
    }
}
