// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, hash::Hash};

use log::debug;

use crate::{
    CheckVnFunc,
    CommitProofElement,
    DecodedValidatorBlockSignature,
    FixedHash,
    ProposalVoteMessage,
    QuorumCertificate,
    QuorumDecision,
    SidechainBlockHeader,
    SidechainProofValidationError,
    UncompressedPublicKey,
};

const LOG_TARGET: &str = "c::sidechain::validations";

/// The maximum number of signatures a quorum certificate may contain. A valid quorum certificate contains at most one
/// signature per committee member, so this is an upper bound for any committee size. Proof validation is the only place
/// it is enforced, before any signature is verified or duplicates are collected.
pub const MAX_QC_SIGNATURES: usize = 1000;

/// The maximum number of elements a commit proof may contain. A proof holds the three quorum certificates of the
/// 3-chain that commits its block and at most one more element: a chain of links down to the block or, when the block
/// is the parent of the 3-chain's last block, that block's own certificate.
pub const MAX_PROOF_ELEMS: usize = 4;

/// Validates that `proof_elements` prove `header` committed on `network`, signed by a quorum of the committee that
/// `check_vn` describes. `quorum_threshold` must be positive: a QC with no signatures holds no power, so a threshold
/// of 0 would accept one.
///
/// Every check that needs no signature verification runs first, for every quorum certificate, so a proof that fails
/// one costs no signature verification.
pub fn check_proof_elements(
    network: u8,
    header: &SidechainBlockHeader,
    proof_elements: &[CommitProofElement],
    check_vn: &CheckVnFunc<'_>,
    expected_decision: QuorumDecision,
    quorum_threshold: u64,
) -> Result<(), SidechainProofValidationError> {
    if quorum_threshold == 0 {
        return Err(SidechainProofValidationError::internal_error(
            "Commit proof validation requires a positive quorum threshold, but the committee's threshold is 0",
        ));
    }
    if header.network != network {
        return Err(SidechainProofValidationError::InvalidProof {
            details: format!(
                "Commit proof is for a block on network {}, but network {network} was expected",
                header.network
            ),
        });
    }
    check_proof_structure(header, proof_elements)?;

    let mut signed_qcs = Vec::with_capacity(proof_elements.len());
    for elem in proof_elements {
        if let CommitProofElement::QuorumCertificate(qc) = elem {
            signed_qcs.push((qc, check_qc_signers(qc, quorum_threshold, check_vn, expected_decision)?));
        }
    }
    for (qc, signers) in &signed_qcs {
        debug!(target: LOG_TARGET, "Verifying quorum certificate that justifies: {}", qc.calculate_justified_block());
        verify_qc_signatures(qc, signers)?;
        debug!(target: LOG_TARGET, "Quorum certificate OK");
    }

    Ok(())
}

