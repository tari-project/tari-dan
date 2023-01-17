//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{BulletRangeProof, Commitment};

#[derive(Debug, Clone)]
pub struct ConfidentialBucket {
    commitment: Commitment,
    range_proof: BulletRangeProof,
}

impl ConfidentialBucket {
    pub fn new(commitment: Commitment, range_proof: BulletRangeProof) -> Self {
        Self {
            commitment,
            range_proof,
        }
    }
}
