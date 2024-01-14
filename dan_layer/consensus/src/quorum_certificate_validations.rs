// Copyright 2024. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use crate::traits::VoteSignatureService;
use tari_dan_common_types::{committee::CommitteeShard, NodeHeight, DerivableFromPublicKey};
use tari_dan_storage::consensus_models::QuorumCertificate;
use tari_epoch_manager::{EpochManagerReader, EpochManagerError};


#[derive(Debug, thiserror::Error)]
pub enum QuorumCertificateValidationError {
    #[error("QC has invalid merkle proof")]
    InvalidMerkleProof,
    #[error("QC has invalid signature")]
    InvalidSignature,
    #[error("Quorum was not reached")]
    QuorumWasNotReached,
    #[error("Malformed QC")]
    MalformedCertificate,
    #[error("Epoch manager error: {0}")]
    StorageError(#[from] EpochManagerError),
    #[error("Block {block_height} is not higher than justify {justify_block_height}")]
    BlockNotHigherThanJustify {
        justify_block_height: NodeHeight,
        block_height: NodeHeight,
    },
}

/// Validates Quorum Certificates in isolation
pub async fn validate_quorum_certificate<
    TAddr: DerivableFromPublicKey,
    TEpochManager:  EpochManagerReader<Addr = TAddr>,
    TSignatureService: VoteSignatureService>
    (qc: &QuorumCertificate, committee_shard: &CommitteeShard,
    vote_signing_service: &TSignatureService,
    epoch_manager: &TEpochManager,
) -> Result<(), QuorumCertificateValidationError> {        
    // ignore genesis block.
    if qc.epoch().as_u64() == 0 {
        return Ok(());
    }

    // fetch the committee members that should have signed the QC
    let mut vns = vec![];
    for signature in qc.signatures() {
        let vn = epoch_manager
            .get_validator_node_by_public_key(qc.epoch(), signature.public_key())
            .await?;
        vns.push(vn.node_hash());
    }

    // validate the QC's merkle proof
    let merkle_root = epoch_manager
        .get_validator_node_merkle_root(qc.epoch())
        .await?;
    let proof = qc.merged_proof().clone();
    let vns_bytes = vns.iter().map(|hash| hash.to_vec()).collect();
    let is_proof_valid = proof.verify_consume(&merkle_root, vns_bytes)
        .map_err(|_| QuorumCertificateValidationError::InvalidMerkleProof)?;
    if !is_proof_valid {
        return Err(QuorumCertificateValidationError::InvalidMerkleProof);
    }

    // validate each signature in the QC
    for (sign, leaf) in qc.signatures().iter().zip(vns.iter()) {
        let challenge = vote_signing_service.create_challenge(leaf, qc.block_id(), &qc.decision());
        if !sign.verify(challenge) {
            return Err(QuorumCertificateValidationError::InvalidSignature);
        }
    }

    // validate that enough committee members have signed the QC
    let num_signatures_in_qc = u32::try_from(qc.signatures().len())
        .map_err(|_| QuorumCertificateValidationError::MalformedCertificate)?;
    if committee_shard.quorum_threshold() > num_signatures_in_qc {
        return Err(QuorumCertificateValidationError::QuorumWasNotReached);
    }
    
    Ok(())
}
