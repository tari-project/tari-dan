//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{PublicKey, Signature};
use tari_crypto::{
    keys::PublicKey as PublicKeyT,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_dan_common_types::{Epoch, SubstateAddress};
use tari_engine_types::{
    hashing::{hasher64, EngineHashDomainLabel},
    instruction::Instruction,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::unsigned_transaction::UnsignedTransaction;

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionSignature {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    public_key: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    signature: Signature,
}

impl TransactionSignature {
    pub fn new(public_key: PublicKey, signature: Signature) -> Self {
        Self { public_key, signature }
    }

    pub fn sign(secret_key: &RistrettoSecretKey, transaction: &UnsignedTransaction) -> Self {
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        let challenge = Self::create_challenge(transaction);

        Self {
            signature: Signature::sign(secret_key, challenge, &mut OsRng).unwrap(),
            public_key,
        }
    }

    pub fn verify(&self, transaction: &UnsignedTransaction) -> bool {
        let challenge = Self::create_challenge(transaction);
        self.signature.verify(&self.public_key, challenge)
    }

    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    pub fn public_key(&self) -> &RistrettoPublicKey {
        &self.public_key
    }

    fn create_challenge(transaction: &UnsignedTransaction) -> [u8; 64] {
        let signature_fields = TransactionSignatureFields::from(transaction);
        hasher64(EngineHashDomainLabel::TransactionSignature)
            .chain(&signature_fields)
            .result()
    }
}

#[derive(Debug, Clone, Serialize)]
struct TransactionSignatureFields<'a> {
    fee_instructions: &'a [Instruction],
    instructions: &'a [Instruction],
    inputs: &'a [SubstateAddress],
    input_refs: &'a [SubstateAddress],
    min_epoch: Option<Epoch>,
    max_epoch: Option<Epoch>,
}

impl<'a> From<&'a UnsignedTransaction> for TransactionSignatureFields<'a> {
    fn from(transaction: &'a UnsignedTransaction) -> Self {
        Self {
            fee_instructions: &transaction.fee_instructions,
            instructions: &transaction.instructions,
            inputs: &transaction.inputs,
            input_refs: &transaction.input_refs,
            min_epoch: transaction.min_epoch,
            max_epoch: transaction.max_epoch,
        }
    }
}
