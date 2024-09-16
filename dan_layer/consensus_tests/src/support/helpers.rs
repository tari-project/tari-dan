//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::ops::RangeBounds;

use rand::{rngs::OsRng, Rng, RngCore};
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::keys::{PublicKey as _, SecretKey};
use tari_dan_common_types::{
    uint::{U256, U256_ZERO},
    NumPreshards,
    ShardGroup,
    SubstateAddress,
};
use tari_engine_types::{
    component::{ComponentBody, ComponentHeader},
    substate::{SubstateId, SubstateValue},
};
use tari_template_lib::models::{ComponentAddress, ComponentKey, EntityId, ObjectKey};

use crate::support::TestAddress;

pub(crate) fn random_substate_in_shard_group(shard_group: ShardGroup, num_shards: NumPreshards) -> SubstateId {
    let range = shard_group.to_substate_address_range(num_shards);
    let middlish = random_substate_address_range(range);
    let entity_id = EntityId::new(copy_fixed(&middlish.to_u256().to_be_bytes()[0..EntityId::LENGTH]));
    let rand_bytes = OsRng.gen::<[u8; ComponentKey::LENGTH]>();
    let component_key = ComponentKey::new(copy_fixed(&rand_bytes));
    SubstateId::Component(ComponentAddress::new(ObjectKey::new(entity_id, component_key)))
}

fn random_substate_address_range<R: RangeBounds<SubstateAddress>>(range: R) -> SubstateAddress {
    let start = match range.start_bound() {
        std::ops::Bound::Included(addr) => addr.to_u256(),
        std::ops::Bound::Excluded(addr) => addr.to_u256() + 1,
        std::ops::Bound::Unbounded => U256_ZERO,
    };
    let end = match range.end_bound() {
        std::ops::Bound::Included(addr) => addr.to_u256(),
        std::ops::Bound::Excluded(addr) => addr.to_u256() - 1,
        std::ops::Bound::Unbounded => U256::MAX,
    };
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let rand = U256::from_le_bytes(bytes);
    SubstateAddress::from_u256_zero_version(start + (rand % (end - start)))
}

fn copy_fixed<const SZ: usize>(bytes: &[u8]) -> [u8; SZ] {
    let mut out = [0u8; SZ];
    out.copy_from_slice(bytes);
    out
}

pub fn derive_keypair_from_address(addr: &TestAddress) -> (PrivateKey, PublicKey) {
    let mut bytes = [0u8; 64];
    bytes[0..addr.as_bytes().len()].copy_from_slice(addr.as_bytes());
    let secret_key = PrivateKey::from_uniform_bytes(&bytes).unwrap();
    let public_key = PublicKey::from_secret_key(&secret_key);
    (secret_key, public_key)
}

pub fn make_test_component(entity_id: EntityId) -> SubstateValue {
    SubstateValue::Component(ComponentHeader {
        template_address: Default::default(),
        module_name: "Test".to_string(),
        owner_key: None,
        owner_rule: Default::default(),
        access_rules: Default::default(),
        entity_id,
        body: ComponentBody {
            state: tari_bor::Value::Null,
        },
    })
}
