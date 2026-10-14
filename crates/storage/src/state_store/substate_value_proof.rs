//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, hash_map::Entry};

use ootle_network::Network;
use tari_common_types::types::FixedHash;
use tari_engine_types::{
    ProtocolVersion,
    SubstateVersion,
    limits::MAX_CBOR_NESTING_DEPTH,
    substate::{SubstateId, SubstateValue, hash_substate},
};
use tari_ootle_common_types::{Epoch, NodeHeight, NumPreshards, ShardGroup, VersionedSubstateId, shard::Shard};
use tari_state_tree::{
    SPARSE_MERKLE_PLACEHOLDER_HASH,
    ShardGroupRootTree,
    SparseMerkleProofExt,
    SpreadPrefixStateTree,
    SubstateDownProof,
    SubstateValueProof,
    SubstateValueProofError,
    TreeHash,
    Version,
    jmt_hash_scheme,
};

use crate::{
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{CommittedBlockProof, VerifiedBlockTip},
    state_store::ShardScopedTreeStoreReader,
};

/// Generates two-level [`SubstateValueProof`]s against one committed shard-group state.
///
/// A proof has three parts, and only the last of them varies per substate:
/// - the shard group's per-shard roots, global shard included,
/// - level 2: the proof that a shard's root is committed at that shard's leaf of the shard-group root - one per shard,
/// - level 1: the JMT leaf proof (inclusion or exclusion) for the substate within its shard.
///
/// The first two are read and computed once and reused, so proving N substates costs N leaf
/// traversals rather than N scans of the whole shard group.
///
/// Every proof is rooted at the shard-group `state_merkle_root` - the same root the latest committed
/// block header commits - so a caller that independently trusts that root (e.g. via a verified
/// committed block proof) can verify each substate's committed value or its absence without trusting
/// the node that produced the proofs. One commit proof therefore authenticates every proof a single
/// generator produces, provided both are generated in the same read transaction.
///
/// Whether a leaf proof is an inclusion or exclusion proof depends on whether the substate is
/// currently up in its shard; the caller chooses `verify_inclusion`/`verify_exclusion` accordingly.
pub struct SubstateProofGenerator<'a, TTx> {
    tx: &'a TTx,
    num_preshards: NumPreshards,
    /// The tree over the shard group's per-shard states, global shard included (see
    /// [`ShardGroupRootTree`]). Every level-2 proof is a leaf of this one tree, so it is built once
    /// however many substates are proved - which is what keeps the cost of a batch independent of the
    /// size of the shard group.
    root_tree: ShardGroupRootTree,
    /// The committed state of each shard in the root tree, keyed by shard.
    shards: HashMap<Shard, CommittedShardState>,
    /// Level-2 proofs, extracted from `root_tree` on first use of each shard.
    shard_root_proofs: HashMap<Shard, SparseMerkleProofExt>,
}

#[derive(Debug, Clone, Copy)]
struct CommittedShardState {
    root: TreeHash,
    /// `None` if the shard has no committed state yet, in which case `root` is the empty-tree
    /// placeholder and no substate in the shard can be proved.
    version: Option<Version>,
}

impl<'a, TTx: StateStoreReadTransaction> SubstateProofGenerator<'a, TTx> {
    /// Reads the committed root of every shard in `shard_group`, plus the global shard.
    /// `protocol_version` is the version of the block whose `state_merkle_root` the proofs are rooted at.
    pub fn new(
        tx: &'a TTx,
        shard_group: ShardGroup,
        num_preshards: NumPreshards,
        protocol_version: ProtocolVersion,
    ) -> Result<Self, StorageError> {
        let mut shard_states = Vec::with_capacity(shard_group.len() + 1);
        let mut shards = HashMap::with_capacity(shard_group.len() + 1);
        for shard in shard_group.shard_iter_with_global() {
            let state = committed_shard_state(tx, shard)?;
            shard_states.push((shard, state.root, state.version.unwrap_or_default()));
            shards.insert(shard, state);
        }

        Ok(Self {
            tx,
            num_preshards,
            root_tree: ShardGroupRootTree::build(protocol_version, shard_states).map_err(|e| {
                StorageError::QueryError {
                    reason: format!("SubstateProofGenerator shard group root tree: {e}"),
                }
            })?,
            shards,
            shard_root_proofs: HashMap::new(),
        })
    }

    /// Proves `versioned_id`'s committed value, or its absence, against the shard-group root.
    ///
    /// `Ok(None)` means this state cannot prove anything about the substate, either way: its shard
    /// lies outside the shard group, or that shard has no committed state to root a proof at. A
    /// caller proving many substates can drop that one and keep the rest; an error means the read
    /// itself failed and nothing it produced can be trusted.
    pub fn generate(&mut self, versioned_id: &VersionedSubstateId) -> Result<Option<SubstateValueProof>, StorageError> {
        let shard = versioned_id.to_shard(self.num_preshards);
        let Some(state) = self.shards.get(&shard).copied() else {
            return Ok(None);
        };
        let Some(version) = state.version else {
            return Ok(None);
        };

        // Level 1: leaf proof (inclusion or exclusion) for the substate within its shard.
        let mut scoped = ShardScopedTreeStoreReader::new(self.tx, shard);
        let tree = SpreadPrefixStateTree::new(&mut scoped);
        let (_leaf_key, _proof_value, leaf_proof) =
            tree.get_proof(version, versioned_id)
                .map_err(|e| StorageError::QueryError {
                    reason: format!("SubstateProofGenerator get_proof: {e}"),
                })?;

        // Level 2: prove the shard root is committed in the shard-group root.
        let shard_root_proof = match self.shard_root_proofs.entry(shard) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => {
                let (_, proof) = self.root_tree.get_proof(shard).map_err(|e| StorageError::QueryError {
                    reason: format!("SubstateProofGenerator shard root proof: {e}"),
                })?;
                entry.insert(proof).clone()
            },
        };

