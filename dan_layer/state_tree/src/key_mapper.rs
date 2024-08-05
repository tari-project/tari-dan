//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateId;

use crate::jellyfish::LeafKey;

pub trait DbKeyMapper {
    fn map_to_leaf_key(id: &SubstateId) -> LeafKey;
}

pub struct SpreadPrefixKeyMapper;

impl DbKeyMapper for SpreadPrefixKeyMapper {
    fn map_to_leaf_key(id: &SubstateId) -> LeafKey {
        let hash = crate::jellyfish::jmt_node_hash(id);
        LeafKey::new(hash.to_vec())
    }
}
