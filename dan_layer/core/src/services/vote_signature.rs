//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::FixedHash;
use tari_core::ValidatorNodeBMT;
use tari_dan_common_types::{hasher::tari_hasher, vn_bmt_node_hash, ShardId, ValidatorMetadata};
use tari_dan_storage::models::VoteMessage;
use tari_mmr::BalancedBinaryMerkleProof;

use crate::{services::SigningService, workers::hotstuff_error::HotStuffError, TariDanCoreHashDomain};

const LOG_TARGET: &str = "tari::consensus::vote_message";

pub fn sign_vote<TSigningService: SigningService>(
    vote_msg_mut: &mut VoteMessage,
    signing_service: &TSigningService,
    shard_id: ShardId,
    vn_bmt: &ValidatorNodeBMT,
) -> Result<(), HotStuffError> {
    // calculate the signature
    let challenge = construct_challenge(vote_msg_mut);
    let signature = signing_service.sign(&*challenge).ok_or(HotStuffError::FailedToSignQc)?;
    // construct the merkle proof for the inclusion of the VN's public key in the epoch
    let node_hash = vn_bmt_node_hash(signing_service.public_key(), &shard_id);
    debug!(
        target: LOG_TARGET,
        "[sign_vote] bmt_node_hash={}, public_key={}, shard_id={}",
        node_hash,
        signing_service.public_key(),
        shard_id,
    );
    let leaf_index = vn_bmt
        .find_leaf_index_for_hash(&node_hash.to_vec())
        .map_err(|_| HotStuffError::ValidatorNodeNotIncludedInBMT)?;
    let merkle_proof = BalancedBinaryMerkleProof::generate_proof(vn_bmt, leaf_index as usize)
        .map_err(|_| HotStuffError::FailedToGenerateMerkleProof)?;

    let root = vn_bmt.get_merkle_root();
    let idx = vn_bmt
        .find_leaf_index_for_hash(&node_hash.to_vec())
        .map_err(|_| HotStuffError::ValidatorNodeNotIncludedInBMT)?;
    // TODO: remove
    if !merkle_proof.verify(&root, node_hash.to_vec()) {
        error!( target: "tari::dan::votemessage", "Merkle proof verification failed for validator node {}, shard:{} hash: {} at index {:?} leaf index {}", signing_service.public_key(), shard_id, node_hash, idx, leaf_index, );
    }

    let validator_metadata = ValidatorMetadata::new(signing_service.public_key().clone(), shard_id, signature);

    vote_msg_mut.set_validator_metadata(validator_metadata);
    vote_msg_mut.set_merkle_proof(merkle_proof);
    vote_msg_mut.set_node_hash(node_hash);
    Ok(())
}

pub fn construct_challenge(vote_msg: &VoteMessage) -> FixedHash {
    tari_hasher::<TariDanCoreHashDomain>("vote_message")
        .chain(vote_msg.local_node_hash().as_bytes())
        .chain(&[vote_msg.decision().as_u8()])
        .chain(vote_msg.all_shard_pledges())
        .result()
}
