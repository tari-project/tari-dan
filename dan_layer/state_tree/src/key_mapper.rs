//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateId;

use crate::{jellyfish::LeafKey, Hash};

pub trait DbKeyMapper<T> {
    fn map_to_leaf_key(id: &T) -> LeafKey;
}

pub struct SpreadPrefixKeyMapper;

impl DbKeyMapper<SubstateId> for SpreadPrefixKeyMapper {
    fn map_to_leaf_key(id: &SubstateId) -> LeafKey {
        let hash = crate::jellyfish::jmt_node_hash(id);
        LeafKey::new(hash)
    }
}

pub struct HashIdentityKeyMapper;

impl DbKeyMapper<Hash> for HashIdentityKeyMapper {
    fn map_to_leaf_key(hash: &Hash) -> LeafKey {
        LeafKey::new(*hash)
    }
}
