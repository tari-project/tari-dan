//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use blake2::{digest::consts::U32, Blake2b};
use serde::{Deserialize, Serialize};
use tari_common::configuration::Network;
use tari_common_types::types::{FixedHash, PublicKey, Signature};
use tari_crypto::{hash_domain, hashing::DomainSeparation};
use tari_engine_types::hashing::TariHasher32;
use tari_hash_domains::TariEngineHashDomain;

// TODO: add to tari_hash_domains
hash_domain!(TransactionHashDomain, "com.tari.base_layer.core.transactions", 0);

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

pub fn vn_node_hash(network: Network, public_key: &PublicKey, substate_address: &SubstateAddress) -> FixedHash {
    let mut digest = Blake2b::<U32>::default();
    TariEngineHashDomain::add_domain_separation_tag(&mut digest, &format!("validator_node.n{}", network.as_byte()));
    // TODO: TariHasher32 is the same as the consensus hasher in tari_core. The consensus hasher should be part of the
    // common hashing crate, currently called tari_hash_domains. Should rename it to tari_hasher/tari_hashing and
    // include the consensus hasher. This is done to remove the dependency on tari_core which has a bunch of
    // dependencies e.g. tari_comms, dht etc. "Type" crates should always have minimal dependencies.
    TariHasher32::from_digest(digest)
        .chain(public_key)
        .chain(&substate_address.0)
        .result()
        .into_array()
        .into()
}
