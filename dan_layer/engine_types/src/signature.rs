//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::convert::TryFrom;

use serde::{Deserialize, Serialize};
use tari_crypto::{
    keys::PublicKey as PublicKeyT,
    ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
};
use tari_utilities::ByteArray;

use crate::{hashing::hasher, instruction::Instruction};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionSignature(RistrettoSchnorr);

impl InstructionSignature {
    pub fn sign(
        secret_key: &RistrettoSecretKey,
        secret_nonce: RistrettoSecretKey,
        instructions: &[Instruction],
    ) -> Self {
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        let nonce_pk = RistrettoPublicKey::from_secret_key(&secret_nonce);
        // TODO: implement dan encoding for (a wrapper of) PublicKey
        let challenge = hasher("instruction-signature")
            .chain(nonce_pk.as_bytes())
            .chain(public_key.as_bytes())
            .chain(instructions)
            .result();
        Self(RistrettoSchnorr::sign_raw(secret_key, secret_nonce, &challenge).unwrap())
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
