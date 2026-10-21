//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Signed fixtures for tests: validator sets with real committee proofs, end-of-epoch checkpoints and block commit
//! proofs that a committee's keys sign over a 3-chain of quorum certificates, and substate proofs anchored at those
//! blocks.

use indexmap::IndexMap;
use ootle_network::Network;
use tari_common_types::types::{CompressedPublicKey, PrivateKey};
use tari_crypto::{keys::SecretKey, tari_utilities::ByteArray};
use tari_engine_types::{
    ProtocolVersion,
    SubstateVersion,
    substate::{SubstateId, SubstateValue, hash_substate},
};
use tari_indexer_client::types::{
    EpochCheckpointEntry,
    SubstateProof,
    SubstateProofAnchor,
    ValidatorSetCommittee,
    ValidatorSetCommitteeMember,
};
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    ExtraFieldKey,
    NumPreshards,
    ShardGroup,
    ValidatorSetHash,
    VersionedSubstateId,
    VotePower,
    committee::{Committee, CommitteeMember},
    shard::Shard,
};
use tari_ootle_storage::consensus_models::{
    BlockHeaderMetadata,
    CommittedBlockProof,
    EndOfEpochCommand,
    EpochCheckpoint,
    TreeRootSummary,
};
use tari_sidechain::{
    ChainLink,
    CommandCommitProof,
    CommitProofElement,
    DecodedValidatorBlockSignature,
    FixedHash,
    ProposalVoteMessage,
    QcSignatures,
    QuorumCertificate,
    QuorumDecision,
    SidechainBlockCommitProof,
    SidechainBlockHeader,
    SignerBitmap,
    ToCommand,
    ValidatorBlockSignature,
};
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    ShardGroupRootTree,
    SpreadPrefixStateTree,
    StateTreePayload,
    SubstateDownProof,
    SubstateTreeChange,
    SubstateValueProof,
    TreeHash,
    ValidatorSetTree,
    compute_merkle_root_for_hashes,
    compute_proof_for_hashes,
    memory_store::MemoryTreeStore,
};
use tari_template_lib_types::{Hash32, crypto::RistrettoPublicKeyBytes};

pub const NETWORK: Network = Network::LocalNet;
pub const NUM_PRESHARDS: NumPreshards = NumPreshards::P4;

/// The shard group the light client walks, holding [`walk_shard`].
pub fn low() -> ShardGroup {
    ShardGroup::new(1u32, 2u32)
}

pub fn high() -> ShardGroup {
    ShardGroup::new(3u32, 4u32)
}

pub fn walk_shard() -> Shard {
    Shard::first()
}

/// The shard group of [`low`] and [`high`] that holds `shard`.
pub fn shard_group_of(shard: Shard) -> ShardGroup {
    if low().contains(&shard) { low() } else { high() }
}

/// A committee's signing keys.
#[derive(Clone)]
pub struct Signers {
    pub keys: Vec<PrivateKey>,
}

impl Signers {
    /// `n` keys, distinct for every distinct `seed`.
    pub fn new(seed: u8, n: u8) -> Self {
        let keys = (1..=n)
            .map(|i| {
                let mut bytes = [seed; 64];
                bytes[0] = i;
                PrivateKey::from_uniform_bytes(&bytes).unwrap()
            })
            .collect();
        Self { keys }
    }

    pub fn committee(&self) -> Committee<RistrettoPublicKeyBytes> {
        let mut members = self
            .keys
            .iter()
            .map(|key| {
                let public_key = public_key_bytes(key);
                CommitteeMember {
                    address: public_key,
                    public_key,
                    vote_power: VotePower::of(1),
                }
            })
            .collect::<Vec<_>>();
        members.sort_by_key(|member| member.public_key);
        Committee::new(members)
    }
}

fn public_key_bytes(key: &PrivateKey) -> RistrettoPublicKeyBytes {
    RistrettoPublicKeyBytes::from_bytes(CompressedPublicKey::from_secret_key(key).as_bytes()).unwrap()
}

/// An epoch's validator set of two shard groups, [`low`] and [`high`].
pub struct ValidatorSetFixture {
    pub network: Network,
    pub epoch: Epoch,
    pub low: Signers,
    pub high: Signers,
    tree: ValidatorSetTree,
}

impl ValidatorSetFixture {
    pub fn new(epoch: u64) -> Self {
        Self::with_signers(
            NETWORK,
            epoch,
            Signers::new((epoch as u8).wrapping_mul(2), 4),
            Signers::new((epoch as u8).wrapping_mul(2).wrapping_add(1), 4),
        )
    }

