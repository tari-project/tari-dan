//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateId;

use crate::jellyfish::LeafKey;

pub trait DbKeyMapper {
    fn map_to_leaf_key(id: &SubstateId) -> LeafKey;
}

const HASH_PREFIX_LENGTH: usize = 20;

pub struct SpreadPrefixKeyMapper;

impl DbKeyMapper for SpreadPrefixKeyMapper {
    fn map_to_leaf_key(id: &SubstateId) -> LeafKey {
        let hash = crate::jellyfish::hash(id.to_canonical_hash());
        let prefixed_key = [&hash[..HASH_PREFIX_LENGTH], hash.as_slice()].concat();
        LeafKey::new(prefixed_key)
    }
}
