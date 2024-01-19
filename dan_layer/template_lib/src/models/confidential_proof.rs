//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, Bytes};

use crate::{
    crypto::{BalanceProofSignature, PedersonCommitmentBytes, RistrettoPublicKeyBytes},
    models::Amount,
};

/// A zero-knowledge proof of a confidential transfer
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfidentialOutputProof {
    /// Proof of the confidential resources that are going to be transferred to the receiver
    pub output_statement: ConfidentialStatement,
    /// Proof of the transaction change, which goes back to the sender's vault
    pub change_statement: Option<ConfidentialStatement>,
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    /// Needed to prove that no coins were created
    pub range_proof: Vec<u8>,
}

/// A zero-knowledge proof that a confidential resource amount is valid
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfidentialStatement {
    #[serde_as(as = "Bytes")]
    pub commitment: [u8; 32],
    /// Public nonce (R) that was used to generate the commitment mask
    // #[cfg_attr(feature = "serde", serde(with = "hex::serde"))]
    pub sender_public_nonce: RistrettoPublicKeyBytes,
    /// Commitment value encrypted for the receiver. Without this it would be difficult (not impossible) for the
    /// receiver to determine the value component of the commitment.
    pub encrypted_data: EncryptedData,
    pub minimum_value_promise: u64,
    pub revealed_amount: Amount,
}

/// A zero-knowledge proof that a withdrawal of confidential resources from a vault is valid
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfidentialWithdrawProof {
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    pub inputs: Vec<PedersonCommitmentBytes>,
    pub output_proof: ConfidentialOutputProof,
    /// Balance proof
    pub balance_proof: BalanceProofSignature,
}

/// Used by the receiver to determine the value component of the commitment, in both confidential transfers and Minotari
/// burns
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EncryptedData(#[serde_as(as = "Bytes")] pub [u8; EncryptedData::size()]);

impl EncryptedData {
    pub const fn size() -> usize {
        80
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for EncryptedData {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Default for EncryptedData {
    fn default() -> Self {
        Self([0u8; Self::size()])
    }
}

impl TryFrom<&[u8]> for EncryptedData {
    type Error = ();

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() != Self::size() {
            return Err(());
        }
        let mut out = [0_u8; Self::size()];
        out.copy_from_slice(value);
        Ok(Self(out))
    }
}
