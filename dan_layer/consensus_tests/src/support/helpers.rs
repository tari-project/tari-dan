//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use rand::{rngs::OsRng, Rng};
use tari_dan_common_types::{shard_bucket::ShardBucket, uint::U256, ShardId};

pub(crate) fn random_shard_in_bucket(bucket: ShardBucket, num_committees: u32) -> ShardId {
    let shard_size = U256::MAX / U256::from(num_committees);
    // Hack to get a random u256 in a range since U256 doesnt implement UniformSample
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&shard_size.as_le_bytes()[..16]);
    let offset = u128::from_le_bytes(bytes);
    let offset = OsRng.gen_range(0, offset);
    let shard = shard_size * U256::from(bucket.as_u32()) + U256::from(offset);
    ShardId::from_u256(shard)
}

#[allow(dead_code)]
pub fn random_shard() -> ShardId {
    let lsb: u128 = OsRng.gen();
    let msb: u128 = OsRng.gen();
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(&lsb.to_le_bytes());
    bytes[16..].copy_from_slice(&msb.to_le_bytes());
    ShardId::from_u256(U256::from_le_bytes(bytes))
}
