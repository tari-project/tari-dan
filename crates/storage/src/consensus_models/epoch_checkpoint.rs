//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use anyhow::anyhow;
use borsh::{BorshDeserialize, BorshSerialize};
use indexmap::IndexMap;
use minicbor::{CborLen, Decode, Encode};
use ootle_network::Network;
use serde::{Deserialize, Serialize};
use tari_engine_types::ProtocolVersion;
use tari_ootle_common_types::{Epoch, ShardGroup, ValidatorSetHash, VotePower, shard::Shard};
use tari_sidechain::{
    CommandCommitProof,
    FixedHash,
    SidechainBlockHeader,
    SidechainProofValidationError,
    SigningCommittee,
    ToCommand,
};
use tari_state_tree::{SPARSE_MERKLE_PLACEHOLDER_HASH, StateTreeError, TreeHash, Version, compute_shard_group_root};

use crate::{
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{BlockHeaderMetadata, CommittedBlockProof},
};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct EpochCheckpoint {
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    proof: CommandCommitProof<EndOfEpochCommand>,
    #[n(1)]
    #[cbor(with = "tari_bor::adapters::indexmap_codec")]
    shard_tree_summary: IndexMap<Shard, TreeRootSummary>,
    /// The preimage of the end-of-epoch header's metadata hash, from which a holder of this checkpoint reads the
    /// header's [`ExtraData`](tari_ootle_common_types::ExtraData). The quorum certificate does not cover it, so
    /// [`Self::validate_well_formed`] checks it against the header's metadata hash.
    #[n(2)]
    header_metadata: BlockHeaderMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct TreeRootSummary {
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    pub root_hash: TreeHash,
    #[n(1)]
    pub state_version: Version,
}

impl EpochCheckpoint {
    pub fn new(
        proof: CommandCommitProof<EndOfEpochCommand>,
        shard_tree_summary: IndexMap<Shard, TreeRootSummary>,
        header_metadata: BlockHeaderMetadata,
    ) -> Self {
        Self {
            proof,
            shard_tree_summary,
            header_metadata,
        }
    }

    /// The preimage of the end-of-epoch header's metadata hash. [`Self::validate_well_formed`] checks it against the
    /// header, so it is the committed preimage once the checkpoint validates.
    pub fn header_metadata(&self) -> &BlockHeaderMetadata {
        &self.header_metadata
    }

    /// The [`ValidatorSetHash`] the end-of-epoch block ratifies for the next epoch, read from
    /// [`Self::header_metadata`]. It is the committed value only once the checkpoint validates.
    pub fn next_epoch_validator_set_hash(&self) -> Option<ValidatorSetHash> {
        self.header_metadata.next_epoch_validator_set_hash()
    }

    pub fn proof(&self) -> &CommandCommitProof<EndOfEpochCommand> {
        &self.proof
    }

    /// The commit proof of the end-of-epoch block, which outlives the block once epoch GC prunes it.
    pub fn block_commit_proof(&self) -> CommittedBlockProof {
        let CommandCommitProof::V1(proof) = &self.proof;
        CommittedBlockProof::new(proof.commit_proof().clone())
    }

    pub fn header(&self) -> &SidechainBlockHeader {
        self.proof.header()
    }

    pub fn checked_shard_group(&self) -> Result<ShardGroup, EpochCheckpointValidationError> {
        convert_sidechain_shard_group_to_shard_group(self.header().shard_group)
    }

    pub fn epoch(&self) -> Epoch {
        Epoch(self.proof.header().epoch)
    }

    pub fn shard_tree_summary(&self) -> &IndexMap<Shard, TreeRootSummary> {
        &self.shard_tree_summary
    }

    pub fn get_shard_root(&self, shard: Shard) -> TreeHash {
        self.shard_tree_summary
            .get(&shard)
            .map(|summary| summary.root_hash)
            .unwrap_or(SPARSE_MERKLE_PLACEHOLDER_HASH)
    }

    pub fn get_shard_state_version(&self, shard: Shard) -> Version {
        self.shard_tree_summary
            .get(&shard)
            .map(|summary| summary.state_version)
            .unwrap_or_default()
    }

    /// The protocol version of the end-of-epoch block, which selects how its state merkle root's leaves are formed.
    pub fn protocol_version(&self) -> Result<ProtocolVersion, EpochCheckpointValidationError> {
        ProtocolVersion::try_from(self.header().protocol_version)
            .map_err(|e| EpochCheckpointValidationError::InvalidEpochCheckpoint(e.into()))
    }

    /// The root commits to each shard's state version as well as its root, so a checkpoint that validates fixes both.
    pub fn compute_state_merkle_root(&self) -> Result<TreeHash, EpochCheckpointValidationError> {
        self.compute_state_merkle_root_as(self.protocol_version()?)
    }

    /// The state merkle root of this checkpoint's shard states with leaves formed under `protocol_version`, which is
    /// the root the next epoch's genesis block commits when that epoch runs a different version.
    pub fn compute_state_merkle_root_as(
        &self,
        protocol_version: ProtocolVersion,
    ) -> Result<TreeHash, EpochCheckpointValidationError> {
        let shard_group = self.checked_shard_group()?;
        let shard_states = shard_group
            .shard_iter_with_global()
            .map(|shard| (shard, self.get_shard_root(shard), self.get_shard_state_version(shard)));
        let root = compute_shard_group_root(protocol_version, shard_states)?;
        Ok(root)
    }

    /// Validates that this epoch checkpoint is valid.
    /// 1. The commit proof is valid
    ///  - The block is on `network`
    ///  - The command is included in the original block
    ///  - The QCs are signed by a correct quorum of validator nodes
    ///  - The 3-chain rule is satisfied
    /// 2. The state merkle root in the block header matches the provided shard hashes.
    ///  - Not to be confused with validating that some stored sharded state tree matches the epoch checkpoint - simply
    ///    a sanity check.
    pub fn validate(
        &self,
        network: Network,
        epoch: Epoch,
        quorum_threshold: VotePower,
        committee: &dyn SigningCommittee,
    ) -> Result<(), EpochCheckpointValidationError> {
        if self.epoch() != epoch {
            return Err(EpochCheckpointValidationError::InvalidEpochCheckpoint(anyhow!(
                "Expected checkpoint epoch {} but proof epoch is {}",
                self.epoch(),
                self.proof.header().epoch
            )));
        }

        self.validate_well_formed()?;

        // Validate the proof
        self.proof
            .validate_committed(network.as_byte(), quorum_threshold.value(), committee)?;

        Ok(())
    }

    /// Validates that the epoch checkpoint is well-formed.
    /// This includes basic sanity checks such as:
    /// - The shard group is valid (start < end + 1)
    /// - The number of shard state roots does not exceed the number of shards in the group
    /// - The state root matches the provided header
    ///
    /// Use EpochCheckpoint::validate() to validate the entire proof (including well-formedness).
    pub fn validate_well_formed(&self) -> Result<(), EpochCheckpointValidationError> {
        // Basic sanity checks
        let header_shard_group = convert_sidechain_shard_group_to_shard_group(self.header().shard_group)?;
        let num_shards = header_shard_group.len();
        if num_shards == 0 {
            return Err(EpochCheckpointValidationError::InvalidEpochCheckpoint(anyhow!(
                "Invalid shard group: end == start {}",
                header_shard_group
            )));
        }

        // 1 + for global shard
        if self.shard_tree_summary.len() > num_shards + 1 {
            return Err(
                EpochCheckpointValidationError::NumberOfShardStateRootsExceedsNumberOfShards {
                    num_shard_state_roots: self.shard_tree_summary.len(),
                    num_shards,
                },
            );
        }

        // TODO: more basic checks?

        let computed = self.header_metadata.calculate_hash();
        if computed != self.header().metadata_hash {
            return Err(EpochCheckpointValidationError::HeaderMetadataMismatch {
                computed,
                header_metadata_hash: self.header().metadata_hash,
            });
        }

        // Validate state root matches provided header
        let state_root = self.compute_state_merkle_root()?;
        if state_root != self.proof.header().state_merkle_root {
            return Err(EpochCheckpointValidationError::ShardStateRootMerkleTreeRootMismatch {
                computed: state_root,
                header_state_root: self.proof.header().state_merkle_root,
            });
        }
        Ok(())
    }
}

impl EpochCheckpoint {
    pub fn get_all_from_epoch<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        from_epoch: Epoch,
        limit: usize,
    ) -> Result<Vec<Self>, StorageError> {
        tx.epoch_checkpoint_get_all_from_epoch(from_epoch, limit)
    }

    pub fn get_all_for_epoch<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
    ) -> Result<Vec<Self>, StorageError> {
        tx.epoch_checkpoint_get_all_for_epoch(epoch)
    }

    pub fn get_last_checkpoint<TTx: StateStoreReadTransaction>(tx: &TTx) -> Result<Self, StorageError> {
        tx.epoch_checkpoint_get_last()
    }

    pub fn get_by_shard_group<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Self, StorageError> {
        tx.epoch_checkpoint_get_by_shard_group(epoch, shard_group)
    }

    pub fn save<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.epoch_checkpoint_save(self)
    }
}

