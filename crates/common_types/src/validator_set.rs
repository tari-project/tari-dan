//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{self, Display};

use minicbor::{CborLen, Decode, Encode};
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_template_lib_types::{Hash32, crypto::RistrettoPublicKeyBytes};

use crate::{
    Epoch,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    VotePower,
    committee::Committee,
    hashing::tari_consensus_hasher,
};

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
/// `version` is the protocol version of the epoch the set belongs to, and the leaf commits to it so that a leaf names
/// the layout it was hashed under. Members must be in strictly ascending public key order, which also makes each public
/// key unique; any other order is an error. Each member's [`MemberKey`]s must be in strictly ascending scheme order,
/// none of them [`KeyScheme::RISTRETTO`], so one member set has exactly one leaf. The preimage is the domain tag
/// followed by fixed-width little-endian integers and length-prefixed key bytes:
///
/// ```text
/// u64(43) || "com.tari.consensus.v0.ValidatorSetCommittee"
/// u32 protocol version
/// u32 shard group start
/// u32 shard group end_inclusive
/// u32 number of members
/// for each member:
///     [u8; 32] public key (scheme 0, Ristretto)
///     u64      vote power
///     u32      number of additional keys
///     for each additional key:
///         u16  scheme
///         u32  key length
///         [u8] key
/// ```
pub fn hash_validator_set_committee<'a, I>(
    version: ProtocolVersion,
    shard_group: &ShardGroup,
    members: I,
) -> Result<Hash32, ValidatorSetError>
where
    I: IntoIterator<Item = (&'a RistrettoPublicKeyBytes, VotePower, &'a [MemberKey])>,
    I::IntoIter: ExactSizeIterator,
{
    let members = members.into_iter();
    let num_members = u32::try_from(members.len()).map_err(|_| ValidatorSetError::TooManyMembers {
        shard_group: *shard_group,
        num_members: members.len(),
    })?;
    let mut hasher = tari_consensus_hasher("ValidatorSetCommittee")
        .chain(&version.as_u32())
        .chain(&shard_group.start().as_u32())
        .chain(&shard_group.end().as_u32())
        .chain(&num_members);
    let mut previous: Option<&RistrettoPublicKeyBytes> = None;
    for (public_key, vote_power, additional_keys) in members {
        if let Some(previous) = previous.filter(|previous| *previous >= public_key) {
            return Err(ValidatorSetError::CommitteeNotSorted {
                shard_group: *shard_group,
                previous: *previous,
                public_key: *public_key,
            });
        }
        previous = Some(public_key);
        let num_keys = u32::try_from(additional_keys.len()).map_err(|_| ValidatorSetError::TooManyKeys {
            shard_group: *shard_group,
            public_key: *public_key,
            num_keys: additional_keys.len(),
        })?;
        hasher = hasher
            .chain(&public_key.into_array())
            .chain(&vote_power.value())
            .chain(&num_keys);
        let mut previous_scheme = KeyScheme::RISTRETTO;
        for key in additional_keys {
            if key.scheme <= previous_scheme {
                return Err(ValidatorSetError::KeysNotSorted {
                    shard_group: *shard_group,
                    public_key: *public_key,
                    previous: previous_scheme,
                    scheme: key.scheme,
                });
            }
            previous_scheme = key.scheme;
            if u32::try_from(key.key.len()).is_err() {
                return Err(ValidatorSetError::KeyTooLong {
                    shard_group: *shard_group,
                    public_key: *public_key,
                    scheme: key.scheme,
                });
            }
            // Borsh encodes a byte slice as its u32 length followed by the bytes.
            hasher = hasher.chain(&key.scheme.as_u16()).chain(key.key.as_slice());
        }
    }
    Ok(hasher.finalize_into_array().into())
}

impl<TAddr> Committee<TAddr> {
    /// Hashes the committee into `shard_group`'s leaf of the validator set tree with [`hash_validator_set_committee`].
    /// The leaf commits to each member's public key and vote power. A committee member has no additional keys.
    pub fn calculate_validator_set_leaf_hash(
        &self,
        version: ProtocolVersion,
        shard_group: &ShardGroup,
    ) -> Result<Hash32, ValidatorSetError> {
        hash_validator_set_committee(
            version,
            shard_group,
            self.into_iter()
                .map(|member| (&member.public_key, member.vote_power, &[][..])),
        )
    }
}

/// A committee member as the validator set commits to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorSetMember {
    /// The member's identity: its committee position and its [`KeyScheme::RISTRETTO`] signing key.
    pub public_key: RistrettoPublicKeyBytes,
    pub vote_power: VotePower,
    /// The member's keys under other signature schemes, strictly ascending by scheme. Absent from the encoding when
    /// empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_keys: Vec<MemberKey>,
}

impl ValidatorSetMember {
    /// The member's key under `scheme`, if it has one.
    pub fn key(&self, scheme: KeyScheme) -> Option<&[u8]> {
        if scheme == KeyScheme::RISTRETTO {
            return Some(self.public_key.as_bytes());
        }
        self.additional_keys
            .iter()
            .find(|key| key.scheme == scheme)
            .map(|key| key.key.as_slice())
    }
}