/// Validates that `proof_elements` form a 3-chain of quorum certificates that, followed by any chain links, descends
/// to `header`. A proof that passes proves `header` only once each of its quorum certificates is validated against a
/// committee, as [`check_proof_elements`] does.
pub fn check_proof_structure(
    header: &SidechainBlockHeader,
    proof_elements: &[CommitProofElement],
) -> Result<(), SidechainProofValidationError> {
    check_header_commits_to_its_fields(header)?;
    check_proof_elements_num_qcs(proof_elements, 3)?;

    let mut last_parent = None::<&FixedHash>;
    let mut proven_3_chain = 0usize;
    for elem in proof_elements {
        match elem {
            CommitProofElement::QuorumCertificate(qc) => {
                let justifies = qc.calculate_justified_block();
                if let Some(last_parent) = last_parent &&
                    *last_parent != justifies
                {
                    return Err(SidechainProofValidationError::InvalidProof {
                        details: format!(
                            "Parent block ID {last_parent} does not match the parent block ID {justifies} in the \
                             quorum certificate"
                        ),
                    });
                }

                if proven_3_chain < 3 {
                    proven_3_chain = proven_3_chain.saturating_add(1);
                    debug!(target: LOG_TARGET, "3-chain rule: {proven_3_chain} of 3 proven");
                }

                debug!(target: LOG_TARGET, "Setting last parent to {}", qc.parent_id);
                last_parent = Some(&qc.parent_id);
            },
            CommitProofElement::ChainLinks(chain) => {
                if proven_3_chain != 3 {
                    return Err(SidechainProofValidationError::InvalidProof {
                        details: format!(
                            "A 3-chain must be proven before a dummy chain. A chain of length {proven_3_chain} was \
                             proven"
                        ),
                    });
                }

                let Some(parent) = last_parent else {
                    return Err(SidechainProofValidationError::InvalidProof {
                        details: "Dummy chain must be preceded by a quorum certificate".to_string(),
                    });
                };
                if chain.is_empty() {
                    return Err(SidechainProofValidationError::InvalidProof {
                        details: "Dummy chain must contain at least one element".to_string(),
                    });
                }
                let mut expected = *parent;
                for (i, link) in chain.iter().enumerate() {
                    let block_id = link.calc_block_id();
                    debug!(target: LOG_TARGET, "Validating dummy chain link {i}: link {block_id} ?= last parent {expected}");
                    if block_id != expected {
                        return Err(SidechainProofValidationError::InvalidProof {
                            details: format!(
                                "Block ID in chain link at index {i} does not match the parent block ID in the quorum \
                                 certificate. Expected {expected}, but got {block_id}",
                            ),
                        });
                    }
                    expected = link.parent_id;
                    last_parent = Some(&link.parent_id);
                }
            },
        }
    }

    let Some(last_elem) = proof_elements.last() else {
        // Not reachable because we check length of proof elements above
        return Err(SidechainProofValidationError::InvalidProof {
            details: "BUG: Proof must contain at least one proof element".to_string(),
        });
    };

    let Some(last_parent) = last_parent else {
        // Not reachable because we check length of proof elements above
        return Err(SidechainProofValidationError::InvalidProof {
            details: "BUG: Proof must contain at least proof element (last_parent == None)".to_string(),
        });
    };

    let justified = match last_elem {
        CommitProofElement::QuorumCertificate(qc) => qc.calculate_justified_block(),
        CommitProofElement::ChainLinks(links) => {
            links
                .last()
                .map(|link| link.parent_id)
                // Already checked that links is not empty
                .ok_or_else(|| SidechainProofValidationError::InvalidProof {
                    details: "BUG: chain must contain at least one element".to_string(),
                })?
        },
    };

    let header_block_id = header.calculate_block_id();
    if justified != header_block_id {
        return Err(SidechainProofValidationError::InvalidProof {
            details: format!(
                "Last parent block ID does not match the block ID in the header. Expected {header_block_id}, but got \
                 {last_parent}"
            ),
        });
    }

    Ok(())
}

/// Rejects a header that carries a field its protocol version's hash preimage does not cover, since nothing
/// authenticates such a field.
fn check_header_commits_to_its_fields(header: &SidechainBlockHeader) -> Result<(), SidechainProofValidationError> {
    if header.protocol_version == 0 && header.transaction_merkle_root.is_some() {
        return Err(SidechainProofValidationError::InvalidProof {
            details: "Header claims protocol version 0, which commits to no transaction merkle root, but carries one"
                .to_string(),
        });
    }
    Ok(())
}

pub fn check_proof_elements_num_qcs(
    proof_elems: &[CommitProofElement],
    expected_len: usize,
) -> Result<(), SidechainProofValidationError> {
    if proof_elems.len() > MAX_PROOF_ELEMS {
        return Err(SidechainProofValidationError::InvalidProof {
            details: format!(
                "Commit Proof contained too many proof elements. Expected at most {} but got {}",
                MAX_PROOF_ELEMS,
                proof_elems.len()
            ),
        });
    }
    let num_qcs = proof_elems
        .iter()
        .filter(|elem| matches!(elem, CommitProofElement::QuorumCertificate(_)))
        .count();
    if num_qcs < expected_len {
        return Err(SidechainProofValidationError::InvalidProof {
            details: format!(
                "Expected at least {} QC proof elements, but got {}",
                expected_len,
                proof_elems.len()
            ),
        });
    }
    Ok(())
}

