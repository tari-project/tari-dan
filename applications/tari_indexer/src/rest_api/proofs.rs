//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_indexer_client::types::{SubstateProof, SubstateProofAnchor};
use tari_ootle_common_types::Epoch;
use tari_ootle_storage::consensus_models::CommittedBlockProof;
use tari_template_lib_types::Hash32;
use tari_validator_node_rpc::client::SubstateProofData;

use crate::rest_api::error::ErrorResponse;

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
    })
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::{CompressedPublicKey, FixedHash};
    use tari_crypto::ristretto::RistrettoSecretKey;
    use tari_ootle_common_types::ShardGroup;
    use tari_sidechain::{SidechainBlockCommitProof, SidechainBlockHeader, ValidatorBlockSignature};

    use super::*;

    fn commit_proof() -> CommittedBlockProof {
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