impl Display for EpochCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "EpochCheckpoint: block_id={}, epoch={}, count(shard_roots)={}",
            self.proof.header().calculate_block_id(),
            self.proof.header().epoch,
            self.shard_tree_summary.len()
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EpochCheckpointValidationError {
    #[error("Shard state root merkle tree root mismatch: computed {computed} != header state root {header_state_root}")]
    ShardStateRootMerkleTreeRootMismatch {
        computed: TreeHash,
        header_state_root: FixedHash,
    },
    #[error("Invalid state tree: {0}")]
    StateTreeError(#[from] StateTreeError),
    #[error("Sidechain proof validation error: {0}")]
    SidechainProofValidationError(#[from] SidechainProofValidationError),
    #[error("Number of shard state roots ({num_shard_state_roots}) exceeds number of shards ({num_shards})")]
    NumberOfShardStateRootsExceedsNumberOfShards {
        num_shard_state_roots: usize,
        num_shards: usize,
    },
    #[error(
        "Checkpoint header metadata hashes to {computed} but the end-of-epoch header commits to {header_metadata_hash}"
    )]
    HeaderMetadataMismatch {
        computed: FixedHash,
        header_metadata_hash: FixedHash,
    },
    #[error("Invalid epoch checkpoint: {0}")]
    InvalidEpochCheckpoint(#[from] anyhow::Error),
}

fn convert_sidechain_shard_group_to_shard_group(
    shard_group: tari_sidechain::ShardGroup,
) -> Result<ShardGroup, EpochCheckpointValidationError> {
    ShardGroup::new_checked(shard_group.start, shard_group.end_inclusive).ok_or_else(|| {
        EpochCheckpointValidationError::InvalidEpochCheckpoint(anyhow!(
            "Invalid shard group: start >= end + 1 ({}-{})",
            shard_group.start,
            shard_group.end_inclusive
        ))
    })
}

#[derive(
    Debug, Clone, PartialEq, Eq, Deserialize, Serialize, BorshSerialize, BorshDeserialize, Encode, Decode, CborLen,
)]
pub struct EndOfEpochCommand {
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    next_epoch_hash: FixedHash,
}