        Ok(Some(SubstateValueProof::new(
            state.root,
            version,
            shard_root_proof,
            leaf_proof,
        )))
    }
}

/// A shard-group state merkle root a quorum committed, with the block that committed it.
///
/// It must have been established independently - from a commit proof validated against the shard
/// group committee (`CommittedBlockProof::validate`), either for this read or in an earlier round and
/// recorded in a trusted-root store. The validator pins the substate value proof to the same committed
/// block whose `state_merkle_root` is trusted (proof and commit proof are generated in one read
/// transaction against the same committed block), so verifying against that root is the whole of the
/// check. The trust decision must therefore be keyed on the root itself: a node cannot forge a
/// substate proof that verifies against a root a quorum already signed.
#[derive(Debug, Clone, Copy)]
pub struct TrustedStateRoot {
    /// The epoch of the block that committed `root`, which selects how the root's leaves are formed:
    /// consensus rejects a header whose version is not the schedule's version at its epoch.
    pub epoch: Epoch,
    /// The shard group whose committee committed `root`. The root commits only to the group's own
    /// shards: a shard outside the group has no leaf in it, exactly as an empty shard has none.
    pub shard_group: ShardGroup,
    pub root: FixedHash,
}

impl From<VerifiedBlockTip> for TrustedStateRoot {
    fn from(tip: VerifiedBlockTip) -> Self {
        Self {
            epoch: tip.epoch,
            shard_group: tip.shard_group,
            root: tip.state_merkle_root,
        }
    }
}

impl TrustedStateRoot {
    fn check_contains(&self, substate: &VersionedSubstateId, shard: Shard) -> Result<(), SubstateProofVerifyError> {
        if self.shard_group.contains_or_global(&shard) {
            return Ok(());
        }
        Err(SubstateProofVerifyError::ShardOutsideAnchor {
            substate: substate.clone(),
            shard,
            shard_group: self.shard_group,
        })
    }
}

/// One of the two roots a down proof is verified against, with the height of the block that committed it.
///
/// A down proof orders its roots by height, and a trusted root vouches for a root, not for the height a header claims
/// for it. An anchor is therefore built only from a [`VerifiedBlockTip`], whose every field, height included, the shard
/// group committee signed.
#[derive(Debug, Clone, Copy)]
pub struct DownProofAnchor {
    root: TrustedStateRoot,
    height: NodeHeight,
}

impl From<VerifiedBlockTip> for DownProofAnchor {
    fn from(tip: VerifiedBlockTip) -> Self {
        Self {
            root: tip.into(),
            height: tip.height,
        }
    }
}

