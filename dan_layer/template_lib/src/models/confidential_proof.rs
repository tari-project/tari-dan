//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};

use crate::{
    crypto::{BalanceProofSignature, RistrettoPublicKeyBytes},
    models::Amount,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfidentialOutputProof {
    pub output_statement: ConfidentialStatement,
    pub change_statement: Option<ConfidentialStatement>,
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    pub range_proof: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfidentialStatement {
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    pub commitment: [u8; 32],
    /// Public nonce (R) that was used to generate the commitment mask
    // #[cfg_attr(feature = "serde", serde(with = "hex::serde"))]
    pub sender_public_nonce: RistrettoPublicKeyBytes,
    /// Commitment value encrypted for the receiver. This enables the receiver to determine the value component of the
    /// commitment.
    // #[cfg_attr(feature = "serde", serde(with = "hex::serde"))]
    pub encrypted_data: EncryptedData,
    pub minimum_value_promise: u64,
    pub revealed_amount: Amount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfidentialWithdrawProof {
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    pub inputs: Vec<[u8; 32]>,
    pub output_proof: ConfidentialOutputProof,
    /// Balance proof
    pub balance_proof: BalanceProofSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EncryptedData(
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    #[serde(with = "serde_big_array::BigArray")] pub [u8; EncryptedData::size()],
);

impl EncryptedData {
    pub const fn size() -> usize {
        80
    }
}

impl AsRef<[u8]> for EncryptedData {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
