//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use rand::{rngs::OsRng, Rng};
use tari_dan_common_types::shard::Shard;
use tari_engine_types::substate::SubstateId;
use tari_template_lib::models::{ComponentAddress, ComponentKey, EntityId, ObjectKey};
use tari_transaction::VersionedSubstateId;

pub(crate) fn random_substate_in_shard(shard: Shard, num_shards: u32) -> VersionedSubstateId {
    let range = shard.to_substate_address_range(num_shards);
    let size = range.end().to_u256() - range.start().to_u256();
    let middlish = range.start().to_u256() + size / 2;
    let entity_id = EntityId::new(copy_fixed(&middlish.to_be_bytes()[0..EntityId::LENGTH]));
    let rand_bytes = OsRng.gen::<[u8; ComponentKey::LENGTH]>();
    let component_key = ComponentKey::new(copy_fixed(&rand_bytes));
    let substate_id = SubstateId::Component(ComponentAddress::new(ObjectKey::new(entity_id, component_key)));
    VersionedSubstateId::new(substate_id, 0)
}

fn copy_fixed<const SZ: usize>(bytes: &[u8]) -> [u8; SZ] {
    let mut out = [0u8; SZ];
    out.copy_from_slice(bytes);
    out
}
