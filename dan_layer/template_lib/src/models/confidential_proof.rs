//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};

use crate::{
    crypto::{BalanceProofSignature, RistrettoPublicKeyBytes},
    models::Amount,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidentialOutputProof {
    pub output_statement: ConfidentialStatement,
    pub change_statement: Option<ConfidentialStatement>,
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    pub range_proof: Vec<u8>,
    pub revealed_amount: Amount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidentialStatement {
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    pub commitment: [u8; 32],
    /// Public nonce (R) that was used to generate the commitment mask
    pub sender_public_nonce: Option<RistrettoPublicKeyBytes>,
    /// Commitment value encrypted for the receiver. Without this it would be difficult (not impossible) for the
    /// receiver to determine the value component of the commitment.
    pub encrypted_value: EncryptedValue,
    pub minimum_value_promise: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidentialWithdrawProof {
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    pub inputs: Vec<[u8; 32]>,
    pub output_proof: ConfidentialOutputProof,
    /// Balance proof
    pub balance_proof: BalanceProofSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EncryptedValue(
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    pub [u8; EncryptedValue::size()],
);

impl EncryptedValue {
    pub const fn size() -> usize {
        24
    }
}
