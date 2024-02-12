//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{hashing::substate_value_hasher32, substate::SubstateId};
use tari_state_tree::{
    key_mapper::DbKeyMapper,
    memory_store::MemoryTreeStore,
    Hash,
    LeafKey,
    StateTree,
    SubstateChange,
    TreeStore,
    Version,
};
use tari_template_lib::models::ComponentAddress;

pub fn change(substate_id_seed: u8, value_seed: Option<u8>) -> SubstateChange {
    change_exact(
        SubstateId::Component(ComponentAddress::from_array([substate_id_seed; 32])),
        value_seed.map(from_seed),
    )
}

fn hash_value(value: &[u8]) -> Hash {
    substate_value_hasher32().chain(value).result().into_array().into()
}

pub fn change_exact(substate_id: SubstateId, value: Option<Vec<u8>>) -> SubstateChange {
    value
        .map(|value| SubstateChange::Up {
            id: substate_id.clone(),
            value_hash: hash_value(&value),
        })
        .unwrap_or_else(|| SubstateChange::Down { id: substate_id })
}

fn from_seed(node_key_seed: u8) -> Vec<u8> {
    vec![node_key_seed; node_key_seed as usize]
}

pub struct HashTreeTester<S> {
    pub tree_store: S,
    pub current_version: Option<Version>,
}

impl<S: TreeStore<Version>> HashTreeTester<S> {
    pub fn new(tree_store: S, current_version: Option<Version>) -> Self {
        Self {
            tree_store,
            current_version,
        }
    }

    pub fn put_substate_changes(&mut self, changes: impl IntoIterator<Item = SubstateChange>) -> Hash {
        self.apply_database_updates(changes)
    }

    fn apply_database_updates(&mut self, changes: impl IntoIterator<Item = SubstateChange>) -> Hash {
        let next_version = self.current_version.unwrap_or(0) + 1;
        let current_version = self.current_version.replace(next_version).unwrap_or(0);
        StateTree::<_, IdentityMapper>::new(&mut self.tree_store)
            .put_substate_changes(current_version, next_version, changes)
            .unwrap()
    }
}

impl HashTreeTester<MemoryTreeStore> {
    pub fn new_empty() -> Self {
        Self::new(MemoryTreeStore::new(), None)
    }
}

pub struct IdentityMapper;

impl DbKeyMapper for IdentityMapper {
    fn map_to_leaf_key(id: &SubstateId) -> LeafKey {
        LeafKey::new(id.to_canonical_hash().to_vec())
    }
}