    pub fn with_signers(network: Network, epoch: u64, low_signers: Signers, high_signers: Signers) -> Self {
        let epoch = Epoch(epoch);
        let version = ProtocolVersion::at(network, epoch);
        let leaves = [(low(), &low_signers), (high(), &high_signers)].map(|(shard_group, signers)| {
            (
                shard_group,
                signers
                    .committee()
                    .calculate_validator_set_leaf_hash(version, &shard_group)
                    .unwrap(),
            )
        });
        let tree = ValidatorSetTree::build(network, epoch, NUM_PRESHARDS, leaves).unwrap();
        Self {
            network,
            epoch,
            low: low_signers,
            high: high_signers,
            tree,
        }
    }

    pub fn hash(&self) -> ValidatorSetHash {
        self.tree.hash()
    }

    pub fn signers(&self, shard_group: ShardGroup) -> &Signers {
        if shard_group == low() { &self.low } else { &self.high }
    }

    /// The committee of `shard_group` as the indexer serves it.
    pub fn committee(&self, shard_group: ShardGroup) -> ValidatorSetCommittee {
        let committee = self.signers(shard_group).committee();
        let proof = self.tree.committee_proof(&shard_group, &committee).unwrap();
        ValidatorSetCommittee {
            epoch: self.epoch,
            shard_group,
            validator_set_hash: self.hash(),
            members: (&committee)
                .into_iter()
                .map(|member| ValidatorSetCommitteeMember {
                    public_key: member.public_key,
                    vote_power: member.vote_power.value(),
                })
                .collect(),
            proof: proof.to_bytes(),
        }
    }
}

/// How [`checkpoint_entry`] builds an end-of-epoch checkpoint.
pub struct CheckpointSpec<'a> {
    pub network: Network,
    pub epoch: Epoch,
    pub shard_group: ShardGroup,
    pub signers: &'a Signers,
    /// The validator set the end-of-epoch header names for the next epoch.
    pub next_set: ValidatorSetHash,
    /// Whether the checkpoint carries the header metadata preimage.
    pub carry_preimage: bool,
    pub next_epoch_committee: Option<ValidatorSetCommittee>,
    /// The state version each of these shards of `shard_group` reached at the end of the epoch. Every other shard is
    /// at version zero.
    pub shard_state_versions: Vec<(Shard, u64)>,
}

impl<'a> CheckpointSpec<'a> {
    /// The checkpoint of [`low`] ending `current`'s epoch, which hands over to `next`.
    pub fn handover(current: &'a ValidatorSetFixture, next: &ValidatorSetFixture) -> Self {
        Self {
            network: current.network,
            epoch: current.epoch,
            shard_group: low(),
            signers: &current.low,
            next_set: next.hash(),
            carry_preimage: true,
            next_epoch_committee: Some(next.committee(low())),
            shard_state_versions: vec![],
        }
    }
}

pub fn checkpoint_entry(spec: CheckpointSpec<'_>) -> EpochCheckpointEntry {
    let mut extra_data = ExtraData::new();
    extra_data.insert_validator_set_hash(ExtraFieldKey::NextEpochValidatorSetHash, &spec.next_set);
    let metadata = BlockHeaderMetadata {
        total_leader_fee: 0,
        timestamp: 1234,
        extra_data,
        timeout_certificate_id: None,
        is_dummy: false,
    };

    let command = EndOfEpochCommand::new(FixedHash::from([spec.epoch.as_u64() as u8; 32]));
    let command_hash = TreeHash::new(command.to_command().hash().into_array());
    let (_, inclusion_proof) = compute_proof_for_hashes([command_hash].into_iter(), command_hash).unwrap();
    let mut header = header(spec.network, spec.epoch, spec.shard_group, 10);
    header.command_merkle_root = FixedHash::new(compute_merkle_root_for_hashes([command_hash]).unwrap().into_array());
    header.metadata_hash = metadata.calculate_hash();

    let shard_tree_summary = spec
        .shard_state_versions
        .iter()
        .map(|&(shard, state_version)| {
            (shard, TreeRootSummary {
                root_hash: SPARSE_MERKLE_PLACEHOLDER_HASH,
                state_version,
            })
        })
        .collect::<IndexMap<_, _>>();
    let build = |commit_proof: SidechainBlockCommitProof| {
        EpochCheckpoint::new(
            CommandCommitProof::new(command.clone(), commit_proof, inclusion_proof.clone()),
            shard_tree_summary.clone(),
            spec.carry_preimage.then(|| metadata.clone()),
        )
    };
    let unsigned = build(SidechainBlockCommitProof {
        header: header.clone(),
        proof_elements: vec![],
    });
    header.state_merkle_root = FixedHash::new(unsigned.compute_state_merkle_root().unwrap().into_array());
    let checkpoint = build(sign(header, spec.signers));

    EpochCheckpointEntry {
        epoch: spec.epoch,
        shard_group: spec.shard_group,
        next_epoch_validator_set_hash: checkpoint.next_epoch_validator_set_hash(),
        checkpoint: tari_bor::encode(&checkpoint).unwrap(),
        next_epoch_committee: spec.next_epoch_committee,
    }
}

