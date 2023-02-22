//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};

use crate::crypto::RistrettoPublicKeyBytes;

#[derive(Debug, Clone, Encode, Decode)]
pub struct ConfidentialProof {
    /// The public mask of the commitment
    pub public_mask: RistrettoPublicKeyBytes,
    /// The commitment being proven
    pub commitment: [u8; 32],
    /// Proof of knowledge of each element in the commitment (value, mask, asset tag). Currently packed Schnorr
    /// signatures <R, u, v>.
    pub knowledge_proof: Vec<u8>,
    /// Proof that no elements are negative
    pub range_proof: Vec<u8>,
    pub minimum_value_promise: u64,
}
