//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};

#[derive(Debug, Clone, Encode, Decode)]
pub struct ConfidentialProof {
    /// The commitment being proven
    pub commitment: [u8; 32],
    /// Proof that no elements are negative
    pub range_proof: Vec<u8>,
    pub minimum_value_promise: u64,
}
