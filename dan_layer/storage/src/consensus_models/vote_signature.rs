//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_core::transactions::transaction_components::ValidatorNodeHashDomain;
use tari_crypto::{keys::PublicKey as _, signatures::SchnorrSignature};
#[cfg(feature = "ts")]
use ts_rs::TS;

pub type ValidatorSchnorrSignature = SchnorrSignature<PublicKey, PrivateKey, ValidatorNodeHashDomain>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ValidatorSignature {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "{public_nonce : string, signature: string}"))]
    pub signature: ValidatorSchnorrSignature,
}

impl ValidatorSignature {
    pub fn new(public_key: PublicKey, signature: ValidatorSchnorrSignature) -> Self {
        Self { public_key, signature }
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.public_key
    }
}

impl ValidatorSignature {
    pub fn sign<M: AsRef<[u8]>>(secret_key: &PrivateKey, message: M) -> Self {
        let signature =
            ValidatorSchnorrSignature::sign(secret_key, message, &mut OsRng).expect("sign_message is infallible");
        let public_key = PublicKey::from_secret_key(secret_key);
        Self::new(public_key, signature)
    }

    pub fn verify<M: AsRef<[u8]>>(&self, message: M) -> bool {
        self.signature.verify(&self.public_key, message)
    }
}
