//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_epoch_manager::{EpochManagerReader, service::EpochManagerHandle};
use tari_indexer_client::types::{
    EpochCheckpointEntry,
    SubstateProof,
    SubstateProofAnchor,
    ValidatorSetCommittee,
    ValidatorSetCommitteeMember,
};
use tari_ootle_common_types::{Epoch, NumPreshards, ValidatorSetHash, optional::Optional, shard::Shard};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::consensus_models::{CommittedBlockProof, EpochCheckpoint};
use tari_state_tree::ValidatorSetCommitteeProof;
use tari_template_lib_types::Hash32;
use tari_validator_node_rpc::client::SubstateProofData;

use crate::rest_api::{committee_cache::CommitteeProofCache, error::ErrorResponse};

/// Rejects a request for proofs from an indexer that does not verify them, and so never holds one.
pub fn require_proof_verification(verifies_substates: bool) -> Result<(), ErrorResponse> {
    if verifies_substates {
        return Ok(());
    }
    Err(ErrorResponse::bad_request(
        "This indexer does not verify substate proofs, so it has none to include. Retry without requesting proofs, or \
         use an indexer with verify_substate_proofs enabled.",
    ))
}

/// The client form of a proof the indexer has verified, with the anchor read out of its commit proof.
pub fn to_substate_proof(proof: SubstateProofData) -> Result<SubstateProof, ErrorResponse> {
    let commit_proof = CommittedBlockProof::from_bytes(&proof.commit_proof)
        .map_err(|e| ErrorResponse::internal_error(format!("Verified commit proof failed to decode: {e}")))?;
    let shard_group = commit_proof
        .shard_group()
        .map_err(|e| ErrorResponse::internal_error(format!("Verified commit proof has an invalid shard group: {e}")))?;
    let anchor = SubstateProofAnchor {
        epoch: commit_proof.epoch(),
        shard_group,
        height: commit_proof.height().as_u64(),
        block_id: Hash32::from_array(commit_proof.block_id().into_array()),
        state_merkle_root: Hash32::from_array(commit_proof.state_merkle_root().into_array()),
    };
    Ok(SubstateProof {
        anchor,
        commit_proof: proof.commit_proof,
        value_proof: proof.substate_value_proof,
        value_hash_epoch: Epoch(proof.proof_epoch),
        down_proof: proof.substate_down_proof,
    })
}

/// The network's number of preshards, which every epoch's shard groups partition.
pub async fn num_preshards(epoch_manager: &EpochManagerHandle<PeerAddress>) -> Result<NumPreshards, ErrorResponse> {
    Ok(epoch_manager
        .get_network_description()
        .await
        .map_err(ErrorResponse::anyhow)?
        .num_preshards)
}

/// `epoch`'s committee for the shard group that contains `shard`, with its proof against the epoch's validator set
/// hash. `None` if this indexer holds no validator set for `epoch`.
pub async fn committee_for_shard(
    epoch_manager: &EpochManagerHandle<PeerAddress>,
    num_preshards: NumPreshards,
    epoch: Epoch,
    shard: Shard,
) -> Result<Option<ValidatorSetCommittee>, ErrorResponse> {
    let num_committees = epoch_manager
        .get_num_committees(epoch)
        .await
        .map_err(ErrorResponse::anyhow)?;
    let Some(shard_group) = num_preshards
        .all_shard_groups_iter(num_committees)
        .find(|shard_group| shard_group.contains(&shard))
    else {
        return Err(ErrorResponse::bad_request(format!(
            "Shard {shard} is not in any shard group of {epoch}"
        )));
    };
    let proof = epoch_manager
        .get_validator_set_committee_proof(epoch, shard_group)
        .await
        .optional()
        .map_err(ErrorResponse::anyhow)?;
    Ok(proof.as_ref().map(to_validator_set_committee))
}

/// `epoch`'s committee for the shard group that contains `shard`, for a caller that checks it against
/// `validator_set_hash`. The committee comes from `cache` when it holds one; otherwise from the epoch manager, and
/// `cache` keeps it only if it proves against `validator_set_hash`. A committee that proves against another hash is
/// still returned, for its caller to reject.
pub async fn committee_proved_against(
    cache: &CommitteeProofCache,
    epoch_manager: &EpochManagerHandle<PeerAddress>,
    epoch: Epoch,
    validator_set_hash: &ValidatorSetHash,
    shard: Shard,
) -> Result<Option<ValidatorSetCommittee>, ErrorResponse> {
    if let Some(committee) = cache.get(epoch, validator_set_hash, shard) {
        return Ok(Some(committee));
    }
    let committee = committee_for_shard(epoch_manager, num_preshards(epoch_manager).await?, epoch, shard).await?;
    if let Some(committee) = committee
        .as_ref()
        .filter(|committee| committee.validator_set_hash == *validator_set_hash)
    {
        cache.insert(committee.clone());
    }
    Ok(committee)
}

/// The client form of a committee proof, with the members and set hash read out of it.
pub fn to_validator_set_committee(proof: &ValidatorSetCommitteeProof) -> ValidatorSetCommittee {
    ValidatorSetCommittee {
        epoch: proof.epoch,
        shard_group: proof.shard_group,
        validator_set_hash: proof.validator_set_hash(),
        members: proof
            .members
            .iter()
            .map(|member| ValidatorSetCommitteeMember {
                public_key: member.public_key,
                vote_power: member.vote_power.value(),
            })
            .collect(),
        proof: proof.to_bytes(),
    }
}

