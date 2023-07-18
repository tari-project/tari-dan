//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashSet,
    hash::{BuildHasher, Hasher},
};

use siphasher::sip::SipHasher13;

pub type UniqueSet<T> = HashSet<T, FixedState>;

#[derive(Debug, Clone, Default)]
struct FixedState;

impl BuildHasher for FixedState {
    type Hasher = SipHasher13;

    fn build_hasher(&self) -> SipHasher13 {
        SipHasher13::new_with_keys(0, 0)
    }
}

#[cfg(test)]
mod tests {
    use rand::{rngs::OsRng, Rng};
    use tari_dan_common_types::{uint::U256, ShardId};

    use super::*;

    pub fn random_shard() -> ShardId {
        let lsb: u128 = OsRng.gen();
        let msb: u128 = OsRng.gen();
        let mut bytes = [0u8; 32];
        bytes[..16].copy_from_slice(&lsb.to_le_bytes());
        bytes[16..].copy_from_slice(&msb.to_le_bytes());
        ShardId::from_u256(U256::from_le_bytes(bytes))
    }

    #[test]
    fn blah() {
        let mut a = UniqueSet::with_hasher(FixedState);

        a.insert(random_shard());
        a.insert(random_shard());
        a.insert(random_shard());
        a.insert(random_shard());
        a.insert(random_shard());
        a.insert(random_shard());
        // a.insert(1);
        // a.insert(2);
        // a.insert(3);
        // a.insert(4);

        for i in &a {
            println!("{}", i);
        }
        println!("-----");

        let e = bincode::serialize(&a).unwrap();
        let d: UniqueSet<ShardId> = bincode::deserialize(&e).unwrap();

        for i in &d {
            println!("{}", i);
        }
        assert_eq!(a.into_iter().collect::<Vec<_>>(), d.into_iter().collect::<Vec<_>>());
    }
}
