//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use rand::{rngs::OsRng, Rng};
use tari_dan_common_types::{shard::Shard, uint::U256};
use tari_engine_types::substate::SubstateId;
use tari_template_lib::models::{ComponentAddress, ComponentKey, EntityId, ObjectKey};
use tari_transaction::VersionedSubstateId;

pub(crate) fn random_substate_in_bucket(bucket: Shard, num_committees: u32) -> VersionedSubstateId {
    let shard_size = U256::MAX / U256::from(num_committees);
    let offset = u128::from_le_bytes(copy_fixed(&shard_size.as_le_bytes()[..16]));
    let shard = shard_size * U256::from(bucket.as_u32()) + U256::from(offset);
    let entity_id = EntityId::new(copy_fixed(&shard.to_be_bytes::<32>()[0..EntityId::LENGTH]));
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
