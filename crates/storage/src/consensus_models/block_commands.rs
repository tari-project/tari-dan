//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{cmp::Ordering, ops::Deref};

use minicbor::{CborLen, Decode, Decoder, Encode, Encoder, decode, encode};
use serde::{Deserialize, Deserializer, Serialize, de};

use super::Command;

/// The commands of a block, in canonical order: sorted by each command's ordering key (see [`Command`]), with no two
/// commands sharing a key. Replicas process a block's commands in this order, and the command merkle root commits to
/// it.
///
/// Every constructor and decoder upholds the invariant: [`Self::init`] sorts and rejects a repeated key, while
/// [`Self::try_from_canonical`] and decoding (serde and CBOR) reject any list not already in canonical order, which
/// keeps the encoding of a block canonical.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BlockCommands(Box<[Command]>);

impl BlockCommands {
    pub fn empty() -> Self {
        Self(Vec::new().into_boxed_slice())
    }

    /// Sorts `commands` into canonical order. Two commands with the same ordering key (e.g. a `LocalPrepare` and a
    /// `LocalAccept` for one transaction) are an error.
    pub fn init<I: IntoIterator<Item = Command>>(commands: I) -> Result<Self, BlockCommandsError> {
        let mut commands = commands.into_iter().collect::<Vec<_>>();
        commands.sort_by(|a, b| a.as_ordering().cmp(&b.as_ordering()));
        if let Some(pair) = commands
            .windows(2)
            .find(|pair| pair[0].as_ordering() == pair[1].as_ordering())
        {
            return Err(BlockCommandsError::DuplicateKey {
                first: pair[0].to_string(),
                second: pair[1].to_string(),
            });
        }
        Ok(Self(commands.into_boxed_slice()))
    }

    /// Accepts `commands` only if they are already in canonical order, i.e. their ordering keys strictly increase.
    pub fn try_from_canonical(commands: Vec<Command>) -> Result<Self, BlockCommandsError> {
        for pair in commands.windows(2) {
            match pair[0].as_ordering().cmp(&pair[1].as_ordering()) {
                Ordering::Less => {},
                Ordering::Equal => {
                    return Err(BlockCommandsError::DuplicateKey {
                        first: pair[0].to_string(),
                        second: pair[1].to_string(),
                    });
                },
                Ordering::Greater => {
                    return Err(BlockCommandsError::OutOfOrder {
                        previous: pair[0].to_string(),
                        command: pair[1].to_string(),
                    });
                },
            }
        }
        Ok(Self(commands.into_boxed_slice()))
    }

    pub fn as_slice(&self) -> &[Command] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<Command> {
        self.0.into_vec()
    }
}

