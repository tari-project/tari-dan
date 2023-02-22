//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{BulletRangeProof, Commitment};
use tari_template_lib::models::ResourceAddress;

use crate::resource_container::ResourceContainer;

#[derive(Debug, Clone)]
pub struct ConfidentialBucket {
    address: ResourceAddress,
    commitment: Commitment,
    range_proof: BulletRangeProof,
}

impl ConfidentialBucket {
    pub fn new(address: ResourceAddress, commitment: Commitment, range_proof: BulletRangeProof) -> Self {
        Self {
            address,
            commitment,
            range_proof,
        }
    }

    pub fn into_resource(self) -> ResourceContainer {
        ResourceContainer::Confidential {
            address: self.address,
            commitments: vec![(self.commitment.as_public_key().clone(), self.range_proof)],
        }
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        &self.address
    }
}
