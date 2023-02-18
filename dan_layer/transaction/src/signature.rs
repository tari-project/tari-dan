//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::convert::TryFrom;

use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use tari_crypto::{
    keys::PublicKey as PublicKeyT,
    ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
};
use tari_engine_types::{hashing::hasher, instruction::Instruction};

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct InstructionSignature(RistrettoSchnorr);

impl InstructionSignature {
    pub fn sign_with_nonce(
        secret_key: &RistrettoSecretKey,
        secret_nonce: RistrettoSecretKey,
        instructions: &[Instruction],
    ) -> Self {
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        let nonce_pk = RistrettoPublicKey::from_secret_key(&secret_nonce);
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        let nonce_pk = RistrettoPublicKey::from_secret_key(&secret_nonce);
        // TODO: implement dan encoding for (a wrapper of) PublicKey
        let challenge = hasher("instruction-signature")
            .chain(&nonce_pk)
            .chain(&public_key)
            .chain(instructions)
            .result();
        Self(RistrettoSchnorr::sign_raw(secret_key, secret_nonce, &challenge).unwrap())
    }

    pub fn sign(secret_key: &RistrettoSecretKey, instructions: &[Instruction]) -> Self {
        let (secret_nonce, _nonce_pk) = RistrettoPublicKey::random_keypair(&mut OsRng);
        Self::sign_with_nonce(secret_key, secret_nonce, instructions)
    }

    pub fn signature(&self) -> RistrettoSchnorr {
        self.0.clone()
    }
}

impl TryFrom<RistrettoSchnorr> for InstructionSignature {
    type Error = String;

    fn try_from(sig: RistrettoSchnorr) -> Result<Self, Self::Error> {
        Ok(InstructionSignature(sig))
    }
}