impl Deref for BlockCommands {
    type Target = [Command];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IntoIterator for BlockCommands {
    type IntoIter = std::vec::IntoIter<Command>;
    type Item = Command;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a BlockCommands {
    type IntoIter = std::slice::Iter<'a, Command>;
    type Item = &'a Command;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'de> Deserialize<'de> for BlockCommands {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let commands = Vec::<Command>::deserialize(deserializer)?;
        Self::try_from_canonical(commands).map_err(de::Error::custom)
    }
}

impl<C> Encode<C> for BlockCommands {
    fn encode<W: encode::Write>(&self, e: &mut Encoder<W>, ctx: &mut C) -> Result<(), encode::Error<W::Error>> {
        self.0.encode(e, ctx)
    }
}

impl<'b, C> Decode<'b, C> for BlockCommands {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Self, decode::Error> {
        let commands = Vec::<Command>::decode(d, ctx)?;
        Self::try_from_canonical(commands).map_err(|e| decode::Error::message(e.to_string()))
    }
}

impl<C> CborLen<C> for BlockCommands {
    fn cbor_len(&self, ctx: &mut C) -> usize {
        self.0.cbor_len(ctx)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum BlockCommandsError {
    #[error("Commands {first} and {second} share an ordering key, so a block cannot hold both")]
    DuplicateKey { first: String, second: String },
    #[error("Command {command} is listed after {previous}, but canonical order places it first")]
    OutOfOrder { previous: String, command: String },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use tari_consensus_types::{BlockId, Decision};
    use tari_ootle_common_types::{ProtocolVersion, ShardGroup};
    use tari_ootle_transaction::TransactionId;
    use tari_sidechain::FixedHash;

    use super::*;
    use crate::consensus_models::{
        BlockHeader,
        EndEpochAtom,
        Evidence,
        ForeignProposalAtom,
        LocalOnlyAtom,
        MultiShardAtom,
    };

    fn multi_shard(seed: u8) -> MultiShardAtom {
        MultiShardAtom {
            id: TransactionId::new([seed; 32]),
            decision: Decision::Commit,
            evidence: Evidence::default(),
            transaction_fee: 0,
            leader_fee: None,
        }
    }

    fn local_only(seed: u8) -> Command {
        Command::LocalOnly(LocalOnlyAtom {
            id: TransactionId::new([seed; 32]),
            decision: Decision::Commit,
            transaction_fee: 0,
            leader_fee: None,
        })
    }

    fn end_epoch(seed: u8) -> Command {
        Command::EndEpoch(EndEpochAtom::new(FixedHash::from([seed; 32])))
    }

    fn foreign_proposal(seed: u8) -> Command {
        Command::ForeignProposal(ForeignProposalAtom {
            block_id: BlockId::from([seed; 32]),
            shard_group: ShardGroup::new(0, 31),
        })
    }

    /// One command per kind of ordering key, listed out of canonical order.
    fn mixed() -> Vec<Command> {
        vec![
            end_epoch(1),
            Command::AllAccept(multi_shard(3)),
            local_only(2),
            foreign_proposal(9),
            Command::SomeAccept(multi_shard(4)),
            Command::LocalPrepare(multi_shard(1)),
            Command::LocalAccept(multi_shard(5)),
            foreign_proposal(7),
        ]
    }

    /// A command ordered by its ordering key, so that a `BTreeSet` of these is the sorted, key-unique container whose
    /// encoding `BlockCommands` must reproduce.
    #[derive(PartialEq, Eq, serde::Serialize)]
    #[serde(transparent)]
    struct Keyed(Command);

    impl<C> Encode<C> for Keyed {
        fn encode<W: encode::Write>(&self, e: &mut Encoder<W>, ctx: &mut C) -> Result<(), encode::Error<W::Error>> {
            self.0.encode(e, ctx)
        }
    }

    impl<C> CborLen<C> for Keyed {
        fn cbor_len(&self, ctx: &mut C) -> usize {
            self.0.cbor_len(ctx)
        }
    }

    impl PartialOrd for Keyed {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Keyed {
        fn cmp(&self, other: &Self) -> Ordering {
            self.0.as_ordering().cmp(&other.0.as_ordering())
        }
    }

    #[test]
    fn init_sorts_into_canonical_order() {
        let commands = BlockCommands::init(mixed()).unwrap();
        let keys = commands.iter().map(Command::as_ordering).collect::<Vec<_>>();
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(commands.first().unwrap().foreign_proposal().is_some());
        assert!(commands.last().unwrap().is_epoch_end());
    }

    #[test]
    fn init_rejects_two_phases_of_one_transaction() {
        let err = BlockCommands::init([
            Command::LocalPrepare(multi_shard(1)),
            Command::LocalAccept(multi_shard(1)),
        ])
        .unwrap_err();
        assert!(matches!(err, BlockCommandsError::DuplicateKey { .. }), "{err}");
    }

    #[test]
    fn init_rejects_two_end_epoch_commands() {
        let err = BlockCommands::init([end_epoch(1), end_epoch(2)]).unwrap_err();
        assert!(matches!(err, BlockCommandsError::DuplicateKey { .. }), "{err}");
    }

    #[test]
    fn try_from_canonical_accepts_sorted_commands() {
        let sorted = BlockCommands::init(mixed()).unwrap().into_vec();
        assert_eq!(
            BlockCommands::try_from_canonical(sorted.clone()).unwrap().into_vec(),
            sorted
        );
    }

    #[test]
    fn try_from_canonical_rejects_out_of_order_commands() {
        let mut commands = BlockCommands::init(mixed()).unwrap().into_vec();
        commands.swap(2, 3);
        let err = BlockCommands::try_from_canonical(commands).unwrap_err();
        assert!(matches!(err, BlockCommandsError::OutOfOrder { .. }), "{err}");
    }

    #[test]
    fn try_from_canonical_rejects_a_repeated_key() {
        let err = BlockCommands::try_from_canonical(vec![
            Command::LocalPrepare(multi_shard(1)),
            Command::LocalAccept(multi_shard(1)),
        ])
        .unwrap_err();
        assert!(matches!(err, BlockCommandsError::DuplicateKey { .. }), "{err}");
    }

    #[test]
    fn decoding_rejects_non_canonical_commands() {
        let out_of_order = vec![Command::LocalPrepare(multi_shard(2)), local_only(1)];
        let bytes = minicbor::to_vec(&out_of_order).unwrap();
        assert!(minicbor::decode::<BlockCommands>(&bytes).is_err());

        let json = serde_json::to_string(&out_of_order).unwrap();
        assert!(serde_json::from_str::<BlockCommands>(&json).is_err());
    }

    #[test]
    fn encoding_and_roots_match_a_sorted_set() {
        let commands = BlockCommands::init(mixed()).unwrap();
        let set = mixed().into_iter().map(Keyed).collect::<BTreeSet<_>>();
        assert_eq!(set.len(), commands.len());

        assert_eq!(minicbor::to_vec(&commands).unwrap(), minicbor::to_vec(&set).unwrap());
        assert_eq!(minicbor::len(&commands), minicbor::len(&set));
        assert_eq!(
            serde_json::to_string(&commands).unwrap(),
            serde_json::to_string(&set).unwrap()
        );
        let set_commands = set.iter().map(|k| &k.0).collect::<Vec<_>>();
        assert_eq!(
            borsh::to_vec(&commands.iter().collect::<Vec<_>>()).unwrap(),
            borsh::to_vec(&set_commands).unwrap()
        );

        let decoded = minicbor::decode::<BlockCommands>(&minicbor::to_vec(&set).unwrap()).unwrap();
        assert_eq!(decoded, commands);

        for protocol_version in [
            ProtocolVersion::V0,
            ProtocolVersion::V1,
            ProtocolVersion::V2,
            ProtocolVersion::V3,
        ] {
            let set_root = tari_state_tree::compute_merkle_root_for_hashes(
                set_commands
                    .iter()
                    .map(|cmd| tari_state_tree::TreeHash::from(cmd.merkle_leaf(protocol_version).into_array())),
            )
            .unwrap();
            assert_eq!(
                BlockHeader::compute_command_merkle_root(protocol_version, &commands)
                    .unwrap()
                    .into_array(),
                set_root.into_array(),
                "command merkle root differs under {protocol_version}"
            );
        }
    }
}
