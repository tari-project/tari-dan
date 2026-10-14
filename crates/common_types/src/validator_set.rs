//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{self, Display};

use minicbor::{CborLen, Decode, Encode};
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_template_lib_types::{Hash32, crypto::RistrettoPublicKeyBytes};

use crate::{Epoch, NumPreshards, ShardGroup, VotePower, committee::Committee, hashing::tari_consensus_hasher};

/// Commits to an epoch's committee assignment: a Merkle tree with one leaf per shard group, at
/// [`validator_set_committee_key`] with value [`hash_validator_set_committee`], bound to the network, epoch and number
/// of preshards. One committee can be proved against the hash without the rest of the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Encode, Decode, CborLen)]
#[serde(transparent)]
#[cbor(transparent)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "string"))]
pub struct ValidatorSetHash(#[n(0)] Hash32);

impl ValidatorSetHash {
    /// Hashes the committee tree root, labelled `ValidatorSet`. The preimage is the domain tag followed by
    /// fixed-width little-endian integers:
    ///
    /// ```text
    /// u64(34) || "com.tari.consensus.v0.ValidatorSet"
    /// u8       network
    /// u64      epoch
    /// u32      num_preshards
    /// [u8; 32] committee_root
    /// ```
    pub fn calculate(network: Network, epoch: Epoch, num_preshards: NumPreshards, committee_root: &Hash32) -> Self {
        Self(
            tari_consensus_hasher("ValidatorSet")
                .chain(&network.as_byte())
                .chain(&epoch.as_u64())
                .chain(&num_preshards.as_u32())
                .chain(&committee_root.into_array())
                .finalize_into_array()
                .into(),
        )
    }

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

/// The key of `shard_group`'s leaf in the validator set tree, labelled `ValidatorSetCommitteeKey`:
///
/// ```text
/// u64(46) || "com.tari.consensus.v0.ValidatorSetCommitteeKey"
/// u32 shard group start
/// u32 shard group end_inclusive
/// ```
pub fn validator_set_committee_key(shard_group: &ShardGroup) -> Hash32 {
    tari_consensus_hasher("ValidatorSetCommitteeKey")
        .chain(&shard_group.start().as_u32())
        .chain(&shard_group.end().as_u32())
        .finalize_into_array()
        .into()
}

/// Hashes `shard_group`'s committee into its leaf of the validator set tree, labelled `ValidatorSetCommittee`.
///
/// Members must be in strictly ascending public key order, which also makes each public key unique; any other order
/// is an error. The preimage is the domain tag followed by fixed-width little-endian
/// integers:
///
/// ```text
/// u64(43) || "com.tari.consensus.v0.ValidatorSetCommittee"
/// u32 shard group start
/// u32 shard group end_inclusive
/// u32 number of members
/// for each member:
///     [u8; 32] public key
///     u64      vote power
/// ```
pub fn hash_validator_set_committee<'a, I>(shard_group: &ShardGroup, members: I) -> Result<Hash32, ValidatorSetError>
where
    I: IntoIterator<Item = (&'a RistrettoPublicKeyBytes, VotePower)>,
    I::IntoIter: ExactSizeIterator,
{
    let members = members.into_iter();
    let num_members = u32::try_from(members.len()).map_err(|_| ValidatorSetError::TooManyMembers {
        shard_group: *shard_group,
        num_members: members.len(),
    })?;
    let mut hasher = tari_consensus_hasher("ValidatorSetCommittee")
        .chain(&shard_group.start().as_u32())
        .chain(&shard_group.end().as_u32())
        .chain(&num_members);
    let mut previous: Option<&RistrettoPublicKeyBytes> = None;
    for (public_key, vote_power) in members {
        if let Some(previous) = previous.filter(|previous| *previous >= public_key) {
            return Err(ValidatorSetError::CommitteeNotSorted {
                shard_group: *shard_group,
                previous: *previous,
                public_key: *public_key,
            });
        }
        previous = Some(public_key);
        hasher = hasher.chain(&public_key.into_array()).chain(&vote_power.value());
    }
    Ok(hasher.finalize_into_array().into())
}

impl<TAddr> Committee<TAddr> {
    /// Hashes the committee into `shard_group`'s leaf of the validator set tree with [`hash_validator_set_committee`].
    /// The leaf commits to each member's public key and vote power.
    pub fn calculate_validator_set_leaf_hash(&self, shard_group: &ShardGroup) -> Result<Hash32, ValidatorSetError> {
        hash_validator_set_committee(
            shard_group,
            self.into_iter().map(|member| (&member.public_key, member.vote_power)),
        )
    }
}

/// A committee member as the validator set commits to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorSetMember {
    pub public_key: RistrettoPublicKeyBytes,
    pub vote_power: VotePower,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidatorSetError {
    #[error(
        "Committee of shard group {shard_group} lists public key {public_key} after {previous}, but a validator set \
         committee lists each public key once in ascending order"
    )]
    CommitteeNotSorted {
        shard_group: ShardGroup,
        previous: RistrettoPublicKeyBytes,
        public_key: RistrettoPublicKeyBytes,
    },
    #[error(
        "Committee of shard group {shard_group} has {num_members} members, more than a validator set can commit to"
    )]
    TooManyMembers {
        shard_group: ShardGroup,
        num_members: usize,
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

    fn committee(members: &[(&'static str, u8, u64)]) -> Committee<&'static str> {
        Committee::new(
            members
                .iter()
                .map(|(address, key_byte, power)| CommitteeMember {
                    address: *address,
                    public_key: pk(*key_byte),
                    vote_power: VotePower::of(*power),
                })
                .collect(),
        )
    }

    fn blake2b_256(label: &str, body: &[u8]) -> Hash32 {
        let tag = format!("com.tari.consensus.v0.{label}");
        let mut preimage = (tag.len() as u64).to_le_bytes().to_vec();
        preimage.extend_from_slice(tag.as_bytes());
        preimage.extend_from_slice(body);
        let digest: [u8; 32] = Blake2b::<U32>::digest(&preimage).into();
        digest.into()
    }

    #[test]
    fn the_leaf_hash_is_the_documented_preimage() {
        let mut body = Vec::new();
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        for (key_byte, power) in [(3u8, 2u64), (4, 1)] {
            body.extend_from_slice(&[key_byte; 32]);
            body.extend_from_slice(&power.to_le_bytes());
        }

        let leaf = committee(&[("c", 3, 2), ("d", 4, 1)])
            .calculate_validator_set_leaf_hash(&ShardGroup::new(2u32, 3u32))
            .unwrap();
        assert_eq!(leaf, blake2b_256("ValidatorSetCommittee", &body));
    }

    #[test]
    fn the_leaf_key_is_the_documented_preimage() {
        let mut body = Vec::new();
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&3u32.to_le_bytes());

        assert_eq!(
            validator_set_committee_key(&ShardGroup::new(2u32, 3u32)),
            blake2b_256("ValidatorSetCommitteeKey", &body)
        );
    }

    #[test]
    fn the_set_hash_is_the_documented_preimage() {
        let root = Hash32::from_array([0x5a; 32]);
        let mut body = vec![Network::LocalNet.as_byte()];
        body.extend_from_slice(&7u64.to_le_bytes());
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(&[0x5a; 32]);

        assert_eq!(
            *ValidatorSetHash::calculate(Network::LocalNet, Epoch(7), num_preshards(), &root).as_hash(),
            blake2b_256("ValidatorSet", &body)
        );
    }

    #[test]
    fn the_leaf_key_and_leaf_hash_of_a_shard_group_differ() {
        let shard_group = ShardGroup::new(0u32, 3u32);
        assert_ne!(
            validator_set_committee_key(&shard_group),
            committee(&[]).calculate_validator_set_leaf_hash(&shard_group).unwrap()
        );
    }

    #[test]
    fn the_leaf_ignores_member_addresses() {
        let shard_group = ShardGroup::new(2u32, 3u32);
        assert_eq!(
            committee(&[("c", 3, 2), ("d", 4, 1)])
                .calculate_validator_set_leaf_hash(&shard_group)
                .unwrap(),
            committee(&[("x", 3, 2), ("y", 4, 1)])
                .calculate_validator_set_leaf_hash(&shard_group)
                .unwrap()
        );
    }

    #[test]
    fn every_committed_field_changes_the_leaf_hash() {
        let shard_group = ShardGroup::new(2u32, 3u32);
        let leaf = |members: &[(&'static str, u8, u64)], shard_group| {
            committee(members)
                .calculate_validator_set_leaf_hash(&shard_group)
                .unwrap()
        };
        let sample = leaf(&[("c", 3, 2), ("d", 4, 1)], shard_group);

        assert_ne!(leaf(&[("c", 3, 2), ("d", 4, 1)], ShardGroup::new(1u32, 3u32)), sample);
        assert_ne!(leaf(&[("c", 3, 1), ("d", 4, 1)], shard_group), sample);
        assert_ne!(leaf(&[("c", 3, 2), ("d", 5, 1)], shard_group), sample);
        assert_ne!(leaf(&[("c", 3, 2)], shard_group), sample);
    }

    #[test]
    fn every_outer_field_changes_the_set_hash() {
        let root = Hash32::from_array([0x5a; 32]);
        let sample = ValidatorSetHash::calculate(Network::LocalNet, Epoch(7), num_preshards(), &root);
        let eight = NumPreshards::try_from(8u32).unwrap();

        assert_ne!(
            ValidatorSetHash::calculate(Network::Esmeralda, Epoch(7), num_preshards(), &root),
            sample
        );
        assert_ne!(
            ValidatorSetHash::calculate(Network::LocalNet, Epoch(8), num_preshards(), &root),
            sample
        );
        assert_ne!(
            ValidatorSetHash::calculate(Network::LocalNet, Epoch(7), eight, &root),
            sample
        );
        assert_ne!(
            ValidatorSetHash::calculate(Network::LocalNet, Epoch(7), num_preshards(), &Hash32::zero()),
            sample
        );
    }

    #[test]
    fn it_rejects_members_out_of_public_key_order() {
        let shard_group = ShardGroup::new(0u32, 3u32);
        let err = committee(&[("a", 2, 1), ("b", 1, 1)])
            .calculate_validator_set_leaf_hash(&shard_group)
            .unwrap_err();

        assert_eq!(err, ValidatorSetError::CommitteeNotSorted {
            shard_group,
            previous: pk(2),
            public_key: pk(1),
        });
    }

    #[test]
    fn it_rejects_a_repeated_public_key() {
        let shard_group = ShardGroup::new(0u32, 3u32);
        let err = committee(&[("a", 1, 1), ("b", 1, 2)])
            .calculate_validator_set_leaf_hash(&shard_group)
            .unwrap_err();

        assert_eq!(err, ValidatorSetError::CommitteeNotSorted {
            shard_group,
            previous: pk(1),
            public_key: pk(1),
        });
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
