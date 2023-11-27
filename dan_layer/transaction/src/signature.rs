//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{PublicKey, Signature};
use tari_crypto::{
    keys::PublicKey as PublicKeyT,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_dan_common_types::{Epoch, ShardId};
use tari_engine_types::{
    hashing::{hasher64, EngineHashDomainLabel},
    instruction::Instruction,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionSignatureFields {
    pub fee_instructions: Vec<Instruction>,
    pub instructions: Vec<Instruction>,
    pub inputs: Vec<ShardId>,
    pub input_refs: Vec<ShardId>,
    pub min_epoch: Option<Epoch>,
    pub max_epoch: Option<Epoch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct TransactionSignature {
    public_key: PublicKey,
    signature: Signature,
}

impl TransactionSignature {
    pub fn new(public_key: PublicKey, signature: Signature) -> Self {
        Self { public_key, signature }
    }

    pub fn sign(secret_key: &RistrettoSecretKey, fields: TransactionSignatureFields) -> Self {
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        let challenge = hasher64(EngineHashDomainLabel::TransactionSignature)
            .chain(&fields)
            .result();

        Self {
            signature: Signature::sign(secret_key, challenge, &mut OsRng).unwrap(),
            public_key,
        }
    }

    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    pub fn public_key(&self) -> &RistrettoPublicKey {
        &self.public_key
    }
}
