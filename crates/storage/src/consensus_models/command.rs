//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use borsh::BorshSerialize;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::{ProtocolVersion, ShardGroup, hashing::command_hasher};
use tari_ootle_transaction::TransactionId;

use super::{ForeignProposalAtom, LeaderFee, TransactionRecord};
use crate::{StateStoreReadTransaction, StorageError, consensus_models::evidence::Evidence};

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MultiShardAtom {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(0)]
    pub id: TransactionId,
    #[n(1)]
    pub decision: Decision,
    #[n(2)]
    pub evidence: Evidence,
    #[n(3)]
    pub transaction_fee: u64,
    #[n(4)]
    pub leader_fee: Option<LeaderFee>,
}

impl MultiShardAtom {
    pub fn id(&self) -> &TransactionId {
        &self.id
    }

    pub fn get_transaction<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<TransactionRecord, StorageError> {
        TransactionRecord::get(tx, &self.id)
    }
}

impl Display for MultiShardAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MultiShardAtom({}, {}, {}, ",
            self.id, self.decision, self.transaction_fee,
        )?;
        match self.leader_fee {
            Some(ref leader_fee) => write!(f, "{}", leader_fee)?,
            None => write!(f, "--")?,
        }
        write!(f, ")")
    }
}

/// A local-only transaction as sequenced in a block. Every replica derives the transaction's evidence from its own
/// execution, so the command carries only the outcome the committee votes on.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct LocalOnlyAtom {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[n(0)]
    pub id: TransactionId,
    #[n(1)]
    pub decision: Decision,
    #[n(2)]
    pub transaction_fee: u64,
    #[n(3)]
    pub leader_fee: Option<LeaderFee>,
}

impl LocalOnlyAtom {
    pub fn id(&self) -> &TransactionId {
        &self.id
    }
}

impl Display for LocalOnlyAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        TransactionAtomRef::LocalOnly(self).fmt(f)
    }
}

/// A transaction command's atom, whichever form the command carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionAtomRef<'a> {
    LocalOnly(&'a LocalOnlyAtom),
    MultiShard(&'a MultiShardAtom),
}

impl<'a> TransactionAtomRef<'a> {
    pub fn id(&self) -> &'a TransactionId {
        match self {
            Self::LocalOnly(atom) => &atom.id,
            Self::MultiShard(atom) => &atom.id,
        }
    }

    pub fn decision(&self) -> Decision {
        match self {
            Self::LocalOnly(atom) => atom.decision,
            Self::MultiShard(atom) => atom.decision,
        }
    }

    pub fn transaction_fee(&self) -> u64 {
        match self {
            Self::LocalOnly(atom) => atom.transaction_fee,
            Self::MultiShard(atom) => atom.transaction_fee,
        }
    }

    pub fn leader_fee(&self) -> Option<&'a LeaderFee> {
        match self {
            Self::LocalOnly(atom) => atom.leader_fee.as_ref(),
            Self::MultiShard(atom) => atom.leader_fee.as_ref(),
        }
    }

    /// The evidence the command carries. A local-only command carries none: its evidence is the local shard group
    /// alone, derived by each replica from its own execution.
    pub fn evidence(&self) -> Option<&'a Evidence> {
        match self {
            Self::LocalOnly(_) => None,
            Self::MultiShard(atom) => Some(&atom.evidence),
        }
    }

    pub fn get_transaction<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<TransactionRecord, StorageError> {
        TransactionRecord::get(tx, self.id())
    }
}

impl Display for TransactionAtomRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalOnly(atom) => {
                write!(
                    f,
                    "LocalOnlyAtom({}, {}, {}, ",
                    atom.id, atom.decision, atom.transaction_fee,
                )?;
                match atom.leader_fee {
                    Some(ref leader_fee) => write!(f, "{}", leader_fee)?,
                    None => write!(f, "--")?,
                }
                write!(f, ")")
            },
            Self::MultiShard(atom) => atom.fmt(f),
        }
    }
}