/// A commit proof of a block of `shard_group` in `epoch` committing `state_merkle_root`, signed by `signers`.
pub fn block_commit_proof(
    network: Network,
    epoch: Epoch,
    shard_group: ShardGroup,
    height: u64,
    state_merkle_root: TreeHash,
    signers: &Signers,
) -> Vec<u8> {
    let mut header = header(network, epoch, shard_group, height);
    header.state_merkle_root = FixedHash::new(state_merkle_root.into_array());
    CommittedBlockProof::new(sign(header, signers)).to_bytes()
}

fn header(network: Network, epoch: Epoch, shard_group: ShardGroup, height: u64) -> SidechainBlockHeader {
    SidechainBlockHeader {
        network: network.as_byte(),
        protocol_version: ProtocolVersion::at(network, epoch).as_u32(),
        parent_id: FixedHash::from([0xaa; 32]),
        justify_id: FixedHash::zero(),
        height,
        epoch: epoch.as_u64(),
        epoch_hash: FixedHash::zero(),
        shard_group: tari_sidechain::ShardGroup {
            start: shard_group.start().as_u32(),
            end_inclusive: shard_group.end().as_u32(),
        },
        proposed_by: CompressedPublicKey::default(),
        state_merkle_root: FixedHash::zero(),
        command_merkle_root: FixedHash::zero(),
        transaction_merkle_root: None,
        signature: ValidatorBlockSignature::default(),
        accumulated_data: Default::default(),
        metadata_hash: FixedHash::zero(),
    }
}

/// Commits `header` with a 3-chain of certificates that every one of `signers` signs.
fn sign(header: SidechainBlockHeader, signers: &Signers) -> SidechainBlockCommitProof {
    let certify = |header_hash: FixedHash, parent_id: FixedHash, height: u64| {
        let block_id = ChainLink { header_hash, parent_id }.calc_block_id();
        let message = ProposalVoteMessage::new(
            header.protocol_version,
            &block_id,
            QuorumDecision::Accept,
            header.epoch,
            height,
        )
        .calculate_hash();
        // Every member signs, so signatures follow committee order: ascending by public key.
        let mut keys = signers.keys.iter().collect::<Vec<_>>();
        keys.sort_by_key(|secret| public_key_bytes(secret));
        let signatures = keys
            .into_iter()
            .map(|secret| {
                let nonce = PrivateKey::from_uniform_bytes(&[secret.as_bytes(), message.as_slice()].concat()).unwrap();
                let signature =
                    DecodedValidatorBlockSignature::sign_with_nonce_and_message(secret, nonce, message).unwrap();
                ValidatorBlockSignature::new_from_schnorr(signature)
            })
            .collect();
        let signatures = QcSignatures::Schnorr {
            signers: SignerBitmap::from_indices(signers.keys.len(), 0..signers.keys.len()).unwrap(),
            signatures,
        };
        QuorumCertificate {
            header_hash,
            parent_id,
            protocol_version: header.protocol_version,
            epoch: header.epoch,
            height,
            signatures,
            decision: QuorumDecision::Accept,
        }
    };

    let block_id = header.calculate_block_id();
    let child_hash = FixedHash::from([0xc1; 32]);
    let child_id = ChainLink {
        header_hash: child_hash,
        parent_id: block_id,
    }
    .calc_block_id();
    let proof_elements = vec![
        CommitProofElement::QuorumCertificate(certify(FixedHash::from([0xc2; 32]), child_id, header.height + 2)),
        CommitProofElement::QuorumCertificate(certify(child_hash, block_id, header.height + 1)),
        CommitProofElement::QuorumCertificate(certify(header.calculate_hash(), header.parent_id, header.height)),
    ];
    SidechainBlockCommitProof { header, proof_elements }
}

