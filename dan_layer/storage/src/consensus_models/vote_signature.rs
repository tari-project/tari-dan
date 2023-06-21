//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_core::transactions::transaction_components::ValidatorNodeHashDomain;
use tari_crypto::{keys::PublicKey as _, signatures::SchnorrSignature};

pub type ValidatorSchnorrSignature = SchnorrSignature<PublicKey, PrivateKey, ValidatorNodeHashDomain>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ValidatorSignature {
    pub public_key: PublicKey,
    pub signature: ValidatorSchnorrSignature,
}

impl ValidatorSignature {
    pub fn new(public_key: PublicKey, signature: ValidatorSchnorrSignature) -> Self {
        Self { public_key, signature }
    }

    pub fn sign<M: AsRef<[u8]>>(secret_key: &PrivateKey, message: M) -> Self {
        let signature =
            ValidatorSchnorrSignature::sign_message(secret_key, message).expect("sign_message is infallible");
        let public_key = PublicKey::from_secret_key(secret_key);
        Self::new(public_key, signature)
    }

    pub fn verify<M: AsRef<[u8]>>(&self, message: M) -> bool {
        self.signature.verify_message(&self.public_key, message)
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.public_key
    }
}