/// Discriminants are explicit and load-bearing: a command's hash is its Borsh encoding, and the base
/// layer recomputes that hash from [`tari_sidechain::Command`] when verifying an end-of-epoch
/// inclusion proof. Each variant must therefore keep the discriminant its counterpart has there,
/// which is that enum's declaration order — 6 is absent here because the sidechain enum still
/// declares a variant this one does not. Renumbering a variant invalidates every proof over it; the
/// tests at the foot of this module hold the two enums to the same values.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Command {
    // Transaction Commands
    /// Request validators to prepare a local-only transaction
    #[n(0)]
    LocalOnly(#[n(0)] LocalOnlyAtom) = 0,
    /// Request validators to prepare a transaction.
    #[n(1)]
    LocalPrepare(#[n(0)] MultiShardAtom) = 1,
    /// Request validators to  agree that all involved shard groups prepared the transaction and
    /// accept (i.e. accept COMMIT/ABORT decision) a transaction. All foreign inputs are received
    /// and the transaction is executed with the same decision.
    #[n(2)]
    LocalAccept(#[n(0)] MultiShardAtom) = 2,
    /// Request validators to agree that all involved shard groups agreed to ACCEPT the transaction.
    #[n(3)]
    AllAccept(#[n(0)] MultiShardAtom) = 3,
    /// Request validators to agree that one or more involved shard groups did not agreed to ACCEPT the transaction.
    #[n(4)]
    SomeAccept(#[n(0)] MultiShardAtom) = 4,
    // Validator node commands
    #[n(5)]
    ForeignProposal(#[n(0)] ForeignProposalAtom) = 5,
    #[n(7)]
    EndEpoch(#[n(0)] EndEpochAtom) = 7,
}

/// Defines the order in which commands should be processed in a block. "Smallest" comes first and "largest" comes last.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CommandOrdering<'a> {
    /// Foreign proposals should come first in the block so that they are processed before commands
    ForeignProposal(ShardGroup, &'a BlockId),
    TransactionId(&'a TransactionId),
    EndEpoch,
}

impl Command {
    pub fn transaction(&self) -> Option<TransactionAtomRef<'_>> {
        match self {
            Command::LocalPrepare(tx) | Command::LocalAccept(tx) | Command::AllAccept(tx) | Command::SomeAccept(tx) => {
                Some(TransactionAtomRef::MultiShard(tx))
            },
            Command::LocalOnly(tx) => Some(TransactionAtomRef::LocalOnly(tx)),
            Command::ForeignProposal(_) | Command::EndEpoch(_) => None,
        }
    }

    /// The percentage of a transaction's static weight that proposing/processing this command costs,
    /// reflecting whether the command executes the transaction. Phases that execute (or carry the
    /// transaction's full footprint) cost 100%; finalisation phases reuse a prior execution and only
    /// apply its diff, so they are discounted. Mirrors [`TransactionPoolStage::proposal_weight_percent`]
    /// keyed on the command rather than the pool stage, so a block's execution weight is computable
    /// deterministically from its commands alone. Non-transaction commands contribute 0.
    pub fn execution_weight_percent(&self) -> u64 {
        match self {
            Command::LocalOnly(_) | Command::LocalPrepare(_) | Command::LocalAccept(_) => 100,
            Command::AllAccept(_) | Command::SomeAccept(_) => 35,
            Command::ForeignProposal(_) | Command::EndEpoch(_) => 0,
        }
    }

    /// The key [`BlockCommands`](super::BlockCommands) orders a block's commands by. Commands that share a key
    /// cannot appear in the same block.
    pub(super) fn as_ordering(&self) -> CommandOrdering<'_> {
        match self {
            Command::LocalPrepare(tx) | Command::LocalAccept(tx) | Command::AllAccept(tx) | Command::SomeAccept(tx) => {
                CommandOrdering::TransactionId(&tx.id)
            },
            Command::LocalOnly(tx) => CommandOrdering::TransactionId(&tx.id),
            Command::ForeignProposal(foreign_proposal) => {
                // Order by shard group then by block id
                CommandOrdering::ForeignProposal(foreign_proposal.shard_group, &foreign_proposal.block_id)
            },
            Command::EndEpoch(_) => CommandOrdering::EndEpoch,
        }
    }

    /// The command's leaf in the command merkle root of a block proposed under `protocol_version`.
    ///
    /// From [`ProtocolVersion::V2`] the preimage of a command that carries evidence also commits to the pledged value
    /// hash of every input. The borsh encoding of the command fixes the number and order of the inputs, so the list
    /// that follows it is unambiguous.
    pub fn hash(&self, protocol_version: ProtocolVersion) -> FixedHash {
        let hasher = command_hasher().chain(self);
        match protocol_version {
            ProtocolVersion::V0 | ProtocolVersion::V1 => hasher.finalize().into(),
            ProtocolVersion::V2 => match self {
                Command::LocalPrepare(atom) |
                Command::LocalAccept(atom) |
                Command::AllAccept(atom) |
                Command::SomeAccept(atom) => hasher.chain(&atom.evidence.pledged_value_hashes()).finalize().into(),
                Command::LocalOnly(_) | Command::ForeignProposal(_) | Command::EndEpoch(_) => hasher.finalize().into(),
            },
        }
    }

    pub fn local_only(&self) -> Option<&LocalOnlyAtom> {
        match self {
            Command::LocalOnly(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn local_prepare(&self) -> Option<&MultiShardAtom> {
        match self {
            Command::LocalPrepare(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn local_accept(&self) -> Option<&MultiShardAtom> {
        match self {
            Command::LocalAccept(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn foreign_proposal(&self) -> Option<&ForeignProposalAtom> {
        match self {
            Command::ForeignProposal(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn end_epoch(&self) -> Option<&EndEpochAtom> {
        match self {
            Command::EndEpoch(atom) => Some(atom),
            _ => None,
        }
    }

    pub fn all_accept(&self) -> Option<&MultiShardAtom> {
        match self {
            Command::AllAccept(tx) => Some(tx),
            _ => None,
        }
    }

    pub fn some_accept(&self) -> Option<&MultiShardAtom> {
        match self {
            Command::SomeAccept(tx) => Some(tx),
            _ => None,
        }
    }

    /// Returns Some if the command should result in finalising (COMMITing or ABORTing) the transaction, otherwise None.
    pub fn finalising(&self) -> Option<TransactionAtomRef<'_>> {
        self.all_accept()
            .or_else(|| self.some_accept())
            .map(TransactionAtomRef::MultiShard)
            .or_else(|| self.local_only().map(TransactionAtomRef::LocalOnly))
    }

    /// Returns Some if the command should result in committing the transaction, otherwise None.
    pub fn committing(&self) -> Option<TransactionAtomRef<'_>> {
        self.all_accept()
            .map(TransactionAtomRef::MultiShard)
            .or_else(|| self.local_only().map(TransactionAtomRef::LocalOnly))
            .filter(|t| t.decision().is_commit())
    }

    /// Returns `local_shard_group`'s portion of the transaction's exhaust burn for accumulation into the block
    /// header's burn total. Returns 0 if this command does not commit a transaction or carries no leader fee.
    ///
    /// A multi-shard burn is split between the shard groups in the atom's evidence — see
    /// [Evidence::exhaust_burn_portion]. A LocalOnly transaction involves only the local shard group, so its portion
    /// is the entire burn.
    ///
    /// Must only be called on locally-constructed commands (e.g. while proposing): the split relies on the locally
    /// maintained evidence key order, which wire-decoded commands do not guarantee.
    ///
    /// Returns `None` if this is a committing multi-shard command whose evidence does not include
    /// `local_shard_group`.
    pub fn exhaust_burn_portion(&self, local_shard_group: ShardGroup) -> Option<u64> {
        let Some(atom) = self.committing() else {
            return Some(0);
        };
        let Some(leader_fee) = atom.leader_fee() else {
            return Some(0);
        };
        match atom.evidence() {
            Some(evidence) => evidence.exhaust_burn_portion(leader_fee.exhaust_burn(), local_shard_group),
            None => Some(leader_fee.exhaust_burn()),
        }
    }

    /// Returns Some if the command **will** result in aborting the transaction, otherwise None.
    pub fn aborting(&self) -> Option<TransactionAtomRef<'_>> {
        self.some_accept()
            .or_else(|| self.local_prepare())
            .or_else(|| self.local_accept())
            .map(TransactionAtomRef::MultiShard)
            .or_else(|| self.local_only().map(TransactionAtomRef::LocalOnly))
            .filter(|t| t.decision().is_abort())
    }

    pub fn is_epoch_end(&self) -> bool {
        matches!(self, Command::EndEpoch(_))
    }

    pub fn is_local_prepare(&self) -> bool {
        matches!(self, Command::LocalPrepare(_))
    }

    pub fn is_local_accept(&self) -> bool {
        matches!(self, Command::LocalAccept(_))
    }
}

impl Display for Command {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::LocalOnly(tx) => write!(f, "LocalOnly({}, {})", tx.id, tx.decision),
            Command::LocalPrepare(tx) => write!(f, "LocalPrepare({}, {})", tx.id, tx.decision),
            Command::LocalAccept(tx) => write!(f, "LocalAccept({}, {})", tx.id, tx.decision),
            Command::AllAccept(tx) => write!(f, "AllAccept({}, {})", tx.id, tx.decision),
            Command::SomeAccept(tx) => write!(f, "SomeAccept({}, {})", tx.id, tx.decision),
            Command::ForeignProposal(fp) => write!(f, "ForeignProposal {}", fp.block_id),
            Command::EndEpoch(atom) => write!(f, "EndEpoch({atom})"),
        }
    }
}

/// The atom committed by an [`Command::EndEpoch`] command.
///
/// Carries the base-layer boundary-block hash of the *next* epoch. Because the hash is part of the
/// command, it is committed in the block's command merkle root and ratified by the quorum that
/// commits the end-of-epoch block: a validator only votes for the EOE block if `next_epoch_hash`
/// matches its own (lagged, reorg-stable) oracle view. This prevents a node from unilaterally
/// locking an epoch hash the committee never agreed on — the failure mode that wedges consensus when
/// a base-layer reorg deeper than the confirmation depth straddles an epoch boundary.
///
/// The Borsh layout (a single 32-byte hash) is identical to [`tari_sidechain::EndEpochAtom`] so the
/// layer-2 `command_hasher` output matches what L1 recomputes during checkpoint inclusion-proof
/// verification.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
pub struct EndEpochAtom {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(with = "ootle_serde::hex")]
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    pub next_epoch_hash: FixedHash,
}

impl EndEpochAtom {
    pub fn new(next_epoch_hash: FixedHash) -> Self {
        Self { next_epoch_hash }
    }

    pub fn next_epoch_hash(&self) -> &FixedHash {
        &self.next_epoch_hash
    }
}

impl Display for EndEpochAtom {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "next_epoch_hash={}", self.next_epoch_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus_models::BlockCommands;

    #[test]
    fn ordering() {
        assert!(
            CommandOrdering::ForeignProposal(ShardGroup::new(32, 63), &BlockId::zero()) >
                CommandOrdering::ForeignProposal(ShardGroup::new(0, 31), &BlockId::zero())
        );
        assert!(
            CommandOrdering::ForeignProposal(ShardGroup::new(0, 64), &BlockId::zero()) <
                CommandOrdering::TransactionId(&TransactionId::default())
        );
        let tx_id = TransactionId::new([1; 32]);

        assert!(CommandOrdering::TransactionId(&TransactionId::default()) < CommandOrdering::TransactionId(&tx_id));
        assert!(CommandOrdering::TransactionId(&tx_id) < CommandOrdering::EndEpoch);
        let cmds = [
            Command::EndEpoch(EndEpochAtom::new(FixedHash::zero())),
            Command::AllAccept(MultiShardAtom {
                id: TransactionId::new([1; 32]),
                decision: Decision::Commit,
                evidence: Evidence::default(),
                transaction_fee: 0,
                leader_fee: None,
            }),
            Command::ForeignProposal(ForeignProposalAtom {
                block_id: BlockId::zero(),
                shard_group: ShardGroup::new(0, 64),
            }),
            Command::LocalPrepare(MultiShardAtom {
                id: TransactionId::default(),
                decision: Decision::Commit,
                evidence: Evidence::default(),
                transaction_fee: 0,
                leader_fee: None,
            }),
        ];
        let expected = [cmds[2].clone(), cmds[3].clone(), cmds[1].clone(), cmds[0].clone()];
        let commands = BlockCommands::init(cmds).unwrap();

        let mut iter = commands.iter();
        for exp in &expected {
            let next = iter.next().unwrap();
            assert_eq!(next, exp);
        }
    }

    #[test]
    fn execution_weight_percent() {
        let atom = || MultiShardAtom {
            id: TransactionId::default(),
            decision: Decision::Commit,
            evidence: Evidence::default(),
            transaction_fee: 0,
            leader_fee: None,
        };
        let local_only_atom = LocalOnlyAtom {
            id: TransactionId::default(),
            decision: Decision::Commit,
            transaction_fee: 0,
            leader_fee: None,
        };
        // Executing phases charge the full transaction weight.
        assert_eq!(Command::LocalOnly(local_only_atom).execution_weight_percent(), 100);
        assert_eq!(Command::LocalPrepare(atom()).execution_weight_percent(), 100);
        assert_eq!(Command::LocalAccept(atom()).execution_weight_percent(), 100);
        // Finalisation phases reuse a prior execution and are discounted.
        assert_eq!(Command::AllAccept(atom()).execution_weight_percent(), 35);
        assert_eq!(Command::SomeAccept(atom()).execution_weight_percent(), 35);
        // Non-transaction commands carry no transaction execution weight.
        assert_eq!(
            Command::ForeignProposal(ForeignProposalAtom {
                block_id: BlockId::zero(),
                shard_group: ShardGroup::new(0, 64),
            })
            .execution_weight_percent(),
            0
        );
        assert_eq!(
            Command::EndEpoch(EndEpochAtom::new(FixedHash::zero())).execution_weight_percent(),
            0
        );
    }
}

#[cfg(test)]
mod borsh_discriminant_tests {
    use super::*;

    /// The base layer recomputes a command's hash from [`tari_sidechain::Command`] when verifying an
    /// end-of-epoch inclusion proof, hashing the Borsh encoding. The discriminant each variant here
    /// serialises to must therefore equal the one the sidechain enum serialises to, which is that
    /// enum's declaration order. The explicit discriminants exist to hold that correspondence while
    /// the two enums list different variants.
    fn multi_shard_atom() -> MultiShardAtom {
        MultiShardAtom {
            id: TransactionId::default(),
            decision: Decision::Commit,
            evidence: Evidence::default(),
            transaction_fee: 0,
            leader_fee: None,
        }
    }

    fn local_only_atom() -> LocalOnlyAtom {
        LocalOnlyAtom {
            id: TransactionId::default(),
            decision: Decision::Commit,
            transaction_fee: 0,
            leader_fee: None,
        }
    }

    fn foreign_proposal_atom() -> ForeignProposalAtom {
        ForeignProposalAtom {
            block_id: BlockId::zero(),
            shard_group: ShardGroup::all_shards(tari_ootle_common_types::NumPreshards::P256),
        }
    }

    #[test]
    fn discriminants_match_the_sidechain_enum() {
        let cases: [(Command, tari_sidechain::Command); 7] = [
            (
                Command::LocalOnly(local_only_atom()),
                tari_sidechain::Command::LocalOnly,
            ),
            (
                Command::LocalPrepare(multi_shard_atom()),
                tari_sidechain::Command::LocalPrepare,
            ),
            (
                Command::LocalAccept(multi_shard_atom()),
                tari_sidechain::Command::LocalAccept,
            ),
            (
                Command::AllAccept(multi_shard_atom()),
                tari_sidechain::Command::AllAccept,
            ),
            (
                Command::SomeAccept(multi_shard_atom()),
                tari_sidechain::Command::SomeAccept,
            ),
            (
                Command::ForeignProposal(foreign_proposal_atom()),
                tari_sidechain::Command::ForeignProposal,
            ),
            (
                Command::EndEpoch(EndEpochAtom::new(FixedHash::zero())),
                tari_sidechain::Command::EndEpoch(tari_sidechain::EndEpochAtom::new(FixedHash::zero())),
            ),
        ];

        for (ours, theirs) in cases {
            let mut ours_bytes = Vec::new();
            BorshSerialize::serialize(&ours, &mut ours_bytes).unwrap();
            let mut theirs_bytes = Vec::new();
            BorshSerialize::serialize(&theirs, &mut theirs_bytes).unwrap();

            assert_eq!(
                ours_bytes[0], theirs_bytes[0],
                "discriminant mismatch for {ours}: {} != {}",
                ours_bytes[0], theirs_bytes[0]
            );
        }
    }

    /// The whole command, not just its tag, is what the base layer hashes for an end-of-epoch proof.
    #[test]
    fn an_end_epoch_command_hashes_identically_on_both_sides() {
        let next_epoch_hash = FixedHash::zero();

        for protocol_version in [ProtocolVersion::V0, ProtocolVersion::V1, ProtocolVersion::V2] {
            assert_eq!(
                Command::EndEpoch(EndEpochAtom::new(next_epoch_hash)).hash(protocol_version),
                tari_sidechain::Command::EndEpoch(tari_sidechain::EndEpochAtom::new(next_epoch_hash)).hash(),
            );
        }
    }
}

#[cfg(test)]
mod pledged_value_hash_tests {
    use tari_engine_types::{SubstateVersion, substate::SubstateId};
    use tari_ootle_common_types::SubstateLockType;
    use tari_template_lib_types::{ComponentAddress, Hash32, ObjectKey};

    use super::*;
    use crate::consensus_models::EvidenceInputLockData;

    fn local_prepare(pledged_value_hash: Option<Hash32>) -> Command {
        let mut evidence = Evidence::empty();
        evidence.add_shard_group(ShardGroup::new(0, 31)).insert(
            SubstateId::Component(ComponentAddress::from_array([1; ObjectKey::LENGTH])),
            SubstateVersion::ZERO,
            SubstateLockType::Write,
            pledged_value_hash,
        );
        Command::LocalPrepare(MultiShardAtom {
            id: TransactionId::default(),
            decision: Decision::Commit,
            evidence,
            transaction_fee: 0,
            leader_fee: None,
        })
    }

    /// Blocks proposed before V2 were signed over this layout, so it must not change.
    #[test]
    fn an_input_lock_encodes_as_its_write_flag_and_version() {
        let lock = EvidenceInputLockData {
            is_write: true,
            version: SubstateVersion::new(7),
            pledged_value_hash: Some(Hash32::from_array([9; 32])),
        };
        assert_eq!(
            borsh::to_vec(&lock).unwrap(),
            borsh::to_vec(&(true, SubstateVersion::new(7))).unwrap()
        );
    }

    #[test]
    fn a_command_hash_before_v2_does_not_cover_the_pledged_value_hash() {
        for protocol_version in [ProtocolVersion::V0, ProtocolVersion::V1] {
            assert_eq!(
                local_prepare(None).hash(protocol_version),
                local_prepare(Some(Hash32::from_array([1; 32]))).hash(protocol_version),
            );
        }
    }

    #[test]
    fn a_v2_command_hash_covers_the_pledged_value_hash() {
        let hashes = [
            local_prepare(None).hash(ProtocolVersion::V2),
            local_prepare(Some(Hash32::from_array([1; 32]))).hash(ProtocolVersion::V2),
            local_prepare(Some(Hash32::from_array([2; 32]))).hash(ProtocolVersion::V2),
        ];
        assert_ne!(hashes[0], hashes[1]);
        assert_ne!(hashes[0], hashes[2]);
        assert_ne!(hashes[1], hashes[2]);
        assert_ne!(hashes[0], local_prepare(None).hash(ProtocolVersion::V1));
    }
}