impl DownProofAnchor {
    pub fn epoch(&self) -> Epoch {
        self.root.epoch
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.root.shard_group
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn root(&self) -> FixedHash {
        self.root.root
    }

    fn position(&self) -> (Epoch, NodeHeight) {
        (self.epoch(), self.height)
    }
}

/// Verifies a substate value proof against an *already-trusted* shard-group state merkle root,
/// skipping commit-proof (QC chain) validation. A substate whose shard lies outside the root's shard
/// group is rejected.
///
/// `proof_epoch` is the epoch the substate was created at, which selects its value hash.
pub fn verify_substate_value_proof_against_root(
    value_proof_bytes: &[u8],
    substate_id: &SubstateId,
    version: SubstateVersion,
    value: Option<&SubstateValue>,
    network: Network,
    num_preshards: NumPreshards,
    proof_epoch: Epoch,
    trusted_root: &TrustedStateRoot,
) -> Result<(), SubstateProofVerifyError> {
    let group_root = TreeHash::new(trusted_root.root.into_array());
    let root_protocol_version = ProtocolVersion::at(network, trusted_root.epoch);

    let value_proof: SubstateValueProof =
        tari_bor::serde_codec::from_slice_with_max_depth(value_proof_bytes, MAX_CBOR_NESTING_DEPTH)
            .map_err(|e| SubstateProofVerifyError::Decode(e.to_string()))?;

    let versioned_id = VersionedSubstateId::new(substate_id.clone(), version);
    trusted_root.check_contains(&versioned_id, versioned_id.to_shard(num_preshards))?;
    match value {
        Some(value) => {
            // Bind the returned value to the committed leaf by re-deriving its value hash, so a
            // validator cannot swap the value while presenting a proof for the real committed leaf.
            let value_hash = TreeHash::new(hash_substate(network, value, version, proof_epoch).into_array());
            value_proof.verify_inclusion(
                jmt_hash_scheme(root_protocol_version),
                root_protocol_version,
                &group_root,
                num_preshards,
                &versioned_id,
                &value_hash,
            )?;
        },
        None => {
            value_proof.verify_exclusion(
                jmt_hash_scheme(root_protocol_version),
                root_protocol_version,
                &group_root,
                num_preshards,
                &versioned_id,
            )?;
        },
    }

    Ok(())
}

/// Decodes a [`SubstateDownProof`] received from a peer.
pub fn decode_substate_down_proof(proof_bytes: &[u8]) -> Result<SubstateDownProof, SubstateProofVerifyError> {
    tari_bor::serde_codec::from_slice_with_max_depth(proof_bytes, MAX_CBOR_NESTING_DEPTH)
        .map_err(|e| SubstateProofVerifyError::Decode(e.to_string()))
}

/// Verifies that `(substate_id, version)` was committed and has since gone down, against two *already-trusted*
/// shard-group roots: `up_root`, which the proof's `up_commit_proof` commits, and `down_root`, the root of the commit
/// proof the proof was served with.
///
/// The proof holds when `down_root` was committed under V2 or later, both roots' shard groups contain the
/// substate's shard, `up_root` is strictly earlier than `down_root`, the substate is included at `up_root` and
/// excluded at `down_root`. Each root is verified under the protocol version of its own epoch. Exclusion against an
/// earlier root is not bound to the substate's shard (see [`exclusion_is_shard_bound`]), so such a proof is reported
/// as [`SubstateProofVerifyError::DownProofExclusionNotShardBound`]: it does not prove the Down, nor show the proof
/// dishonest.
#[allow(clippy::too_many_arguments)]
pub fn verify_substate_down_proof_against_roots(
    proof_bytes: &[u8],
    substate_id: &SubstateId,
    version: SubstateVersion,
    network: Network,
    num_preshards: NumPreshards,
    up_root: &DownProofAnchor,
    down_root: &DownProofAnchor,
) -> Result<(), SubstateProofVerifyError> {
    // Whatever else the proof holds, an exclusion against such a root cannot show the substate went down.
    let down_protocol_version = ProtocolVersion::at(network, down_root.epoch());
    if !exclusion_is_shard_bound(down_protocol_version) {
        return Err(SubstateProofVerifyError::DownProofExclusionNotShardBound {
            protocol_version: down_protocol_version,
        });
    }

    let proof = decode_substate_down_proof(proof_bytes)?;

    // `up_root` must be the root of the commit proof the proof carries, not one the caller established for another
    // block.
    let up_anchor = CommittedBlockProof::from_bytes(&proof.up_commit_proof)
        .map_err(|e| SubstateProofVerifyError::Decode(e.to_string()))?;
    if up_anchor.state_merkle_root() != up_root.root() ||
        up_anchor.epoch() != up_root.epoch() ||
        up_anchor.height() != up_root.height()
    {
        return Err(SubstateProofVerifyError::DownProofAnchorMismatch);
    }

    let versioned_id = VersionedSubstateId::new(substate_id.clone(), version);
    let shard = versioned_id.to_shard(num_preshards);
    up_root.root.check_contains(&versioned_id, shard)?;
    down_root.root.check_contains(&versioned_id, shard)?;

    // Every shard group commits the global shard on its own chain, so two groups' copies of the global tree are not
    // known to agree: a global substate's inclusion under one group's root and exclusion under another's does not show
    // it went down. An honest node can span a reshard this way, so this leaves the Down unproven rather than refuting
    // it.
    if shard.is_global() && up_root.shard_group() != down_root.shard_group() {
        return Err(SubstateProofVerifyError::DownProofGlobalAcrossShardGroups {
            up_shard_group: up_root.shard_group(),
            down_shard_group: down_root.shard_group(),
        });
    }
    // Heights number the blocks of one shard group's chain, so two roots of one epoch are comparable only within a
    // group. Groups of one epoch do not overlap, so no honest pair of roots holding the same shard differs here.
    if up_root.epoch() == down_root.epoch() && up_root.shard_group() != down_root.shard_group() {
        return Err(SubstateProofVerifyError::DownProofShardGroupMismatch {
            epoch: up_root.epoch(),
            up_shard_group: up_root.shard_group(),
            down_shard_group: down_root.shard_group(),
        });
    }
    if up_root.position() >= down_root.position() {
        return Err(SubstateProofVerifyError::DownProofNotOrdered {
            up_epoch: up_root.epoch(),
            up_height: up_root.height(),
            down_epoch: down_root.epoch(),
            down_height: down_root.height(),
        });
    }

    let up_protocol_version = ProtocolVersion::at(network, up_root.epoch());
    proof
        .up
        .verify_inclusion(
            jmt_hash_scheme(up_protocol_version),
            up_protocol_version,
            &TreeHash::new(up_root.root().into_array()),
            num_preshards,
            &versioned_id,
            &proof.up_value_hash,
        )
        .map_err(SubstateProofVerifyError::DownProofNotUp)?;

    proof
        .down
        .verify_exclusion(
            jmt_hash_scheme(down_protocol_version),
            down_protocol_version,
            &TreeHash::new(down_root.root().into_array()),
            num_preshards,
            &versioned_id,
        )
        .map_err(SubstateProofVerifyError::DownProofNotDown)?;

    Ok(())
}

/// Whether an exclusion proof against a root committed under `protocol_version` can only cite the substate's own
/// shard.
///
/// From V2 the shard-group tree keys each leaf by its shard, so a proof's level-2 path pins the shard it speaks for.
/// Before that a leaf is keyed by its value alone, and any shard's genuine root and state version prove the absence
/// of a substate that is live in another shard. Inclusion is unaffected: a substate's leaf key is unique to its shard.
pub fn exclusion_is_shard_bound(protocol_version: ProtocolVersion) -> bool {
    match protocol_version {
        ProtocolVersion::V0 | ProtocolVersion::V1 => false,
        ProtocolVersion::V2 => true,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubstateProofVerifyError {
    #[error("failed to decode substate value proof: {0}")]
    Decode(String),
    #[error("substate value proof invalid: {0}")]
    ValueProof(#[from] SubstateValueProofError),
    #[error("{substate} is in {shard}, outside {shard_group} whose committed root the proof was anchored to")]
    ShardOutsideAnchor {
        substate: VersionedSubstateId,
        shard: Shard,
        shard_group: ShardGroup,
    },
    #[error(
        "down proof's inclusion root (epoch {up_epoch}, height {up_height}) is not earlier than its exclusion root \
         (epoch {down_epoch}, height {down_height})"
    )]
    DownProofNotOrdered {
        up_epoch: Epoch,
        up_height: NodeHeight,
        down_epoch: Epoch,
        down_height: NodeHeight,
    },
    #[error(
        "down proof's roots are both of epoch {epoch} but of different shard groups ({up_shard_group} and \
         {down_shard_group}), whose heights are not comparable"
    )]
    DownProofShardGroupMismatch {
        epoch: Epoch,
        up_shard_group: ShardGroup,
        down_shard_group: ShardGroup,
    },
    #[error(
        "down proof's roots are of different shard groups ({up_shard_group} and {down_shard_group}), whose copies of \
         the global shard are not known to agree"
    )]
    DownProofGlobalAcrossShardGroups {
        up_shard_group: ShardGroup,
        down_shard_group: ShardGroup,
    },
    #[error(
        "down proof's exclusion root was committed under {protocol_version}, whose shard-group leaves do not name \
         their shard"
    )]
    DownProofExclusionNotShardBound { protocol_version: ProtocolVersion },
    #[error("down proof's inclusion root is not the root of the commit proof it carries")]
    DownProofAnchorMismatch,
    #[error("down proof does not show the substate was committed: {0}")]
    DownProofNotUp(SubstateValueProofError),
    #[error("down proof does not show the substate is absent: {0}")]
    DownProofNotDown(SubstateValueProofError),
}

