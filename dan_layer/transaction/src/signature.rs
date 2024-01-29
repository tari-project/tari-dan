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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionSignatureFields {
    pub fee_instructions: Vec<Instruction>,
    pub instructions: Vec<Instruction>,
    pub inputs: Vec<SubstateAddress>,
    pub input_refs: Vec<SubstateAddress>,
    pub min_epoch: Option<Epoch>,
    pub max_epoch: Option<Epoch>,
}

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

    pub fn sign(secret_key: &RistrettoSecretKey, fields: TransactionSignatureFields) -> Self {
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        let challenge = Self::create_challenge(fields);

        Self {
            signature: Signature::sign(secret_key, challenge, &mut OsRng).unwrap(),
            public_key,
        }
    }

    pub fn verify(&self, fields: TransactionSignatureFields) -> bool {
        let challenge = Self::create_challenge(fields);
        self.signature.verify(&self.public_key, challenge)
    }

    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    pub fn public_key(&self) -> &RistrettoPublicKey {
        &self.public_key
    }

    fn create_challenge(fields: TransactionSignatureFields) -> [u8; 64] {
        hasher64(EngineHashDomainLabel::TransactionSignature)
            .chain(&fields)
            .result()
    }
}
