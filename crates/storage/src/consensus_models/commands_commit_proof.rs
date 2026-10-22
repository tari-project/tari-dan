//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use minicbor::{CborLen, Decode, Encode};
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::{ProtocolVersion, UnknownProtocolVersionError};
use tari_ootle_common_types::{ShardGroup, VotePower};
use tari_sidechain::{
    CommitProofElement,
    FixedHash,
    SidechainBlockCommitProof,
    SidechainProofValidationError,
    SigningCommittee,
};
use tari_state_tree::{StateTreeError, TreeHash, compute_merkle_root_for_hashes};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::consensus_models::{Command, command::command_leaf};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Encode, Decode, CborLen)]
pub enum CommandsCommitProof {
    #[n(0)]
    V1(#[n(0)] CommandsCommitProofV1),
}

impl CommandsCommitProof {
    pub fn new_latest(commands: Vec<CommandOrHash>, commit_proof: SidechainBlockCommitProof) -> Self {
        Self::new_v1(commands, commit_proof)
    }

    pub fn new_v1(commands: Vec<CommandOrHash>, commit_proof: SidechainBlockCommitProof) -> Self {
        Self::V1(CommandsCommitProofV1 { commands, commit_proof })
    }

    pub fn commands(&self) -> &[CommandOrHash] {
        match self {
            Self::V1(proof) => proof.commands(),
        }
    }

    pub fn applicable_commands_iter(&self) -> impl Iterator<Item = &Command> + '_ {
        self.commands().iter().filter_map(|cmd| cmd.command())
    }

    /// The first command the proof carries only by its hash although `shard_group` needs it in full. The command's
    /// leaf authenticates the shard groups it names, so a proof that withholds a command cannot hide it.
    pub fn withheld_command_for(&self, shard_group: ShardGroup) -> Option<&PrunedCommand> {
        self.commands().iter().find_map(|cmd| match cmd {
            CommandOrHash::Pruned(pruned) if pruned.full_command_shard_groups.contains(&shard_group) => Some(pruned),
            CommandOrHash::Command(_) | CommandOrHash::Pruned(_) => None,
        })
    }

    pub fn sidechain_block_commit_proof(&self) -> &SidechainBlockCommitProof {
        match self {
            Self::V1(proof) => proof.commit_proof(),
        }
    }

    pub fn calculate_block_id(&self) -> FixedHash {
        match self {
            Self::V1(proof) => proof.commit_proof().header.calculate_block_id(),
        }
    }

    /// The protocol version of the committed block, which selects how its command hashes are formed.
    pub fn protocol_version(&self) -> Result<ProtocolVersion, UnknownProtocolVersionError> {
        match self {
            Self::V1(proof) => proof.protocol_version(),
        }
    }

    pub fn validate_header(&self) -> Result<(), ForeignProposalCommitProofError> {
        match self {
            Self::V1(proof) => proof.validate_header(),
        }
    }

    /// Validates the proof against `committee`, the committee expected to have signed its certificates.
    pub fn validate_committed(
        &self,
        network: Network,
        quorum_threshold: VotePower,
        committee: &dyn SigningCommittee,
    ) -> Result<(), ForeignProposalCommitProofError> {
        match self {
            Self::V1(proof) => proof.validate_committed(network, quorum_threshold, committee),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Encode, Decode, CborLen)]
pub struct CommandsCommitProofV1 {
    #[n(0)]
    pub commands: Vec<CommandOrHash>,
    #[n(1)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    pub commit_proof: SidechainBlockCommitProof,
}

impl CommandsCommitProofV1 {
    pub fn commands(&self) -> &[CommandOrHash] {
        &self.commands
    }

    pub fn commit_proof(&self) -> &SidechainBlockCommitProof {
        &self.commit_proof
    }

    pub fn protocol_version(&self) -> Result<ProtocolVersion, UnknownProtocolVersionError> {
        ProtocolVersion::try_from(self.commit_proof.header.protocol_version)
    }

    pub fn validate_committed(
        &self,
        network: Network,
        quorum_threshold: VotePower,
        committee: &dyn SigningCommittee,
    ) -> Result<(), ForeignProposalCommitProofError> {
        self.validate_command_merkle_root()?;
        self.commit_proof
            .validate_committed(network.as_byte(), quorum_threshold.value(), committee)?;
        Ok(())
    }

