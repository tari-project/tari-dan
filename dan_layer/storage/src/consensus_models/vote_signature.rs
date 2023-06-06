//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::Signature;

use crate::consensus_models::validator_id::ValidatorId;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ValidatorSignature {
    pub validator_id: ValidatorId,
    pub signature: Signature,
}

impl ValidatorSignature {
    pub fn new(validator_id: ValidatorId, signature: Signature) -> Self {
        Self {
            validator_id,
            signature,
        }
    }
}
