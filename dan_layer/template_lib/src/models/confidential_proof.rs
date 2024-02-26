//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use serde_with::{serde_as, Bytes};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{
    crypto::{BalanceProofSignature, PedersonCommitmentBytes, RistrettoPublicKeyBytes},
    models::Amount,
};

/// A zero-knowledge proof of a confidential transfer
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialOutputProof {
    /// Proof of the confidential resources that are going to be transferred to the receiver
    pub output_statement: Option<ConfidentialStatement>,
    /// Proof of the transaction change, which goes back to the sender's vault
    pub change_statement: Option<ConfidentialStatement>,
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    /// Needed to prove that no coins were created
    pub range_proof: Vec<u8>,
    pub output_revealed_amount: Amount,
    pub change_revealed_amount: Amount,
}

impl ConfidentialOutputProof {
    /// Creates an output proof for minting which only mints a revealed amount.
    pub fn mint_revealed(amount: Amount) -> Self {
        Self {
            output_statement: None,
            change_statement: None,
            range_proof: vec![],
            output_revealed_amount: amount,
            change_revealed_amount: Amount::zero(),
        }
    }
}

/// A zero-knowledge proof that a confidential resource amount is valid
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialStatement {
    #[serde_as(as = "Bytes")]
    pub commitment: [u8; 32],
    /// Public nonce (R) that was used to generate the commitment mask
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub sender_public_nonce: RistrettoPublicKeyBytes,
    /// Encrypted mask and value for the recipient.
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub encrypted_data: EncryptedData,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub minimum_value_promise: u64,
}

/// A zero-knowledge proof that a withdrawal of confidential resources from a vault is valid
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ConfidentialWithdrawProof {
    // #[cfg_attr(feature = "hex", serde(with = "hex::serde"))]
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub inputs: Vec<PedersonCommitmentBytes>,
    /// The amount to withdraw from revealed funds i.e. the revealed funds as inputs
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub input_revealed_amount: Amount,
    pub output_proof: ConfidentialOutputProof,
    /// Balance proof
    #[cfg_attr(feature = "ts", ts(type = "Array<number>"))]
    pub balance_proof: BalanceProofSignature,
}

impl ConfidentialWithdrawProof {
    /// Creates a withdrawal proof for revealed funds of a specific amount
    pub fn revealed_withdraw(amount: Amount) -> Self {
        // There are no confidential inputs or outputs (this amounts to the same thing as a Fungible resource transfer)
        // So signature s = 0 + e.x where x is a 0 excess, is valid.
        let balance_proof = BalanceProofSignature::try_from_parts(&[0u8; 32], &[0u8; 32]).unwrap();

        Self {
            inputs: vec![],
            input_revealed_amount: amount,
            output_proof: ConfidentialOutputProof::mint_revealed(amount),
            balance_proof,
        }
    }

    pub fn revealed_input_amount(&self) -> Amount {
        self.input_revealed_amount
    }

    pub fn revealed_output_amount(&self) -> Amount {
        self.output_proof.output_revealed_amount
    }

    pub fn revealed_change_amount(&self) -> Amount {
        self.output_proof.change_revealed_amount
    }
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
