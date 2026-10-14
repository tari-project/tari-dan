//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_common_types::types::{CompressedPublicKey, FixedHash};
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::{ProtocolVersion, UnknownProtocolVersionError};
use tari_ootle_common_types::VotePower;
use tari_sidechain::{CommitProofElement, SidechainBlockCommitProof, SidechainProofValidationError};
use tari_state_tree::{StateTreeError, TreeHash, compute_merkle_root_for_hashes};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::consensus_models::Command;

pub type CheckVnFunc<'a> = dyn Fn(&RistrettoPublicKeyBytes) -> Result<VotePower, SidechainProofValidationError> + 'a;

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

    /// Test-only: mutable access to the command list. Replacing a `Command` entry with
    /// `CommandOrHash::Hash(same_hash)` does not change the merkle root (see `CommandOrHash::hash`), so this models a
    /// Byzantine transport trimming a command out of a genuine, already-signed commit proof.
    pub fn commands_mut(&mut self) -> &mut Vec<CommandOrHash> {
        match self {
            Self::V1(proof) => &mut proof.commands,
        }
    }

    pub fn applicable_commands_iter(&self) -> impl Iterator<Item = &Command> + '_ {
        self.commands().iter().filter_map(|cmd| cmd.command())
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

    pub fn validate_committed(
        &self,
        quorum_threshold: VotePower,
        check_vn: &CheckVnFunc<'_>,
    ) -> Result<(), ForeignProposalCommitProofError> {
        match self {
            Self::V1(proof) => proof.validate_committed(quorum_threshold, check_vn),
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
        quorum_threshold: VotePower,
        check_vn: &CheckVnFunc<'_>,
    ) -> Result<(), ForeignProposalCommitProofError> {
        self.validate_command_merkle_root()?;
        self.commit_proof
            // TODO: Currently 1 Vn = 1 vote power. But this may change in the future. Update the library.
            .validate_committed(quorum_threshold.value() as usize, &|pk: &CompressedPublicKey| {
                // TODO: commit proof should validate vote power not just pk existence
                check_vn(&RistrettoPublicKeyBytes::from_bytes(pk.as_bytes()).expect("already checked"))
                    .map(|v| !v.is_zero())
            })?;
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
            .map(|cmd| TreeHash::new(cmd.hash(protocol_version).into_array()));
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
                    if qc.signatures.is_empty() {
                        return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                            "QuorumCertificate at index {i} has no signatures"
                        )));
                    }
                    if qc
                        .signatures
                        .iter()
                        .any(|s| s.public_key.as_bytes().len() != RistrettoPublicKeyBytes::length())
                    {
                        return Err(ForeignProposalCommitProofError::Invalid(anyhow::anyhow!(
                            "QuorumCertificate at index {i} has invalid public key"
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
// SECURITY (GHSA-hgp9-3vqw-2fp2): the sender chooses, per recipient shard group, which commands are sent as
// `Command` (full) vs `Hash` (trimmed) based on whether the command involves that recipient
// (`generate_transaction_commands_commit_proof_for_shard_group`). `CommandOrHash::hash` is defined so a `Command`
// and a `Hash` of the same content produce the SAME leaf hash, which is what lets this trimming leave the proof's
// merkle root (and therefore every signature and QC over it) unchanged -- this is also what makes it exploitable: a
// relayer in the message path can apply the exact same trim to a command that DOES involve the recipient, and the
// recipient has no way to tell a legitimate trim from a malicious one using only this proof.
//
// Confirmed end-to-end in `consensus_tests::command_trimming_censorship`: with one relevant `Command` entry trimmed
// to `Hash` in flight, the affected transaction never resolves (no commit, no abort) -- a permanent liveness bug
// for that one transaction, not a bounded delay. See the security advisory for full detail.
//
// A verifiably sound fix needs one of:
//   1. Bind a per-recipient-shard-group relevance commitment into the SAME authenticated data the recipient already
//      checks. This is harder than it sounds: the merkle root this proof is checked against
//      (`commit_proof.header.command_merkle_root`) is a field of the ORIGINAL block header, computed once by the
//      generating committee before any recipient-specific proof exists, and reused unchanged for every recipient.
//      Making relevance provable without the full command therefore means changing what the block header commits to,
//      which is a wire/protocol-level change (affects block ids and QC signing broadly) and needs design input from the
//      team, not a solo patch to this file.
//   2. A non-cryptographic liveness backstop: if a transaction stays unresolved past a generous multiple of normal
//      commit latency after a foreign proposal for its block has been accepted, re-request that exact block_id from a
//      different member of the foreign committee rather than treating the first accepted copy as final. This doesn't
//      make trimming detectable, but bounds the damage from "permanent" to "delayed, with retries against other peers"
//      -- PROVIDED the local liveness/retry wiring correctly tracks "which transactions are still waiting on evidence
//      from which foreign shard group", which needs care to get right in this event-driven HotStuff implementation (see
//      `OnReceiveForeignProposalHandler`).
pub enum CommandOrHash {
    /// The full command data
    #[n(0)]
    Command(#[n(0)] Command),
    /// A hash of a command
    #[n(1)]
    Hash(
        #[n(0)]
        #[cbor(with = "tari_bor::adapters::serde_bridge")]
        FixedHash,
    ),
}

impl CommandOrHash {
    pub fn hash(&self, protocol_version: ProtocolVersion) -> FixedHash {
        match self {
            Self::Command(cmd) => cmd.hash(protocol_version),
            Self::Hash(hash) => *hash,
        }
    }

    pub fn command(&self) -> Option<&Command> {
        match self {
            Self::Command(cmd) => Some(cmd),
            Self::Hash(_) => None,
        }
    }

    pub fn into_command(self) -> Option<Command> {
        match self {
            Self::Command(cmd) => Some(cmd),
            Self::Hash(_) => None,
        }
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
