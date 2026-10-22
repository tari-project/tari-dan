//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_common_types::types::PrivateKey;
use tari_crypto::{ristretto::RistrettoPublicKey, signatures::SchnorrSignature};
use tari_sidechain::hashing::ValidatorNodeHashDomain;
use tari_template_lib::types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};

pub type ValidatorSchnorrSignature = SchnorrSignature<RistrettoPublicKey, PrivateKey, ValidatorNodeHashDomain>;

#[derive(Clone, Debug, Hash, Deserialize, Serialize, BorshSerialize, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ValidatorSignatureBytes {
    #[n(0)]
    pub public_key: RistrettoPublicKeyBytes,
    #[n(1)]
    pub signature: SchnorrSignatureBytes,
}

impl ValidatorSignatureBytes {
    pub fn new(public_key: RistrettoPublicKeyBytes, signature: SchnorrSignatureBytes) -> Self {
        Self { public_key, signature }
    }

    pub fn public_key(&self) -> &RistrettoPublicKeyBytes {
        &self.public_key
    }
}