/// Checks every part of `qc` that needs no signature verification and returns its signers' decoded keys and
/// signatures, which hold at least `quorum_threshold` voting power.
fn check_qc_signers(
    qc: &QuorumCertificate,
    quorum_threshold: u64,
    check_vn: &CheckVnFunc<'_>,
    quorum_decision: QuorumDecision,
) -> Result<Vec<(UncompressedPublicKey, DecodedValidatorBlockSignature)>, SidechainProofValidationError> {
    if qc.signatures.len() > MAX_QC_SIGNATURES {
        return Err(SidechainProofValidationError::InvalidProof {
            details: format!(
                "Quorum certificate must contain at most {} signatures but contained {}",
                MAX_QC_SIGNATURES,
                qc.signatures.len()
            ),
        });
    }

    if qc.decision != quorum_decision {
        return Err(SidechainProofValidationError::InvalidProof {
            details: format!(
                "Quorum certificate decision must be {:?} but was {:?}",
                quorum_decision, qc.decision
            ),
        });
    }

    if has_duplicates(qc.signatures.iter().map(|s| &s.public_key)) {
        return Err(SidechainProofValidationError::InvalidProof {
            details: "Quorum certificate contains more than one signature from a single validator".to_string(),
        });
    }

    let mut decoded = Vec::with_capacity(qc.signatures.len());
    let mut signed_power = 0u64;
    for sig in &qc.signatures {
        let power = check_vn(&sig.public_key)?;
        if power == 0 {
            return Err(SidechainProofValidationError::InvalidProof {
                details: format!(
                    "QC was signed by public key {} that is not in the active validator set",
                    sig.public_key
                ),
            });
        }
        signed_power = signed_power.saturating_add(power);

        let Some(pair) = sig.decode() else {
            return Err(invalid_qc_signature(&qc.calculate_justified_block()));
        };
        decoded.push(pair);
    }

    // The signers are distinct and `verify_qc_signatures` accepts all of their signatures or none, so this is the power
    // of the signers whose signatures verify. It is checked first so that a QC short of quorum costs no verification.
    if signed_power < quorum_threshold {
        return Err(SidechainProofValidationError::InvalidProof {
            details: format!(
                "Quorum certificate signers hold {signed_power} voting power but at least {quorum_threshold} is \
                 required"
            ),
        });
    }

    Ok(decoded)
}

/// Verifies that every one of `signers` signed `qc`.
fn verify_qc_signatures(
    qc: &QuorumCertificate,
    signers: &[(UncompressedPublicKey, DecodedValidatorBlockSignature)],
) -> Result<(), SidechainProofValidationError> {
    let block_id = qc.calculate_justified_block();
    // Every member of a QC signs the same message.
    let message =
        ProposalVoteMessage::new(qc.protocol_version, &block_id, qc.decision, qc.epoch, qc.height).calculate_hash();
    let batch = signers
        .iter()
        .map(|(public_key, signature)| (signature, public_key, message))
        .collect::<Vec<_>>();
    if !DecodedValidatorBlockSignature::verify_batch(&batch) {
        return Err(invalid_qc_signature(&block_id));
    }
    Ok(())
}

fn invalid_qc_signature(block_id: &FixedHash) -> SidechainProofValidationError {
    SidechainProofValidationError::InvalidProof {
        details: format!("Invalid signature for QC for block ID {block_id}"),
    }
}

fn has_duplicates<I, T>(iter: I) -> bool
where
    I: IntoIterator<Item = T> + ExactSizeIterator,
    T: Eq + Hash,
{
    let mut set = HashSet::with_capacity(iter.len());
    for item in iter {
        if !set.insert(item) {
            return true;
        }
    }
    false
}