    pub fn validate_header(&self) -> Result<(), ForeignProposalCommitProofError> {
        self.validate_well_formed()?;
        let header = &self.commit_proof.header;
        if header.height == 0 {
            return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                "Header height is 0"
            )));
        }

        let signature = header
            .signature
            .to_schnorr_signature()
            .map_err(|e| ForeignProposalCommitProofError::Invalid(anyhow!("Malformed signature: {e}")))?;
        let block_id = header.calculate_block_id();

        let proposed_by = header
            .proposed_by
            .to_public_key()
            .map_err(|e| ForeignProposalCommitProofError::Invalid(anyhow!("Malformed proposer public key: {e}")))?;
        if !signature.verify(&proposed_by, block_id) {
            return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                "Signature verification failed for block id: {}",
                block_id
            )));
        }
        Ok(())
    }

    fn validate_command_merkle_root(&self) -> Result<(), ForeignProposalCommitProofError> {
        // We essentially rebuild the tree here to check the Merkle root. Since we expect fewer than 1000
        // commands, this is probably acceptable. We don't have a Merkle proof that supports multiple non-consecutive
        // leaves. Such a proof would carry on overhead when compared to a Merkle proof for a single leaf. This
        // is because the sparse single-leaf proofs rely on depth-ordered siblings (i.e. siblings vec + the leaf
        // key implicitly "encode" the hash ordering for proof verification) and therefore we'd need some other way to
        // represent this ordering within a multi-proof with nonduplicate nodes. Since we only include the full command
        // data for applicable commands, such a multi-proof may not be worthwhile.
        let protocol_version = self.protocol_version()?;
        let command_hashes = self
            .commands
            .iter()
            .map(|cmd| TreeHash::new(cmd.merkle_leaf(protocol_version).into_array()));
        let root_hash = compute_merkle_root_for_hashes(command_hashes)?;
        if FixedHash::from(root_hash.into_array()) != self.commit_proof.header.command_merkle_root {
            return Err(ForeignProposalCommitProofError::InvalidCommandMerkleRoot {
                calculated: root_hash,
                expected: self.commit_proof.header.command_merkle_root,
            });
        }
        Ok(())
    }

    fn validate_well_formed(&self) -> Result<(), ForeignProposalCommitProofError> {
        const MAX_COMMANDS: usize = 1000;

        if self.commands.is_empty() {
            return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                "CommandsCommitProofV1 must have at least one command"
            )));
        }

        if self.commands.len() > MAX_COMMANDS {
            return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                "CommandsCommitProofV1 must have at most 1000 commands but has {}",
                self.commands.len()
            )));
        }

        // Since CompressedPublicKey is a variable length vec, we make double sure that the length is correct.
        if self
            .commit_proof
            .header
            .signature
            .get_compressed_public_nonce()
            .as_bytes()
            .len() !=
            RistrettoPublicKeyBytes::length()
        {
            return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                "Signature public nonce is not a valid RistrettoPublicKey"
            )));
        }
        if self.commit_proof.header.proposed_by.as_bytes().len() != RistrettoPublicKeyBytes::length() {
            return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                "Proposed by public key is not a valid RistrettoPublicKey"
            )));
        }

        for (i, elem) in self.commit_proof.proof_elements.iter().enumerate() {
            match elem {
                CommitProofElement::QuorumCertificate(qc) => {
                    if qc.signatures.num_signatures() == 0 {
                        return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                            "QuorumCertificate at index {i} has no signatures"
                        )));
                    }
                },
                CommitProofElement::ChainLinks(links) => {
                    if links.is_empty() {
                        return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                            "ChainLinks at index {i} is empty"
                        )));
                    }
                },
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Encode, Decode, CborLen)]
pub enum CommandOrHash {
    /// The full command data
    #[n(0)]
    Command(#[n(0)] Command),
    /// A command pruned from the proof, carried by its hash and the shard groups the proof must carry it in full for.
    #[n(1)]
    Pruned(#[n(0)] PrunedCommand),
}

impl CommandOrHash {
    /// The form a foreign commit proof for `shard_group` carries `command` in, for a block proposed under
    /// `protocol_version`.
    pub fn for_shard_group(protocol_version: ProtocolVersion, command: &Command, shard_group: ShardGroup) -> Self {
        let full_command_shard_groups = command.full_command_shard_groups();
        if full_command_shard_groups.contains(&shard_group) {
            return Self::Command(command.clone());
        }
        match protocol_version {
            ProtocolVersion::V0 => match command {
                Command::EndEpoch(_) => Self::Command(command.clone()),
                Command::LocalOnly(_) |
                Command::LocalPrepare(_) |
                Command::LocalAccept(_) |
                Command::AllAccept(_) |
                Command::SomeAccept(_) |
                Command::ForeignProposal(_) => Self::Pruned(PrunedCommand {
                    hash: command.hash(protocol_version),
                    full_command_shard_groups,
                }),
            },
        }
    }

    /// The entry's leaf in the command merkle root of a block proposed under `protocol_version`.
    pub fn merkle_leaf(&self, protocol_version: ProtocolVersion) -> FixedHash {
        match (self, protocol_version) {
            (Self::Command(cmd), _) => cmd.merkle_leaf(protocol_version),
            (Self::Pruned(pruned), ProtocolVersion::V0) => {
                command_leaf(&pruned.hash, &pruned.full_command_shard_groups)
            },
        }
    }

    pub fn command(&self) -> Option<&Command> {
        match self {
            Self::Command(cmd) => Some(cmd),
            Self::Pruned(_) => None,
        }
    }

    pub fn into_command(self) -> Option<Command> {
        match self {
            Self::Command(cmd) => Some(cmd),
            Self::Pruned(_) => None,
        }
    }
}

/// A command a foreign commit proof carries only by its hash, with the shard groups the proof must carry it in full
/// for. The merkle leaf commits to both, so a proof that names a shard group here withholds a command that shard group
/// needs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Encode, Decode, CborLen)]
pub struct PrunedCommand {
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    pub hash: FixedHash,
    #[n(1)]
    pub full_command_shard_groups: Vec<ShardGroup>,
}

#[cfg(test)]
mod tests {
    use tari_consensus_types::Decision;
    use tari_crypto::ristretto::RistrettoSecretKey;
    use tari_engine_types::{SubstateVersion, substate::SubstateId};
    use tari_ootle_common_types::SubstateLockType;
    use tari_ootle_transaction::TransactionId;
    use tari_sidechain::{CompressedPublicKey, SidechainBlockHeader, ValidatorBlockSignature};
    use tari_template_lib_types::{ComponentAddress, ObjectKey};

    use super::*;
    use crate::consensus_models::{EndEpochAtom, Evidence, MultiShardAtom};

    fn input_group() -> ShardGroup {
        ShardGroup::new(0u32, 31u32)
    }

    fn output_group() -> ShardGroup {
        ShardGroup::new(32u32, 63u32)
    }

    fn uninvolved_group() -> ShardGroup {
        ShardGroup::new(64u32, 95u32)
    }

    /// A proof that carries `entries` for a block proposed under `protocol_version` that committed `commands`.
    fn proof(
        protocol_version: ProtocolVersion,
        commands: &[Command],
        entries: Vec<CommandOrHash>,
    ) -> CommandsCommitProofV1 {
        let leaves = commands
            .iter()
            .map(|cmd| TreeHash::new(cmd.merkle_leaf(protocol_version).into_array()));
        let command_merkle_root = FixedHash::from(compute_merkle_root_for_hashes(leaves).unwrap().into_array());
        CommandsCommitProofV1 {
            commands: entries,
            commit_proof: SidechainBlockCommitProof {
                header: SidechainBlockHeader {
                    network: 0,
                    protocol_version: protocol_version.as_u32(),
                    parent_id: FixedHash::zero(),
                    justify_id: FixedHash::zero(),
                    height: 1,
                    epoch: 1,
                    epoch_hash: FixedHash::zero(),
                    shard_group: tari_sidechain::ShardGroup {
                        start: 0,
                        end_inclusive: 31,
                    },
                    proposed_by: CompressedPublicKey::default(),
                    state_merkle_root: FixedHash::zero(),
                    command_merkle_root,
                    transaction_merkle_root: FixedHash::zero(),
                    signature: ValidatorBlockSignature::new(
                        CompressedPublicKey::default(),
                        RistrettoSecretKey::default(),
                    ),
                    accumulated_data: Default::default(),
                    metadata_hash: FixedHash::zero(),
                },
                proof_elements: vec![],
            },
        }
    }

    fn atom() -> MultiShardAtom {
        let mut evidence = Evidence::empty();
        evidence.add_shard_group(input_group()).insert(
            SubstateId::Component(ComponentAddress::from_array([1; ObjectKey::LENGTH])),
            SubstateVersion::ZERO,
            SubstateLockType::Write,
            None,
        );
        evidence.add_shard_group(output_group()).insert_output(
            SubstateId::Component(ComponentAddress::from_array([2; ObjectKey::LENGTH])),
            SubstateVersion::ZERO,
        );
        MultiShardAtom {
            id: TransactionId::new([3; 32]),
            decision: Decision::Commit,
            evidence,
            transaction_fee: 0,
            leader_fee: None,
        }
    }

    fn commands() -> Vec<Command> {
        vec![
            Command::LocalPrepare(atom()),
            Command::LocalAccept(atom()),
            Command::AllAccept(atom()),
            Command::EndEpoch(EndEpochAtom::new(FixedHash::zero())),
        ]
    }

    /// The proof a sender generates for `shard_group` of a block proposed under `protocol_version`.
    fn honest_proof(protocol_version: ProtocolVersion, shard_group: ShardGroup) -> CommandsCommitProofV1 {
        let commands = commands();
        let entries = commands
            .iter()
            .map(|cmd| CommandOrHash::for_shard_group(protocol_version, cmd, shard_group))
            .collect();
        proof(protocol_version, &commands, entries)
    }

    /// `proof` with the entry at `index` replaced by `entry`.
    fn with_entry(mut proof: CommandsCommitProofV1, index: usize, entry: CommandOrHash) -> CommandsCommitProofV1 {
        proof.commands[index] = entry;
        proof
    }

    fn withheld_command_for(proof: &CommandsCommitProofV1, shard_group: ShardGroup) -> Option<PrunedCommand> {
        CommandsCommitProof::V1(proof.clone())
            .withheld_command_for(shard_group)
            .cloned()
    }

    #[test]
    fn a_proof_carries_in_full_every_command_the_shard_group_needs() {
        let local_prepare = 0;
        let local_accept = 1;
        let cases = [
            (input_group(), &[local_prepare, local_accept][..]),
            (output_group(), &[local_accept][..]),
            (uninvolved_group(), &[][..]),
        ];
        for (shard_group, needed) in cases {
            let proof = honest_proof(ProtocolVersion::V0, shard_group);
            proof.validate_command_merkle_root().unwrap();
            assert_eq!(withheld_command_for(&proof, shard_group), None, "{shard_group}");
            for index in needed {
                assert!(proof.commands[*index].command().is_some(), "{shard_group}: {index}");
            }
        }
    }

    #[test]
    fn a_proof_that_prunes_a_needed_command_names_the_shard_group_it_withholds_it_from() {
        let local_accept = &commands()[1];
        let pruned = CommandOrHash::Pruned(PrunedCommand {
            hash: local_accept.hash(ProtocolVersion::V0),
            full_command_shard_groups: local_accept.full_command_shard_groups(),
        });
        let proof = with_entry(honest_proof(ProtocolVersion::V0, output_group()), 1, pruned);

        proof.validate_command_merkle_root().unwrap();
        assert_eq!(
            withheld_command_for(&proof, output_group()).map(|t| t.hash),
            Some(local_accept.hash(ProtocolVersion::V0))
        );
    }

    #[test]
    fn a_pruned_command_that_leaves_out_a_shard_group_does_not_authenticate() {
        let local_accept = &commands()[1];
        for full_command_shard_groups in [vec![input_group()], vec![], vec![output_group(), input_group()]] {
            let pruned = CommandOrHash::Pruned(PrunedCommand {
                hash: local_accept.hash(ProtocolVersion::V0),
                full_command_shard_groups: full_command_shard_groups.clone(),
            });
            let proof = with_entry(honest_proof(ProtocolVersion::V0, output_group()), 1, pruned);
            assert!(
                matches!(
                    proof.validate_command_merkle_root(),
                    Err(ForeignProposalCommitProofError::InvalidCommandMerkleRoot { .. })
                ),
                "{full_command_shard_groups:?}"
            );
        }
    }

    #[test]
    fn an_end_epoch_command_keeps_its_bare_hash_leaf_and_is_carried_in_full() {
        let end_epoch = &commands()[3];
        assert_eq!(
            end_epoch.merkle_leaf(ProtocolVersion::V0),
            end_epoch.hash(ProtocolVersion::V0)
        );
        let proof = honest_proof(ProtocolVersion::V0, uninvolved_group());
        assert_eq!(proof.commands[3], CommandOrHash::Command(end_epoch.clone()));
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ForeignProposalCommitProofError {
    #[error("State tree error: {0}")]
    StateTreeError(#[from] StateTreeError),
    #[error("Invalid command Merkle root. Calculated: {calculated}, expected: {expected}")]
    InvalidCommandMerkleRoot { calculated: TreeHash, expected: FixedHash },
    #[error("Foreign proposal block: {0}")]
    UnknownProtocolVersion(#[from] UnknownProtocolVersionError),
    #[error("Sidechain Proof Validation Error: {0}")]
    SidechainProofValidationError(#[from] SidechainProofValidationError),
    #[error("The foreign proposal was invalid: {0}")]
    Invalid(#[from] anyhow::Error),
}
