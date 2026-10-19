//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use tari_engine_types::substate::SubstateValue;
use tari_sidechain::hashing::TariConsensusHasher;
pub use tari_sidechain::hashing::{
    block_hasher,
    block_metadata_hasher,
    command_hasher,
    proposal_vote_signature_hasher,
    tari_consensus_hasher,
};
use tari_template_lib_types::Hash32;

pub fn quorum_certificate_id_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("QuorumCertificateId")
}

pub fn timeout_certificate_id_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("TimeoutCertificateId")
}

/// Hashes a transaction's outcome into a leaf of a block's transaction merkle root.
pub fn finalized_transaction_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("FinalizedTransaction")
}

/// Hashes a command, with the shard groups a foreign commit proof must carry it in full for, into a leaf of a block's
/// command merkle root.
pub fn command_leaf_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("CommandLeaf")
}

/// Commits to a substate value that one shard group pledges to another. The receiving shard group recomputes it over
/// the pledged value and compares it with the hash in the sending shard group's committed evidence.
pub fn hash_pledged_substate_value(value: &SubstateValue) -> Hash32 {
    tari_consensus_hasher("PledgedSubstateValue")
        .chain(value)
        .finalize_into_array()
        .into()
}
