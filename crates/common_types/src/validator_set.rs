//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fmt::{self, Display},
};

use minicbor::{CborLen, Decode, Encode};
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_template_lib_types::{Hash32, crypto::RistrettoPublicKeyBytes};

use crate::{Epoch, NumPreshards, ShardGroup, VotePower, committee::Committee, hashing::tari_consensus_hasher};

/// The committee assignment of one epoch: every shard group and the public key and vote power of each of its members.
///
/// Shard groups are ordered by `(start, end_inclusive)` and members by public key, so the order a caller supplies them
/// in never reaches [`Self::calculate_hash`]. A public key appears at most once in the whole set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorSet {
    network: Network,
    epoch: Epoch,
    num_preshards: NumPreshards,
    committees: BTreeMap<ShardGroup, BTreeMap<RistrettoPublicKeyBytes, VotePower>>,
}

impl ValidatorSet {
    /// Builds the set from each shard group's members.
    ///
    /// Fails if a shard group is given twice or a public key appears more than once, within one shard group or across
    /// shard groups.
    pub fn init<I, M>(
        network: Network,
        epoch: Epoch,
        num_preshards: NumPreshards,
        committees: I,
    ) -> Result<Self, ValidatorSetError>
    where
        I: IntoIterator<Item = (ShardGroup, M)>,
        M: IntoIterator<Item = (RistrettoPublicKeyBytes, VotePower)>,
    {
        let mut seen_public_keys = BTreeSet::new();
        let mut assignment = BTreeMap::new();
        for (shard_group, members) in committees {
            let Entry::Vacant(entry) = assignment.entry(shard_group) else {
                return Err(ValidatorSetError::DuplicateShardGroup { shard_group });
            };
            let mut committee = BTreeMap::new();
            for (public_key, vote_power) in members {
                if !seen_public_keys.insert(public_key) {
                    return Err(ValidatorSetError::DuplicatePublicKey {
                        shard_group,
                        public_key,
                    });
                }
                committee.insert(public_key, vote_power);
            }
            entry.insert(committee);
        }

        Ok(Self {
            network,
            epoch,
            num_preshards,
            committees: assignment,
        })
    }

    /// Builds the set from the epoch manager's committees. Member addresses are not part of the set.
    pub fn init_from_committees<'a, TAddr: 'a, I>(
        network: Network,
        epoch: Epoch,
        num_preshards: NumPreshards,
        committees: I,
    ) -> Result<Self, ValidatorSetError>
    where
        I: IntoIterator<Item = (ShardGroup, &'a Committee<TAddr>)>,
    {
        Self::init(
            network,
            epoch,
            num_preshards,
            committees.into_iter().map(|(shard_group, committee)| {
                (
                    shard_group,
                    committee
                        .into_iter()
                        .map(|member| (member.public_key, member.vote_power)),
                )
            }),
        )
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn num_preshards(&self) -> NumPreshards {
        self.num_preshards
    }

    pub fn num_shard_groups(&self) -> usize {
        self.committees.len()
    }

    /// Returns the members of `shard_group` ordered by public key, or `None` if the set has no such shard group.
    pub fn committee(
        &self,
        shard_group: &ShardGroup,
    ) -> Option<impl Iterator<Item = (&RistrettoPublicKeyBytes, VotePower)> + '_> {
        self.committees
            .get(shard_group)
            .map(|committee| committee.iter().map(|(pk, power)| (pk, *power)))
    }

    /// Returns the shard groups in ascending order.
    pub fn shard_groups(&self) -> impl Iterator<Item = &ShardGroup> + '_ {
        self.committees.keys()
    }

    /// Hashes the set with the Blake2b-256 consensus hasher, labelled `ValidatorSet`.
    ///
    /// The preimage is the domain tag followed by fixed-width little-endian integers, so a client in any language can
    /// reproduce it:
    ///
    /// ```text
    /// u64(34) || "com.tari.consensus.v0.ValidatorSet"
    /// u8  network
    /// u64 epoch
    /// u32 num_preshards
    /// u32 number of shard groups
    /// for each shard group, ascending by (start, end_inclusive):
    ///     u32 start
    ///     u32 end_inclusive
    ///     u32 number of members
    ///     for each member, ascending by public key bytes:
    ///         [u8; 32] public key
    ///         u64      vote power
    /// ```
    pub fn calculate_hash(&self) -> ValidatorSetHash {
        let mut hasher = tari_consensus_hasher("ValidatorSet")
            .chain(&self.network.as_byte())
            .chain(&self.epoch.as_u64())
            .chain(&self.num_preshards.as_u32())
            .chain(&len_as_u32(self.committees.len()));
        for (shard_group, committee) in &self.committees {
            hasher = hasher
                .chain(&shard_group.start().as_u32())
                .chain(&shard_group.end().as_u32())
                .chain(&len_as_u32(committee.len()));
            for (public_key, vote_power) in committee {
                hasher = hasher.chain(&public_key.into_array()).chain(&vote_power.value());
            }
        }
        ValidatorSetHash(hasher.finalize_into_array().into())
    }
}

