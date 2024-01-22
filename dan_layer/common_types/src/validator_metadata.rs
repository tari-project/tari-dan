//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use blake2::{digest::consts::U32, Blake2b};
use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_core::{consensus::DomainSeparatedConsensusHasher, transactions::TransactionHashDomain};

use crate::SubstateAddress;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ValidatorMetadata {
    pub public_key: PublicKey,
    pub vn_shard_key: SubstateAddress,
    pub signature: Signature,
}

impl ValidatorMetadata {
    pub fn new(public_key: PublicKey, vn_shard_key: SubstateAddress, signature: Signature) -> Self {
        Self {
            public_key,
            vn_shard_key,
            signature,
        }
    }
}

pub fn vn_node_hash(public_key: &PublicKey, substate_address: &SubstateAddress) -> FixedHash {
    DomainSeparatedConsensusHasher::<TransactionHashDomain, Blake2b<U32>>::new("validator_node")
        .chain(public_key)
        .chain(&substate_address.0)
        .finalize()
        .into()
}