/// The committed JMT root and state-tree version of `shard`. A shard with no committed state has the
/// empty-tree placeholder for a root and no version.
fn committed_shard_state<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    shard: Shard,
) -> Result<CommittedShardState, StorageError> {
    let Some(version) = tx.state_tree_versions_get_latest(shard)? else {
        return Ok(CommittedShardState {
            root: SPARSE_MERKLE_PLACEHOLDER_HASH,
            version: None,
        });
    };
    let mut scoped = ShardScopedTreeStoreReader::new(tx, shard);
    let tree = SpreadPrefixStateTree::new(&mut scoped);
    let root = tree.get_root_hash(version).map_err(|e| StorageError::QueryError {
        reason: format!("SubstateProofGenerator shard {shard} root: {e}"),
    })?;
    Ok(CommittedShardState {
        root,
        version: Some(version),
    })
}

#[cfg(test)]
mod tests {
    use tari_state_tree::{JmtHashScheme, StateTreePayload, compute_shard_group_root, memory_store::MemoryTreeStore};
    use tari_template_lib_types::{ComponentAddress, ObjectKey};

    use super::*;

    const NUM_PRESHARDS: NumPreshards = NumPreshards::P4;

    /// A shard group's root commits to the shards in the group and nothing else, so a substate in a
    /// shard outside the group has no leaf there, the same as a substate in an empty shard. An
    /// absence proved against another group's root must not count as the substate's absence.
    #[test]
    fn an_absence_proved_against_another_shard_groups_root_is_rejected() {
        let substate_id = SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([1; ObjectKey::LENGTH])));
        let version = SubstateVersion::ZERO;
        let versioned_id = VersionedSubstateId::new(substate_id.clone(), version);
        let own_shard = versioned_id.to_shard(NUM_PRESHARDS);
        let other_shard = Shard::from_u32(if own_shard.as_u32() == 1 { 2 } else { 1 });
        let other_group = ShardGroup::new(other_shard, other_shard);

        let other_group_states = [
            (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
            (other_shard, TreeHash::new([7; 32]), 1),
        ];
        let other_group_root = compute_shard_group_root(ProtocolVersion::V2, other_group_states).unwrap();
        // The substate's shard has no leaf in the other group's root, so the proof that its key is
        // absent - the proof an empty shard gets - verifies against that root.
        let with_own_shard_empty = ShardGroupRootTree::build(
            ProtocolVersion::V2,
            other_group_states
                .into_iter()
                .chain([(own_shard, SPARSE_MERKLE_PLACEHOLDER_HASH, 0)]),
        )
        .unwrap();
        assert_eq!(with_own_shard_empty.root(), other_group_root);
        let (_, shard_root_proof) = with_own_shard_empty.get_proof(own_shard).unwrap();
        let mut empty = MemoryTreeStore::<StateTreePayload>::new();
        SpreadPrefixStateTree::new(&mut empty)
            .put_substate_changes(None, 1, vec![])
            .unwrap();
        let (_, _, leaf_proof) = SpreadPrefixStateTree::new(&mut empty)
            .get_proof(1, &versioned_id)
            .unwrap();
        let proof = SubstateValueProof::new(SPARSE_MERKLE_PLACEHOLDER_HASH, 0, shard_root_proof, leaf_proof);
        proof
            .verify_exclusion(
                JmtHashScheme::V1,
                ProtocolVersion::V2,
                &other_group_root,
                NUM_PRESHARDS,
                &versioned_id,
            )
            .unwrap();
        let proof_bytes = tari_bor::serde_codec::to_vec(&proof).unwrap();

        let result = verify_substate_value_proof_against_root(
            &proof_bytes,
            &substate_id,
            version,
            None,
            Network::LocalNet,
            NUM_PRESHARDS,
            Epoch(1),
            &TrustedStateRoot {
                epoch: Epoch(1),
                shard_group: other_group,
                root: FixedHash::new(other_group_root.into_array()),
            },
        );
        assert!(
            matches!(result, Err(SubstateProofVerifyError::ShardOutsideAnchor { .. })),
            "{result:?}"
        );
    }

    /// An anchor is built only from a validated tip, and carries the height the committee signed.
    #[test]
    fn a_down_proof_anchor_carries_its_validated_tip() {
        let tip = VerifiedBlockTip {
            epoch: Epoch(3),
            shard_group: ShardGroup::new(Shard::from_u32(1), Shard::from_u32(2)),
            height: NodeHeight(9),
            block_id: FixedHash::from([1; 32]),
            epoch_hash: FixedHash::from([2; 32]),
            state_merkle_root: FixedHash::from([3; 32]),
        };
        let anchor = DownProofAnchor::from(tip);
        assert_eq!(anchor.epoch(), tip.epoch);
        assert_eq!(anchor.shard_group(), tip.shard_group);
        assert_eq!(anchor.height(), tip.height);
        assert_eq!(anchor.root(), tip.state_merkle_root);
    }

    mod down_proof {
        use tari_sidechain::{SidechainBlockCommitProof, SidechainBlockHeader};
        use tari_state_tree::{SubstateTreeChange, memory_store::MemoryTreeStore};
        use tari_template_lib_types::Hash32;

        use super::*;

        /// V2 on every epoch.
        const NETWORK: Network = Network::LocalNet;
        const EPOCH: Epoch = Epoch(1);

        /// The tip `commit_proof`'s header describes, taken at its word: these commit proofs are not signed.
        fn unvalidated_tip(commit_proof: &CommittedBlockProof) -> VerifiedBlockTip {
            VerifiedBlockTip {
                epoch: commit_proof.epoch(),
                shard_group: commit_proof.shard_group().unwrap(),
                height: commit_proof.height(),
                block_id: commit_proof.block_id(),
                epoch_hash: commit_proof.epoch_hash(),
                state_merkle_root: commit_proof.state_merkle_root(),
            }
        }

        fn id(seed: u8) -> SubstateId {
            SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([seed; ObjectKey::LENGTH])))
        }

        fn up(id: &VersionedSubstateId, seed: u8) -> SubstateTreeChange {
            SubstateTreeChange::Up {
                id: id.clone(),
                value_hash: Hash32::from_array([seed; 32]),
            }
        }

        fn down(id: &VersionedSubstateId) -> SubstateTreeChange {
            SubstateTreeChange::Down { id: id.clone() }
        }

        /// A shard of the same group as `shard`.
        fn sibling_of(shard: Shard) -> Shard {
            if shard.as_u32() == 1 {
                Shard::from_u32(2)
            } else {
                Shard::from_u32(shard.as_u32() - 1)
            }
        }

        fn group_of(a: Shard, b: Shard) -> ShardGroup {
            ShardGroup::new(a.min(b), a.max(b))
        }

        fn commit_proof(shard_group: ShardGroup, epoch: Epoch, height: u64, root: TreeHash) -> CommittedBlockProof {
            let header = SidechainBlockHeader {
                network: NETWORK.as_byte(),
                protocol_version: ProtocolVersion::at(NETWORK, epoch).as_u32(),
                parent_id: FixedHash::zero(),
                justify_id: FixedHash::zero(),
                height,
                epoch: epoch.as_u64(),
                epoch_hash: FixedHash::zero(),
                shard_group: tari_sidechain::ShardGroup {
                    start: shard_group.start().as_u32(),
                    end_inclusive: shard_group.end().as_u32(),
                },
                proposed_by: Default::default(),
                state_merkle_root: FixedHash::new(root.into_array()),
                command_merkle_root: FixedHash::zero(),
                transaction_merkle_root: None,
                signature: Default::default(),
                accumulated_data: Default::default(),
                metadata_hash: FixedHash::zero(),
            };
            CommittedBlockProof::new(SidechainBlockCommitProof {
                header,
                proof_elements: vec![],
            })
        }

        /// A substate's shard at two state versions, with a sibling shard of the same group beside it.
        ///
        /// Version 1 creates `target` and, unless `alone`, an unrelated substate; version 2 applies `at_v2`. R1 is
        /// the group root at version 1 (height 2) and R2 the group root at version 2 (height 4).
        struct Scenario {
            target: VersionedSubstateId,
            shard: Shard,
            sibling: Shard,
            own: MemoryTreeStore<StateTreePayload>,
            sibling_store: MemoryTreeStore<StateTreePayload>,
            sibling_root: TreeHash,
            r1_tree: ShardGroupRootTree,
            r1_shard_root: TreeHash,
            r2_tree: ShardGroupRootTree,
            r2_shard_root: TreeHash,
        }

        impl Scenario {
            fn new(alone: bool, at_v2: impl FnOnce(&VersionedSubstateId) -> Vec<SubstateTreeChange>) -> Self {
                let target = VersionedSubstateId::new(id(1), SubstateVersion::ZERO);
                let shard = target.to_shard(NUM_PRESHARDS);
                let sibling = sibling_of(shard);

                let mut own = MemoryTreeStore::<StateTreePayload>::new();
                let mut v1 = vec![up(&target, 10)];
                if !alone {
                    // A substate of the same shard, so the shard is not emptied when the target goes down.
                    let neighbour = (2..=u8::MAX)
                        .map(|seed| VersionedSubstateId::new(id(seed), SubstateVersion::ZERO))
                        .find(|v| v.to_shard(NUM_PRESHARDS) == shard)
                        .unwrap();
                    v1.push(up(&neighbour, 11));
                }
                let r1_shard_root = SpreadPrefixStateTree::new(&mut own)
                    .put_substate_changes(None, 1, v1)
                    .unwrap();
                let r2_shard_root = SpreadPrefixStateTree::new(&mut own)
                    .put_substate_changes(Some(1), 2, at_v2(&target))
                    .unwrap();

                // The sibling holds a substate whose leaf key differs from the target's, so its tree proves the
                // target absent.
                let mut sibling_store = MemoryTreeStore::<StateTreePayload>::new();
                let sibling_root = SpreadPrefixStateTree::new(&mut sibling_store)
                    .put_substate_changes(None, 1, vec![up(
                        &VersionedSubstateId::new(id(200), SubstateVersion::ZERO),
                        12,
                    )])
                    .unwrap();

                let group_tree = |shard_root, shard_version| {
                    ShardGroupRootTree::build(ProtocolVersion::at(NETWORK, EPOCH), [
                        (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
                        (shard, shard_root, shard_version),
                        (sibling, sibling_root, 1),
                    ])
                    .unwrap()
                };
                Self {
                    r1_tree: group_tree(r1_shard_root, 1),
                    r2_tree: group_tree(r2_shard_root, 2),
                    target,
                    shard,
                    sibling,
                    own,
                    sibling_store,
                    sibling_root,
                    r1_shard_root,
                    r2_shard_root,
                }
            }

            fn group(&self) -> ShardGroup {
                group_of(self.shard, self.sibling)
            }

            fn r1_commit_proof(&self, height: u64) -> CommittedBlockProof {
                commit_proof(self.group(), EPOCH, height, self.r1_tree.root())
            }

            fn r1_tip(&self, height: u64) -> VerifiedBlockTip {
                unvalidated_tip(&self.r1_commit_proof(height))
            }

            fn r1(&self, height: u64) -> DownProofAnchor {
                self.r1_tip(height).into()
            }

            fn r2_tip(&self) -> VerifiedBlockTip {
                unvalidated_tip(&commit_proof(self.group(), EPOCH, 4, self.r2_tree.root()))
            }

            fn r2(&self) -> DownProofAnchor {
                self.r2_tip().into()
            }

            /// `(proof, value hash)` of `id` in the substate's shard at version 1, under R1.
            fn up_proof(&mut self, id: &VersionedSubstateId) -> (SubstateValueProof, Option<TreeHash>) {
                let (_, value, leaf_proof) = SpreadPrefixStateTree::new(&mut self.own).get_proof(1, id).unwrap();
                let (_, shard_root_proof) = self.r1_tree.get_proof(self.shard).unwrap();
                (
                    SubstateValueProof::new(self.r1_shard_root, 1, shard_root_proof, leaf_proof),
                    value.map(|(hash, _, _)| hash),
                )
            }

            /// Proof of `id` in the substate's shard at version 2, under R2.
            fn down_proof(&mut self, id: &VersionedSubstateId) -> SubstateValueProof {
                let (_, _, leaf_proof) = SpreadPrefixStateTree::new(&mut self.own).get_proof(2, id).unwrap();
                let (_, shard_root_proof) = self.r2_tree.get_proof(self.shard).unwrap();
                SubstateValueProof::new(self.r2_shard_root, 2, shard_root_proof, leaf_proof)
            }

            fn honest(&mut self, id: &VersionedSubstateId) -> SubstateDownProof {
                let (up, up_value_hash) = self.up_proof(id);
                SubstateDownProof {
                    up,
                    up_value_hash: up_value_hash.unwrap_or(TreeHash::new([9; 32])),
                    up_commit_proof: self.r1_commit_proof(2).to_bytes(),
                    down: self.down_proof(id),
                }
            }

            fn verify(
                &self,
                proof: &SubstateDownProof,
                id: &VersionedSubstateId,
                r1: DownProofAnchor,
            ) -> Result<(), SubstateProofVerifyError> {
                verify_substate_down_proof_against_roots(
                    &tari_bor::serde_codec::to_vec(proof).unwrap(),
                    id.substate_id(),
                    id.version(),
                    NETWORK,
                    NUM_PRESHARDS,
                    &r1,
                    &self.r2(),
                )
            }
        }

        fn destroyed_and_replaced(target: &VersionedSubstateId) -> Vec<SubstateTreeChange> {
            vec![
                down(target),
                up(
                    &VersionedSubstateId::new(target.substate_id().clone(), target.version().next()),
                    20,
                ),
            ]
        }

        #[test]
        fn an_honest_down_proof_verifies() {
            let mut scenario = Scenario::new(false, destroyed_and_replaced);
            let target = scenario.target.clone();
            let proof = scenario.honest(&target);
            scenario.verify(&proof, &target, scenario.r1(2)).unwrap();
        }

        /// A live substate is absent from every shard but its own, so a sibling shard's genuine tree "proves" its
        /// absence. Under V2 that tree's leaf names the sibling, so the proof fails.
        #[test]
        fn an_exclusion_lifted_from_another_shard_is_rejected() {
            let mut scenario = Scenario::new(false, |_| vec![]);
            let target = scenario.target.clone();
            let mut proof = scenario.honest(&target);
            let (_, _, sibling_leaf_proof) = SpreadPrefixStateTree::new(&mut scenario.sibling_store)
                .get_proof(1, &target)
                .unwrap();
            for shard_root_proof in [
                scenario.r2_tree.get_proof(scenario.sibling).unwrap().1,
                scenario.r2_tree.get_proof(scenario.shard).unwrap().1,
            ] {
                proof.down =
                    SubstateValueProof::new(scenario.sibling_root, 1, shard_root_proof, sibling_leaf_proof.clone());
                let result = scenario.verify(&proof, &target, scenario.r1(2));
                assert!(
                    matches!(result, Err(SubstateProofVerifyError::DownProofNotDown(_))),
                    "{result:?}"
                );
            }
            // And the substate's own shard cannot show it absent.
            proof.down = scenario.down_proof(&target);
            let result = scenario.verify(&proof, &target, scenario.r1(2));
            assert!(
                matches!(result, Err(SubstateProofVerifyError::DownProofNotDown(_))),
                "{result:?}"
            );
        }

        #[test]
        fn a_version_that_never_existed_is_not_down() {
            let mut scenario = Scenario::new(false, destroyed_and_replaced);
            let never = VersionedSubstateId::new(scenario.target.substate_id().clone(), SubstateVersion::new(7));
            let proof = scenario.honest(&never);
            let result = scenario.verify(&proof, &never, scenario.r1(2));
            assert!(
                matches!(result, Err(SubstateProofVerifyError::DownProofNotUp(_))),
                "{result:?}"
            );
        }

        #[test]
        fn an_inclusion_root_that_is_not_earlier_is_rejected() {
            let mut scenario = Scenario::new(false, destroyed_and_replaced);
            let target = scenario.target.clone();
            for height in [4, 5] {
                let mut proof = scenario.honest(&target);
                proof.up_commit_proof = scenario.r1_commit_proof(height).to_bytes();
                let result = scenario.verify(&proof, &target, scenario.r1(height));
                assert!(
                    matches!(result, Err(SubstateProofVerifyError::DownProofNotOrdered { .. })),
                    "{result:?}"
                );
            }
        }

        #[test]
        fn an_inclusion_root_other_than_the_carried_anchor_is_rejected() {
            let mut scenario = Scenario::new(false, destroyed_and_replaced);
            let target = scenario.target.clone();
            let proof = scenario.honest(&target);
            let result = scenario.verify(&proof, &target, scenario.r1(3));
            assert!(
                matches!(result, Err(SubstateProofVerifyError::DownProofAnchorMismatch)),
                "{result:?}"
            );
        }

        #[test]
        fn a_tampered_value_hash_is_rejected() {
            let mut scenario = Scenario::new(false, destroyed_and_replaced);
            let target = scenario.target.clone();
            let mut proof = scenario.honest(&target);
            proof.up_value_hash = TreeHash::new([42; 32]);
            let result = scenario.verify(&proof, &target, scenario.r1(2));
            assert!(
                matches!(result, Err(SubstateProofVerifyError::DownProofNotUp(_))),
                "{result:?}"
            );
        }

        /// Destroying the only substate of a shard leaves the shard at the empty-tree root, which its V2 leaf binds to
        /// the shard and its state version.
        #[test]
        fn a_shard_emptied_by_the_destruction_proves_it_down() {
            let mut scenario = Scenario::new(true, |target| vec![down(target)]);
            assert_eq!(scenario.r2_shard_root, SPARSE_MERKLE_PLACEHOLDER_HASH);
            let target = scenario.target.clone();
            let proof = scenario.honest(&target);
            scenario.verify(&proof, &target, scenario.r1(2)).unwrap();
        }

        /// A global substate's shard is in every group, and each group commits it on its own chain. Roots of two
        /// groups, in one epoch or in different ones, do not show it went down.
        #[test]
        fn roots_of_different_groups_do_not_prove_a_global_substate_down() {
            use tari_engine_types::published_template::PublishedTemplateAddress;

            let target = VersionedSubstateId::new(
                SubstateId::Template(PublishedTemplateAddress::from_hash(Hash32::from_array([4; 32]))),
                SubstateVersion::ZERO,
            );
            assert_eq!(target.to_shard(NUM_PRESHARDS), Shard::global());

            let mut global = MemoryTreeStore::<StateTreePayload>::new();
            let r1_global_root = SpreadPrefixStateTree::new(&mut global)
                .put_substate_changes(None, 1, vec![up(&target, 10)])
                .unwrap();
            let r2_global_root = SpreadPrefixStateTree::new(&mut global)
                .put_substate_changes(Some(1), 2, vec![down(&target)])
                .unwrap();
            let (_, value, up_leaf) = SpreadPrefixStateTree::new(&mut global).get_proof(1, &target).unwrap();
            let (_, _, down_leaf) = SpreadPrefixStateTree::new(&mut global).get_proof(2, &target).unwrap();

            let group_a = ShardGroup::new(Shard::from_u32(1), Shard::from_u32(2));
            let group_b = ShardGroup::new(Shard::from_u32(3), Shard::from_u32(4));
            let tree = |shard_root, version| {
                ShardGroupRootTree::build(ProtocolVersion::at(NETWORK, EPOCH), [(
                    Shard::global(),
                    shard_root,
                    version,
                )])
                .unwrap()
            };
            let r1_tree = tree(r1_global_root, 1);
            let r2_tree = tree(r2_global_root, 2);

            let verify = |down_group: ShardGroup, down_epoch: Epoch| {
                let up_commit_proof = commit_proof(group_a, EPOCH, 2, r1_tree.root());
                let proof = SubstateDownProof {
                    up: SubstateValueProof::new(
                        r1_global_root,
                        1,
                        r1_tree.get_proof(Shard::global()).unwrap().1,
                        up_leaf.clone(),
                    ),
                    up_value_hash: value.unwrap().0,
                    up_commit_proof: up_commit_proof.to_bytes(),
                    down: SubstateValueProof::new(
                        r2_global_root,
                        2,
                        r2_tree.get_proof(Shard::global()).unwrap().1,
                        down_leaf.clone(),
                    ),
                };
                let down_root = DownProofAnchor::from(unvalidated_tip(&commit_proof(
                    down_group,
                    down_epoch,
                    4,
                    r2_tree.root(),
                )));
                verify_substate_down_proof_against_roots(
                    &tari_bor::serde_codec::to_vec(&proof).unwrap(),
                    target.substate_id(),
                    target.version(),
                    NETWORK,
                    NUM_PRESHARDS,
                    &DownProofAnchor::from(unvalidated_tip(&up_commit_proof)),
                    &down_root,
                )
            };

            for epoch in [EPOCH, Epoch(EPOCH.as_u64() + 1)] {
                let result = verify(group_b, epoch);
                assert!(
                    matches!(
                        result,
                        Err(SubstateProofVerifyError::DownProofGlobalAcrossShardGroups { .. })
                    ),
                    "{epoch}: {result:?}"
                );
                verify(group_a, epoch).unwrap();
            }
        }

        /// Before V2 another shard's genuine root proves any substate absent, so a down proof whose exclusion root
        /// precedes V2 shows nothing. Esmeralda runs V0 then V1 and has not scheduled V2.
        #[test]
        fn an_exclusion_root_before_v2_does_not_prove_a_down() {
            let mut scenario = Scenario::new(false, destroyed_and_replaced);
            let target = scenario.target.clone();
            let proof = tari_bor::serde_codec::to_vec(scenario.honest(&target)).unwrap();
            for epoch in [Epoch(1), Epoch(11925)] {
                let protocol_version = ProtocolVersion::at(Network::Esmeralda, epoch);
                assert!(matches!(protocol_version, ProtocolVersion::V0 | ProtocolVersion::V1));
                let mut up_tip = scenario.r1_tip(2);
                let mut down_tip = scenario.r2_tip();
                up_tip.epoch = epoch;
                down_tip.epoch = epoch;
                let (up_root, down_root) = (DownProofAnchor::from(up_tip), DownProofAnchor::from(down_tip));
                let result = verify_substate_down_proof_against_roots(
                    &proof,
                    target.substate_id(),
                    target.version(),
                    Network::Esmeralda,
                    NUM_PRESHARDS,
                    &up_root,
                    &down_root,
                );
                assert!(
                    matches!(
                        result,
                        Err(SubstateProofVerifyError::DownProofExclusionNotShardBound { .. })
                    ),
                    "{epoch}: {result:?}"
                );
            }
        }

        #[test]
        fn shard_bound_exclusion_starts_at_v2() {
            assert!(!exclusion_is_shard_bound(ProtocolVersion::V0));
            assert!(!exclusion_is_shard_bound(ProtocolVersion::V1));
            assert!(exclusion_is_shard_bound(ProtocolVersion::V2));
        }

        /// Groups of one epoch do not overlap, so no honest pair of roots of one epoch holds the substate's shard in
        /// two different groups.
        #[test]
        fn roots_of_different_groups_in_one_epoch_are_rejected() {
            let mut scenario = Scenario::new(false, destroyed_and_replaced);
            let target = scenario.target.clone();
            let proof = scenario.honest(&target);
            let mut r2_tip = scenario.r2_tip();
            r2_tip.shard_group = ShardGroup::new(Shard::from_u32(1), Shard::from_u32(NUM_PRESHARDS.as_u32()));
            assert_ne!(r2_tip.shard_group, scenario.group());
            let r2 = DownProofAnchor::from(r2_tip);
            let result = verify_substate_down_proof_against_roots(
                &tari_bor::serde_codec::to_vec(&proof).unwrap(),
                target.substate_id(),
                target.version(),
                NETWORK,
                NUM_PRESHARDS,
                &scenario.r1(2),
                &r2,
            );
            assert!(
                matches!(
                    result,
                    Err(SubstateProofVerifyError::DownProofShardGroupMismatch { .. })
                ),
                "{result:?}"
            );
        }

        #[test]
        fn a_root_whose_group_does_not_hold_the_shard_is_rejected() {
            let mut scenario = Scenario::new(false, destroyed_and_replaced);
            let target = scenario.target.clone();
            let proof = scenario.honest(&target);
            let elsewhere = (1..=4).map(Shard::from_u32).find(|s| *s != scenario.shard).unwrap();
            let mut r2_tip = scenario.r2_tip();
            r2_tip.shard_group = ShardGroup::new(elsewhere, elsewhere);
            let r2 = DownProofAnchor::from(r2_tip);
            let result = verify_substate_down_proof_against_roots(
                &tari_bor::serde_codec::to_vec(&proof).unwrap(),
                target.substate_id(),
                target.version(),
                NETWORK,
                NUM_PRESHARDS,
                &scenario.r1(2),
                &r2,
            );
            assert!(
                matches!(result, Err(SubstateProofVerifyError::ShardOutsideAnchor { .. })),
                "{result:?}"
            );
        }
    }
}