/// The signature scheme of a validator set member's key.
///
/// A scheme is an open number, so a decoder that predates a scheme still decodes a committee carrying keys under it,
/// hashes it to the same leaf and verifies it. Whether a scheme's keys may be used to verify signatures is the
/// protocol version's decision, made where the key is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyScheme(u16);

impl KeyScheme {
    /// Ristretto Schnorr, the scheme of every member's identity key, [`ValidatorSetMember::public_key`].
    pub const RISTRETTO: Self = Self(0);

    pub const fn from_u16(scheme: u16) -> Self {
        Self(scheme)
    }

    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

impl Display for KeyScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

/// A validator set member's public key under a signature scheme other than [`KeyScheme::RISTRETTO`]. The key is
/// opaque to the validator set, which commits to its scheme and bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberKey {
    pub scheme: KeyScheme,
    #[serde(with = "ootle_serde::hex")]
    pub key: Vec<u8>,
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
    #[error(
        "Member {public_key} of shard group {shard_group} lists a key of scheme {scheme} after scheme {previous}, but \
         a member lists each scheme once in ascending order, after the Ristretto scheme 0 of its public key"
    )]
    KeysNotSorted {
        shard_group: ShardGroup,
        public_key: RistrettoPublicKeyBytes,
        previous: KeyScheme,
        scheme: KeyScheme,
    },
    #[error(
        "Member {public_key} of shard group {shard_group} has {num_keys} additional keys, more than a validator set \
         can commit to"
    )]
    TooManyKeys {
        shard_group: ShardGroup,
        public_key: RistrettoPublicKeyBytes,
        num_keys: usize,
    },
    #[error(
        "Member {public_key} of shard group {shard_group} has a scheme {scheme} key longer than a validator set can \
         commit to"
    )]
    KeyTooLong {
        shard_group: ShardGroup,
        public_key: RistrettoPublicKeyBytes,
        scheme: KeyScheme,
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

    const VERSION: ProtocolVersion = ProtocolVersion::V2;

    fn key(scheme: u16, bytes: &[u8]) -> MemberKey {
        MemberKey {
            scheme: KeyScheme::from_u16(scheme),
            key: bytes.to_vec(),
        }
    }

    fn hash_members(
        shard_group: ShardGroup,
        members: &[(RistrettoPublicKeyBytes, u64, Vec<MemberKey>)],
    ) -> Result<Hash32, ValidatorSetError> {
        hash_validator_set_committee(
            VERSION,
            &shard_group,
            members
                .iter()
                .map(|(key, power, keys)| (key, VotePower::of(*power), keys.as_slice())),
        )
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
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        for (key_byte, power) in [(3u8, 2u64), (4, 1)] {
            body.extend_from_slice(&[key_byte; 32]);
            body.extend_from_slice(&power.to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
        }

        let leaf = committee(&[("c", 3, 2), ("d", 4, 1)])
            .calculate_validator_set_leaf_hash(VERSION, &ShardGroup::new(2u32, 3u32))
            .unwrap();
        assert_eq!(leaf, blake2b_256("ValidatorSetCommittee", &body));
    }

    #[test]
    fn the_leaf_hash_of_additional_keys_is_the_documented_preimage() {
        let mut body = Vec::new();
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&[3; 32]);
        body.extend_from_slice(&2u64.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&[0xa1, 0xa2, 0xa3]);
        body.extend_from_slice(&700u16.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());

        let leaf = hash_members(ShardGroup::new(2u32, 3u32), &[(pk(3), 2, vec![
            key(1, &[0xa1, 0xa2, 0xa3]),
            key(700, &[]),
        ])])
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
            committee(&[])
                .calculate_validator_set_leaf_hash(VERSION, &shard_group)
                .unwrap()
        );
    }

    #[test]
    fn the_leaf_ignores_member_addresses() {
        let shard_group = ShardGroup::new(2u32, 3u32);
        assert_eq!(
            committee(&[("c", 3, 2), ("d", 4, 1)])
                .calculate_validator_set_leaf_hash(VERSION, &shard_group)
                .unwrap(),
            committee(&[("x", 3, 2), ("y", 4, 1)])
                .calculate_validator_set_leaf_hash(VERSION, &shard_group)
                .unwrap()
        );
    }

    #[test]
    fn every_committed_field_changes_the_leaf_hash() {
        let shard_group = ShardGroup::new(2u32, 3u32);
        let leaf = |members: &[(&'static str, u8, u64)], shard_group| {
            committee(members)
                .calculate_validator_set_leaf_hash(VERSION, &shard_group)
                .unwrap()
        };
        let sample = leaf(&[("c", 3, 2), ("d", 4, 1)], shard_group);

        assert_ne!(leaf(&[("c", 3, 2), ("d", 4, 1)], ShardGroup::new(1u32, 3u32)), sample);
        assert_ne!(leaf(&[("c", 3, 1), ("d", 4, 1)], shard_group), sample);
        assert_ne!(leaf(&[("c", 3, 2), ("d", 5, 1)], shard_group), sample);
        assert_ne!(leaf(&[("c", 3, 2)], shard_group), sample);
        assert_ne!(
            committee(&[("c", 3, 2), ("d", 4, 1)])
                .calculate_validator_set_leaf_hash(ProtocolVersion::V1, &shard_group)
                .unwrap(),
            sample
        );
    }

    #[test]
    fn every_additional_key_field_changes_the_leaf_hash() {
        let shard_group = ShardGroup::new(2u32, 3u32);
        let leaf = |keys: Vec<MemberKey>| hash_members(shard_group, &[(pk(3), 2, keys)]).unwrap();
        let sample = leaf(vec![key(1, &[1, 2])]);

        assert_ne!(leaf(vec![]), sample);
        assert_ne!(leaf(vec![key(2, &[1, 2])]), sample);
        assert_ne!(leaf(vec![key(1, &[1, 3])]), sample);
        assert_ne!(leaf(vec![key(1, &[1])]), sample);
        assert_ne!(leaf(vec![key(1, &[1, 2]), key(2, &[])]), sample);
    }

    #[test]
    fn it_rejects_additional_keys_out_of_scheme_order() {
        let shard_group = ShardGroup::new(0u32, 3u32);
        let err = hash_members(shard_group, &[(pk(1), 1, vec![key(2, &[1]), key(1, &[1])])]).unwrap_err();

        assert_eq!(err, ValidatorSetError::KeysNotSorted {
            shard_group,
            public_key: pk(1),
            previous: KeyScheme::from_u16(2),
            scheme: KeyScheme::from_u16(1),
        });
    }

    #[test]
    fn it_rejects_a_repeated_additional_key_scheme() {
        let shard_group = ShardGroup::new(0u32, 3u32);
        let err = hash_members(shard_group, &[(pk(1), 1, vec![key(1, &[1]), key(1, &[2])])]).unwrap_err();

        assert_eq!(err, ValidatorSetError::KeysNotSorted {
            shard_group,
            public_key: pk(1),
            previous: KeyScheme::from_u16(1),
            scheme: KeyScheme::from_u16(1),
        });
    }

    #[test]
    fn it_rejects_an_additional_ristretto_key() {
        let shard_group = ShardGroup::new(0u32, 3u32);
        let err = hash_members(shard_group, &[(pk(1), 1, vec![key(0, &[1; 32])])]).unwrap_err();

        assert_eq!(err, ValidatorSetError::KeysNotSorted {
            shard_group,
            public_key: pk(1),
            previous: KeyScheme::RISTRETTO,
            scheme: KeyScheme::RISTRETTO,
        });
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
        let err = hash_members(shard_group, &[(pk(2), 1, vec![]), (pk(1), 1, vec![])]).unwrap_err();

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
            .calculate_validator_set_leaf_hash(VERSION, &shard_group)
            .unwrap_err();

        assert_eq!(err, ValidatorSetError::CommitteeNotSorted {
            shard_group,
            previous: pk(1),
            public_key: pk(1),
        });
    }

    #[test]
    fn a_member_without_additional_keys_encodes_only_its_key_and_power() {
        #[derive(Serialize)]
        struct KeyAndPower {
            public_key: RistrettoPublicKeyBytes,
            vote_power: VotePower,
        }
        let member = ValidatorSetMember {
            public_key: pk(3),
            vote_power: VotePower::of(2),
            additional_keys: vec![],
        };
        let key_and_power = KeyAndPower {
            public_key: pk(3),
            vote_power: VotePower::of(2),
        };

        let cbor = tari_bor::serde_codec::to_vec(&member).unwrap();
        assert_eq!(cbor, tari_bor::serde_codec::to_vec(&key_and_power).unwrap());
        assert_eq!(
            tari_bor::serde_codec::from_slice::<ValidatorSetMember>(&cbor).unwrap(),
            member
        );
        assert_eq!(
            serde_json::to_string(&member).unwrap(),
            serde_json::to_string(&key_and_power).unwrap()
        );
    }

    #[test]
    fn a_member_with_keys_of_any_scheme_round_trips() {
        let member = ValidatorSetMember {
            public_key: pk(3),
            vote_power: VotePower::of(2),
            additional_keys: vec![key(1, &[0xa1, 0xa2]), key(u16::MAX, &[0xff; 96])],
        };

        let cbor = tari_bor::serde_codec::to_vec(&member).unwrap();
        assert_eq!(
            tari_bor::serde_codec::from_slice::<ValidatorSetMember>(&cbor).unwrap(),
            member
        );
        let json = serde_json::to_string(&member).unwrap();
        assert!(json.contains(r#"{"scheme":1,"key":"a1a2"}"#), "{json}");
        assert_eq!(serde_json::from_str::<ValidatorSetMember>(&json).unwrap(), member);

        assert_eq!(member.key(KeyScheme::RISTRETTO), Some(pk(3).as_bytes()));
        assert_eq!(member.key(KeyScheme::from_u16(1)), Some(&[0xa1, 0xa2][..]));
        assert_eq!(member.key(KeyScheme::from_u16(2)), None);
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
