//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::{PublicKey, Signature};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ValidatorSignature {
    pub public_key: PublicKey,
    pub signature: Signature,
}

impl ValidatorSignature {
    pub fn new(public_key: PublicKey, signature: Signature) -> Self {
        Self { public_key, signature }
    }
}