impl EndOfEpochCommand {
    pub fn new(next_epoch_hash: FixedHash) -> Self {
        Self { next_epoch_hash }
    }

    pub fn next_epoch_hash(&self) -> &FixedHash {
        &self.next_epoch_hash
    }
}

impl ToCommand for EndOfEpochCommand {
    fn to_command(&self) -> tari_sidechain::Command {
        // Must match the dan-side `Command::EndEpoch(EndEpochAtom)` byte-for-byte so the command hash
        // (and thus the inclusion proof against the committed block's command merkle root) verifies.
        tari_sidechain::Command::EndEpoch(tari_sidechain::EndEpochAtom::new(self.next_epoch_hash))
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::{ExtraData, ExtraFieldKey};
    use tari_sidechain::SidechainBlockCommitProof;
    use tari_state_tree::compute_proof_for_hashes;

    use super::*;

    fn metadata(set_hash: ValidatorSetHash) -> BlockHeaderMetadata {
        let mut extra_data = ExtraData::new();
        extra_data.insert_validator_set_hash(ExtraFieldKey::NextEpochValidatorSetHash, &set_hash);
        BlockHeaderMetadata {
            total_leader_fee: 5,
            timestamp: 1234,
            extra_data,
            timeout_certificate_id: None,
            is_dummy: false,
        }
    }

    /// A checkpoint whose header commits to `committed` and whose state root matches its shard summary.
    fn checkpoint(committed: &BlockHeaderMetadata, carried: BlockHeaderMetadata) -> EpochCheckpoint {
        let key = TreeHash::new([1; 32]);
        let (_, inclusion_proof) = compute_proof_for_hashes([key].into_iter(), key).unwrap();
        let mut header = SidechainBlockHeader {
            network: 0,
            protocol_version: 0,
            parent_id: Default::default(),
            justify_id: Default::default(),
            height: 0,
            epoch: 3,
            epoch_hash: Default::default(),
            shard_group: tari_sidechain::ShardGroup {
                start: 1,
                end_inclusive: 2,
            },
            proposed_by: Default::default(),
            state_merkle_root: Default::default(),
            command_merkle_root: Default::default(),
            transaction_merkle_root: Default::default(),
            signature: Default::default(),
            accumulated_data: Default::default(),
            metadata_hash: committed.calculate_hash(),
        };
        let build = |header: SidechainBlockHeader| {
            EpochCheckpoint::new(
                CommandCommitProof::new(
                    EndOfEpochCommand::new(FixedHash::default()),
                    SidechainBlockCommitProof {
                        header,
                        proof_elements: vec![],
                    },
                    inclusion_proof.clone(),
                ),
                IndexMap::new(),
                carried.clone(),
            )
        };
        let state_root = build(header.clone()).compute_state_merkle_root().unwrap();
        header.state_merkle_root = FixedHash::new(state_root.into_array());
        build(header)
    }

    #[test]
    fn a_checkpoint_carrying_the_committed_preimage_names_the_next_validator_set() {
        let set_hash = ValidatorSetHash::from_array([9; 32]);
        let committed = metadata(set_hash);
        let checkpoint = checkpoint(&committed, committed.clone());
        checkpoint.validate_well_formed().unwrap();
        assert_eq!(checkpoint.next_epoch_validator_set_hash(), Some(set_hash));
    }

    #[test]
    fn a_checkpoint_carrying_another_preimage_is_rejected() {
        let committed = metadata(ValidatorSetHash::from_array([9; 32]));
        let forged = metadata(ValidatorSetHash::from_array([8; 32]));
        let err = checkpoint(&committed, forged).validate_well_formed().unwrap_err();
        assert!(
            matches!(err, EpochCheckpointValidationError::HeaderMetadataMismatch { .. }),
            "{err}"
        );
    }

    #[test]
    fn a_checkpoint_round_trips_its_preimage() {
        let set_hash = ValidatorSetHash::from_array([9; 32]);
        let committed = metadata(set_hash);
        let checkpoint = checkpoint(&committed, committed.clone());

        let decoded: EpochCheckpoint = tari_bor::decode(&tari_bor::encode(&checkpoint).unwrap()).unwrap();
        decoded.validate_well_formed().unwrap();
        assert_eq!(decoded.next_epoch_validator_set_hash(), Some(set_hash));

        let decoded: EpochCheckpoint = serde_json::from_str(&serde_json::to_string(&checkpoint).unwrap()).unwrap();
        decoded.validate_well_formed().unwrap();
        assert_eq!(decoded.next_epoch_validator_set_hash(), Some(set_hash));
    }
}
