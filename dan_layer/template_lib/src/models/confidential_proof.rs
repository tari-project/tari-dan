//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};

use crate::{crypto::BalanceProofSignature, models::Amount};

#[derive(Debug, Clone, Encode, Decode)]
pub struct ConfidentialProof {
    pub output_statement: ConfidentialStatement,
    pub change_statement: Option<ConfidentialStatement>,
    pub range_proof: Vec<u8>,
    pub revealed_amount: Amount,
}

#[derive(Debug, Clone, Default, Encode, Decode)]
pub struct ConfidentialStatement {
    pub commitment: [u8; 32],
    pub minimum_value_promise: u64,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct ConfidentialWithdrawProof {
    pub output_proof: ConfidentialProof,
    /// Balance proof
    pub balance_proof: BalanceProofSignature,
}