/// A proof that `id` at `version` holds `value`, anchored at a block of its shard group in `set`'s epoch that the shard
/// group's committee signed.
pub fn substate_proof(
    set: &ValidatorSetFixture,
    id: &SubstateId,
    version: SubstateVersion,
    value: &SubstateValue,
) -> SubstateProof {
    let versioned_id = VersionedSubstateId::new(id.clone(), version);
    let shard = versioned_id.to_shard(NUM_PRESHARDS);
    let shard_group = shard_group_of(shard);
    let value_hash = hash_substate(set.network, value, version, set.epoch);

    let mut store = MemoryTreeStore::<StateTreePayload>::new();
    let shard_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(None, 1, vec![SubstateTreeChange::Up {
            id: versioned_id.clone(),
            value_hash,
        }])
        .unwrap();
    let group_tree = shard_group_root_tree(set.network, set.epoch, shard, shard_root, 1);
    let (_, _, leaf_proof) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(1, &versioned_id)
        .unwrap();
    let value_proof = SubstateValueProof::new(shard_root, 1, group_tree.get_proof(shard).unwrap().1, leaf_proof);

    SubstateProof {
        anchor: SubstateProofAnchor {
            epoch: set.epoch,
            shard_group,
            height: 5,
            block_id: Default::default(),
            state_merkle_root: Default::default(),
        },
        commit_proof: block_commit_proof(
            set.network,
            set.epoch,
            shard_group,
            5,
            group_tree.root(),
            set.signers(shard_group),
        ),
        value_proof: tari_bor::serde_codec::to_vec(&value_proof).unwrap(),
        value_hash_epoch: set.epoch,
        down_proof: None,
    }
}

/// A proof that `id` at `version` has gone down: up at height 5 of its shard group's chain in `up_set`'s epoch and
/// absent at height 9 in `down_set`'s, each block signed by its set's committee. The proof is anchored at the block it
/// is absent at.
pub fn substate_down_proof(
    up_set: &ValidatorSetFixture,
    down_set: &ValidatorSetFixture,
    id: &SubstateId,
    version: SubstateVersion,
) -> SubstateProof {
    let versioned_id = VersionedSubstateId::new(id.clone(), version);
    let shard = versioned_id.to_shard(NUM_PRESHARDS);
    let shard_group = shard_group_of(shard);

    let mut store = MemoryTreeStore::<StateTreePayload>::new();
    let up_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(None, 1, vec![SubstateTreeChange::Up {
            id: versioned_id.clone(),
            value_hash: Hash32::from_array([5; 32]),
        }])
        .unwrap();
    let down_root = SpreadPrefixStateTree::new(&mut store)
        .put_substate_changes(Some(1), 2, vec![SubstateTreeChange::Down {
            id: versioned_id.clone(),
        }])
        .unwrap();
    let up_tree = shard_group_root_tree(up_set.network, up_set.epoch, shard, up_root, 1);
    let down_tree = shard_group_root_tree(down_set.network, down_set.epoch, shard, down_root, 2);
    let (_, up_value, up_leaf) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(1, &versioned_id)
        .unwrap();
    let (_, _, down_leaf) = SpreadPrefixStateTree::new(&mut store)
        .get_proof(2, &versioned_id)
        .unwrap();
    let down = SubstateValueProof::new(down_root, 2, down_tree.get_proof(shard).unwrap().1, down_leaf);

    let down_proof = SubstateDownProof {
        up: SubstateValueProof::new(up_root, 1, up_tree.get_proof(shard).unwrap().1, up_leaf),
        up_value_hash: up_value.unwrap().0,
        up_commit_proof: block_commit_proof(
            up_set.network,
            up_set.epoch,
            shard_group,
            5,
            up_tree.root(),
            up_set.signers(shard_group),
        ),
        down: down.clone(),
    };
    SubstateProof {
        anchor: SubstateProofAnchor {
            epoch: down_set.epoch,
            shard_group,
            height: 9,
            block_id: Default::default(),
            state_merkle_root: Default::default(),
        },
        commit_proof: block_commit_proof(
            down_set.network,
            down_set.epoch,
            shard_group,
            9,
            down_tree.root(),
            down_set.signers(shard_group),
        ),
        value_proof: tari_bor::serde_codec::to_vec(&down).unwrap(),
        value_hash_epoch: up_set.epoch,
        down_proof: Some(tari_bor::serde_codec::to_vec(&down_proof).unwrap()),
    }
}

/// The root tree of a shard group whose only non-empty shard is `shard`, with root `shard_root` at `version`.
pub fn shard_group_root_tree(
    network: Network,
    epoch: Epoch,
    shard: Shard,
    shard_root: TreeHash,
    version: u64,
) -> ShardGroupRootTree {
    ShardGroupRootTree::build(ProtocolVersion::at(network, epoch), [
        (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
        (shard, shard_root, version),
    ])
    .unwrap()
}