fn len_as_u32(len: usize) -> u32 {
    u32::try_from(len).expect("INVARIANT: a validator set has fewer than 2^32 shard groups and members")
}

/// Commits to an epoch's [`ValidatorSet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Encode, Decode, CborLen)]
#[serde(transparent)]
#[cbor(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "string"))]
pub struct ValidatorSetHash(#[n(0)] Hash32);

impl ValidatorSetHash {
    pub const fn from_array(bytes: [u8; Hash32::LENGTH]) -> Self {
        Self(Hash32::from_array(bytes))
    }

    pub const fn into_array(self) -> [u8; Hash32::LENGTH] {
        self.0.into_array()
    }

    pub const fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub const fn as_hash(&self) -> &Hash32 {
        &self.0
    }
}

impl From<Hash32> for ValidatorSetHash {
    fn from(hash: Hash32) -> Self {
        Self(hash)
    }
}

impl AsRef<[u8]> for ValidatorSetHash {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Display for ValidatorSetHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidatorSetError {
    #[error("Validator set lists shard group {shard_group} more than once")]
    DuplicateShardGroup { shard_group: ShardGroup },
    #[error("Validator set lists public key {public_key} more than once (again in shard group {shard_group})")]
    DuplicatePublicKey {
        shard_group: ShardGroup,
        public_key: RistrettoPublicKeyBytes,
    },
}

#[cfg(test)]
mod tests {
    use blake2::{Blake2b, Digest, digest::consts::U32};

    use super::*;
    use crate::committee::CommitteeMember;

    fn pk(byte: u8) -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(&[byte; 32]).unwrap()
    }

    fn num_preshards() -> NumPreshards {
        NumPreshards::try_from(4u32).unwrap()
    }

    fn one() -> VotePower {
        VotePower::of(1)
    }

    fn golden_set() -> ValidatorSet {
        ValidatorSet::init(Network::LocalNet, Epoch(7), num_preshards(), [
            (ShardGroup::new(2u32, 3u32), vec![
                (pk(4), one()),
                (pk(3), VotePower::of(2)),
            ]),
            (ShardGroup::new(0u32, 1u32), vec![(pk(2), one()), (pk(1), one())]),
        ])
        .unwrap()
    }

    #[test]
    fn it_hashes_to_the_golden_vector() {
        assert_eq!(
            golden_set().calculate_hash().to_string(),
            "ba186f52df7236687b23f7311e263ebcee8f40ea7bcf6d9b9dc2860cf0a26391"
        );
    }

    #[test]
    fn it_hashes_the_documented_preimage() {
        let label = b"com.tari.consensus.v0.ValidatorSet";
        let mut preimage = Vec::new();
        preimage.extend_from_slice(&(label.len() as u64).to_le_bytes());
        preimage.extend_from_slice(label);
        preimage.push(Network::LocalNet.as_byte());
        preimage.extend_from_slice(&7u64.to_le_bytes());
        preimage.extend_from_slice(&4u32.to_le_bytes());
        preimage.extend_from_slice(&2u32.to_le_bytes());
        for (start, end, members) in [(0u32, 1u32, [(1u8, 1u64), (2, 1)]), (2, 3, [(3, 2), (4, 1)])] {
            preimage.extend_from_slice(&start.to_le_bytes());
            preimage.extend_from_slice(&end.to_le_bytes());
            preimage.extend_from_slice(&2u32.to_le_bytes());
            for (key_byte, power) in members {
                preimage.extend_from_slice(&[key_byte; 32]);
                preimage.extend_from_slice(&power.to_le_bytes());
            }
        }
        let expected: [u8; 32] = Blake2b::<U32>::digest(&preimage).into();

        assert_eq!(golden_set().calculate_hash().into_array(), expected);
    }

    #[test]
    fn input_order_does_not_change_the_hash() {
        let reordered = ValidatorSet::init(Network::LocalNet, Epoch(7), num_preshards(), [
            (ShardGroup::new(0u32, 1u32), vec![(pk(1), one()), (pk(2), one())]),
            (ShardGroup::new(2u32, 3u32), vec![
                (pk(3), VotePower::of(2)),
                (pk(4), one()),
            ]),
        ])
        .unwrap();

        assert_eq!(reordered, golden_set());
        assert_eq!(reordered.calculate_hash(), golden_set().calculate_hash());
    }

    #[test]
    fn every_committed_field_changes_the_hash() {
        let golden = golden_set().calculate_hash();
        let groups = || {
            [
                (ShardGroup::new(0u32, 1u32), vec![(pk(1), one()), (pk(2), one())]),
                (ShardGroup::new(2u32, 3u32), vec![
                    (pk(3), VotePower::of(2)),
                    (pk(4), one()),
                ]),
            ]
        };
        let hash_of = |network, epoch, num_preshards, committees| {
            ValidatorSet::init(network, epoch, num_preshards, committees)
                .unwrap()
                .calculate_hash()
        };

        assert_ne!(hash_of(Network::Esmeralda, Epoch(7), num_preshards(), groups()), golden);
        assert_ne!(hash_of(Network::LocalNet, Epoch(8), num_preshards(), groups()), golden);
        let eight = NumPreshards::try_from(8u32).unwrap();
        assert_ne!(hash_of(Network::LocalNet, Epoch(7), eight, groups()), golden);

        let mut moved_boundary = groups();
        moved_boundary[0].0 = ShardGroup::new(0u32, 0u32);
        moved_boundary[1].0 = ShardGroup::new(1u32, 3u32);
        assert_ne!(
            hash_of(Network::LocalNet, Epoch(7), num_preshards(), moved_boundary),
            golden
        );

        let mut reweighted = groups();
        reweighted[1].1[0].1 = one();
        assert_ne!(
            hash_of(Network::LocalNet, Epoch(7), num_preshards(), reweighted),
            golden
        );

        let mut swapped_member = groups();
        swapped_member[0].1[0].0 = pk(5);
        assert_ne!(
            hash_of(Network::LocalNet, Epoch(7), num_preshards(), swapped_member),
            golden
        );

        let mut moved_member = groups();
        let member = moved_member[0].1.pop().unwrap();
        moved_member[1].1.push(member);
        assert_ne!(
            hash_of(Network::LocalNet, Epoch(7), num_preshards(), moved_member),
            golden
        );
    }

    #[test]
    fn it_rejects_a_repeated_shard_group() {
        let err = ValidatorSet::init(Network::LocalNet, Epoch(7), num_preshards(), [
            (ShardGroup::new(0u32, 1u32), vec![(pk(1), one())]),
            (ShardGroup::new(0u32, 1u32), vec![(pk(2), one())]),
        ])
        .unwrap_err();

        assert_eq!(err, ValidatorSetError::DuplicateShardGroup {
            shard_group: ShardGroup::new(0u32, 1u32)
        });
    }

    #[test]
    fn it_rejects_a_public_key_repeated_within_a_shard_group() {
        let err = ValidatorSet::init(Network::LocalNet, Epoch(7), num_preshards(), [(
            ShardGroup::new(0u32, 3u32),
            vec![(pk(1), one()), (pk(1), VotePower::of(2))],
        )])
        .unwrap_err();

        assert_eq!(err, ValidatorSetError::DuplicatePublicKey {
            shard_group: ShardGroup::new(0u32, 3u32),
            public_key: pk(1),
        });
    }

    #[test]
    fn it_rejects_a_public_key_in_two_shard_groups() {
        let err = ValidatorSet::init(Network::LocalNet, Epoch(7), num_preshards(), [
            (ShardGroup::new(0u32, 1u32), vec![(pk(1), one())]),
            (ShardGroup::new(2u32, 3u32), vec![(pk(1), one())]),
        ])
        .unwrap_err();

        assert_eq!(err, ValidatorSetError::DuplicatePublicKey {
            shard_group: ShardGroup::new(2u32, 3u32),
            public_key: pk(1),
        });
    }

    #[test]
    fn it_builds_the_same_set_from_committees_without_addresses() {
        let member = |address: &'static str, key_byte, power| CommitteeMember {
            address,
            public_key: pk(key_byte),
            vote_power: power,
        };
        let low = Committee::new(vec![member("b", 2, one()), member("a", 1, one())]);
        let high = Committee::new(vec![member("c", 3, VotePower::of(2)), member("d", 4, one())]);

        let set = ValidatorSet::init_from_committees(Network::LocalNet, Epoch(7), num_preshards(), [
            (ShardGroup::new(2u32, 3u32), &high),
            (ShardGroup::new(0u32, 1u32), &low),
        ])
        .unwrap();

        assert_eq!(set, golden_set());
    }

    #[test]
    fn the_hash_encodes_as_a_hex_string_and_a_cbor_byte_string() {
        let hash = ValidatorSetHash::from_array([0xab; 32]);

        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, format!("\"{}\"", "ab".repeat(32)));
        assert_eq!(serde_json::from_str::<ValidatorSetHash>(&json).unwrap(), hash);

        let cbor = minicbor::to_vec(hash).unwrap();
        assert_eq!(
            cbor,
            minicbor::to_vec(minicbor::bytes::ByteArray::from([0xab; 32])).unwrap()
        );
        assert_eq!(minicbor::decode::<ValidatorSetHash>(&cbor).unwrap(), hash);
    }
}
