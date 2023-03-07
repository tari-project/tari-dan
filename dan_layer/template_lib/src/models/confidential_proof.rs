//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};

use crate::{
    crypto::{BalanceProofSignature, RistrettoPublicKeyBytes},
    models::Amount,
};

#[derive(Debug, Clone, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ConfidentialOutputProof {
    pub output_statement: ConfidentialStatement,
    pub change_statement: Option<ConfidentialStatement>,
    #[cfg_attr(features = "serde", serde(with = "hex::serde"))]
    pub range_proof: Vec<u8>,
    pub revealed_amount: Amount,
}

#[derive(Debug, Clone, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ConfidentialStatement {
    #[cfg_attr(features = "serde", serde(with = "hex::serde"))]
    pub commitment: [u8; 32],
    /// Public nonce (R) that was used to generate the commitment mask
    pub sender_public_nonce: Option<RistrettoPublicKeyBytes>,
    /// Commitment value encrypted for the receiver. Without this it would be difficult (not impossible) for the
    /// receiver to determine the value component of the commitment.
    pub encrypted_value: EncryptedValue,
    pub minimum_value_promise: u64,
}

#[derive(Debug, Clone, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct ConfidentialWithdrawProof {
    #[cfg_attr(features = "serde", serde(with = "hex::serde"))]
    pub inputs: Vec<[u8; 32]>,
    pub output_proof: ConfidentialOutputProof,
    /// Balance proof
    pub balance_proof: BalanceProofSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode, Default)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub struct EncryptedValue(#[cfg_attr(features = "serde", serde(with = "hex::serde"))] pub [u8; EncryptedValue::size()]);

impl EncryptedValue {
    pub const fn size() -> usize {
        24
    }
}