/// The client form of a stored checkpoint, with the fields a caller selects on read out of it.
pub fn to_checkpoint_entry(
    checkpoint: &EpochCheckpoint,
    next_epoch_committee: Option<ValidatorSetCommittee>,
) -> Result<EpochCheckpointEntry, ErrorResponse> {
    let shard_group = checkpoint
        .checked_shard_group()
        .map_err(|e| ErrorResponse::internal_error(format!("Stored checkpoint has an invalid shard group: {e}")))?;
    let encoded = tari_bor::encode(checkpoint)
        .map_err(|e| ErrorResponse::internal_error(format!("Stored checkpoint failed to encode: {e}")))?;
    Ok(EpochCheckpointEntry {
        epoch: checkpoint.epoch(),
        shard_group,
        next_epoch_validator_set_hash: checkpoint.next_epoch_validator_set_hash(),
        checkpoint: encoded,
        next_epoch_committee,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use tari_common_types::types::{CompressedPublicKey, FixedHash};
    use tari_crypto::ristretto::RistrettoSecretKey;
    use tari_ootle_common_types::ShardGroup;
    use tari_sidechain::{SidechainBlockCommitProof, SidechainBlockHeader, ValidatorBlockSignature};

    use super::*;

    pub(crate) fn commit_proof() -> CommittedBlockProof {
        CommittedBlockProof::new(SidechainBlockCommitProof {
            header: SidechainBlockHeader {
                network: 0,
                protocol_version: 0,
                parent_id: FixedHash::zero(),
                justify_id: FixedHash::zero(),
                height: 7,
                epoch: 3,
                epoch_hash: FixedHash::zero(),
                shard_group: tari_sidechain::ShardGroup {
                    start: 1,
                    end_inclusive: 4,
                },
                proposed_by: CompressedPublicKey::default(),
                state_merkle_root: FixedHash::new([7u8; 32]),
                command_merkle_root: FixedHash::zero(),
                transaction_merkle_root: None,
                signature: ValidatorBlockSignature::new(CompressedPublicKey::default(), RistrettoSecretKey::default()),
                accumulated_data: Default::default(),
                metadata_hash: FixedHash::zero(),
            },
            proof_elements: vec![],
        })
    }

    #[test]
    fn the_anchor_is_read_from_the_commit_proof_and_the_proof_bytes_pass_through() {
        let commit_proof = commit_proof();
        let proof = to_substate_proof(SubstateProofData {
            substate_value_proof: vec![1, 2, 3],
            commit_proof: commit_proof.to_bytes(),
            proof_epoch: 2,
            substate_down_proof: None,
            destroyed_at_state_version: None,
        })
        .unwrap();

        assert_eq!(proof.anchor.epoch, Epoch(3));
        assert_eq!(proof.anchor.shard_group, ShardGroup::new_checked(1, 4).unwrap());
        assert_eq!(proof.anchor.height, 7);
        assert_eq!(proof.anchor.state_merkle_root, Hash32::from_array([7u8; 32]));
        assert_eq!(
            proof.anchor.block_id,
            Hash32::from_array(commit_proof.block_id().into_array())
        );
        assert_eq!(proof.commit_proof, commit_proof.to_bytes());
        assert_eq!(proof.value_proof, vec![1, 2, 3]);
        assert_eq!(proof.value_hash_epoch, Epoch(2));
        assert_eq!(proof.down_proof, None);
    }

    #[test]
    fn a_down_proof_passes_through() {
        let proof = to_substate_proof(SubstateProofData {
            substate_value_proof: vec![1, 2, 3],
            commit_proof: commit_proof().to_bytes(),
            proof_epoch: 2,
            substate_down_proof: Some(vec![4, 5, 6]),
            destroyed_at_state_version: Some(9),
        })
        .unwrap();

        assert_eq!(proof.down_proof, Some(vec![4, 5, 6]));
        assert_eq!(proof.value_proof, vec![1, 2, 3]);
    }

    #[test]
    fn a_committee_carries_its_members_set_hash_and_decodable_proof() {
        use ootle_network::Network;
        use tari_ootle_common_types::{
            NumPreshards,
            VotePower,
            committee::{Committee, CommitteeMember},
        };
        use tari_state_tree::ValidatorSetTree;
        use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

        let shard_group = ShardGroup::new(0u32, 3u32);
        let committee = Committee::new(
            [1u8, 2]
                .into_iter()
                .map(|byte| CommitteeMember {
                    address: byte,
                    public_key: RistrettoPublicKeyBytes::from_bytes(&[byte; 32]).unwrap(),
                    vote_power: VotePower::of(1),
                })
                .collect(),
        );
        let tree = ValidatorSetTree::build(Network::LocalNet, Epoch(5), NumPreshards::try_from(4u32).unwrap(), [(
            shard_group,
            committee.calculate_validator_set_leaf_hash(&shard_group).unwrap(),
        )])
        .unwrap();
        let proof = tree.committee_proof(&shard_group, &committee).unwrap();

        let served = to_validator_set_committee(&proof);

        assert_eq!(served.epoch, Epoch(5));
        assert_eq!(served.shard_group, shard_group);
        assert_eq!(served.validator_set_hash, tree.hash());
        assert_eq!(
            served.members.iter().map(|m| m.public_key).collect::<Vec<_>>(),
            proof.members.iter().map(|m| m.public_key).collect::<Vec<_>>()
        );
        let decoded = ValidatorSetCommitteeProof::from_bytes(&served.proof).unwrap();
        decoded.verify(&tree.hash()).unwrap();
    }

    #[test]
    fn proofs_are_refused_by_an_indexer_that_does_not_verify_them() {
        assert!(require_proof_verification(true).is_ok());
        assert_eq!(
            require_proof_verification(false).unwrap_err().status,
            axum::http::StatusCode::BAD_REQUEST
        );
    }
}
