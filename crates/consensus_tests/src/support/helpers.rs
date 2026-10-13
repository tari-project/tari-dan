//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::ops::RangeInclusive;

use rand::{Rng, RngExt};
use tari_common_types::types::PrivateKey;
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::RistrettoPublicKey,
};
use tari_engine_types::{
    component::{Component, ComponentBody, ComponentHeader},
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_common_types::{NumPreshards, ShardGroup, SubstateAddress};
use tari_template_lib_types::{ComponentAddress, ComponentKey, EntityId, ObjectKey, SubstateOwnerRule};

use crate::support::TestAddress;

pub(crate) fn random_substate_in_shard_group(shard_group: ShardGroup, num_shards: NumPreshards) -> SubstateId {
    let range = shard_group.to_substate_address_range(num_shards);
    let random_in_range = random_substate_address_range(range);
    let entity_id = EntityId::new(copy_fixed(
        &random_in_range.to_u256().to_be_bytes()[0..EntityId::LENGTH],
    ));
    let rand_bytes: [u8; ComponentKey::LENGTH] = rand::rng().random();
    let component_key = ComponentKey::new(copy_fixed(&rand_bytes));
    SubstateId::Component(ComponentAddress::new(ObjectKey::new(entity_id, component_key)))
}

// TODO: this biases the start of the shard group
fn random_substate_address_range(range: RangeInclusive<SubstateAddress>) -> SubstateAddress {
    let start = range.start();
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let mut start = start.into_array();
    start[16..32].copy_from_slice(&bytes);
    SubstateAddress::from_bytes(&start).unwrap()
}

fn copy_fixed<const SZ: usize>(bytes: &[u8]) -> [u8; SZ] {
    let mut out = [0u8; SZ];
    out.copy_from_slice(bytes);
    out
}

pub fn derive_keypair_from_address(addr: &TestAddress) -> (PrivateKey, RistrettoPublicKey) {
    let mut bytes = [0u8; 64];
    bytes[0..addr.as_bytes().len()].copy_from_slice(addr.as_bytes());
    let secret_key = PrivateKey::from_uniform_bytes(&bytes).unwrap();
    let public_key = RistrettoPublicKey::from_secret_key(&secret_key);
    (secret_key, public_key)
}

/// A component whose state is a byte string of `state_size` bytes, or null if `state_size` is zero.
pub fn make_test_component_with_state_size(entity_id: EntityId, state_size: usize) -> SubstateValue {
    let state = if state_size == 0 {
        tari_bor::Value::Null
    } else {
        tari_bor::Value::Bytes(vec![0xAB; state_size])
    };
    SubstateValue::Component(Component {
        header: ComponentHeader {
            template_address: Default::default(),
            owner_rule: SubstateOwnerRule::None,
            access_rules: Default::default(),
            entity_id,
        },
        body: ComponentBody { state },
    })
}

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::{ToSubstateAddress, VersionedSubstateId};

    use super::*;

    mod random_substate_address_range {
        use tari_ootle_common_types::SubstateVersion;

        use super::*;

        #[test]
        fn it_generates_a_random_address_in_shard_group() {
            const NUM_PRESHARDS: NumPreshards = NumPreshards::P64;
            for _ in 0..100 {
                let sg = ShardGroup::new(1, 16);
                let substate = random_substate_in_shard_group(sg, NUM_PRESHARDS);
                let actual_sg = VersionedSubstateId::new(substate, SubstateVersion::ZERO)
                    .to_substate_address()
                    .to_shard_group(NUM_PRESHARDS, 4);
                assert_eq!(sg, actual_sg);

                let sg = ShardGroup::new(2 * 16 + 1, 3 * 16);
                let substate = random_substate_in_shard_group(sg, NUM_PRESHARDS);
                let actual_sg = VersionedSubstateId::new(substate, SubstateVersion::ZERO)
                    .to_substate_address()
                    .to_shard_group(NUM_PRESHARDS, 4);
                assert_eq!(sg, actual_sg);

                let sg = ShardGroup::new(3 * 16 + 1, 4 * 16);
                let substate = random_substate_in_shard_group(sg, NUM_PRESHARDS);
                let actual_sg = VersionedSubstateId::new(substate, SubstateVersion::ZERO)
                    .to_substate_address()
                    .to_shard_group(NUM_PRESHARDS, 4);
                assert_eq!(sg, actual_sg);
            }
        }
    }
}
