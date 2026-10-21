//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use blake2::{Blake2b, digest::consts::U32};
use log::*;
use ootle_byte_type::ConvertFromByteType;
use tari_common_types::types::{CompressedCommitment, CompressedPublicKey, CompressedSignature, PrivateKey};
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    ristretto::{RistrettoSchnorr, RistrettoSecretKey, pedersen::PedersenCommitment},
    tari_utilities::ByteArray,
};
use tari_engine::traits::{ClaimProofError, ClaimProofRejection, ClaimProofVerifier, VerifiedClaim};
use tari_engine_types::{
    confidential::{
        BurnOutput,
        BurnOutputInclusionProof,
        MAX_MMR_PROOF_HASHES,
        MinotariBurnClaimProof,
        MmrInclusionProof,
    },
    crypto::get_commitment_factory,
};
use tari_hashing::{TransactionHashDomain, hashers::InputMmrHasherBlake256};
use tari_mmr::common::{LeafIndex, checked_n_leaves};
use tari_ootle_common_types::{
    Epoch,
    base_layer_hashing::ownership_proof_hasher64,
    optional::{IsNotFoundError, Optional},
};
use tari_ootle_storage::global::{GlobalDb, GlobalDbAdapter};
use tari_ootle_transaction::Network;
use tari_sidechain::FixedHash;
use tari_template_lib::types::crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes};
use tari_transaction_components::{
    MicroMinotari,
    consensus::DomainSeparatedConsensusHasher,
    transaction_components::{
        CoinBaseExtra,
        ConfidentialOutputData,
        OutputFeatures,
        OutputFeaturesVersion,
        OutputType,
        RangeProofType,
        SideChainFeature,
        SideChainFeatureData,
        SideChainId,
        TransactionOutputVersion,
    },
};

const LOG_TARGET: &str = "tari::ootle::claim_burn_proof_verifier";

/// Verifies a claim in full: the burn output rules, the ownership proof, and the output's inclusion in an L1 block
/// whose header this node holds.
pub struct TariClaimBurnProofVerifier<TGlobalBackend> {
    burn_output: BurnOutputVerifier,
    knowledge_proof: KnowledgeProofVerifier,
    output_inclusion: OutputInclusionVerifier<TGlobalBackend>,
}

impl<TGlobalBackend> TariClaimBurnProofVerifier<TGlobalBackend> {
    pub fn new(
        network: Network,
        sidechain_id: Option<RistrettoPublicKeyBytes>,
        global_db: GlobalDb<TGlobalBackend>,
    ) -> Self {
        Self {
            burn_output: BurnOutputVerifier::new(network, sidechain_id),
            knowledge_proof: KnowledgeProofVerifier::new(network, sidechain_id),
            output_inclusion: OutputInclusionVerifier::new(global_db),
        }
    }
}

impl<TGlobalBackend> ClaimProofVerifier for TariClaimBurnProofVerifier<TGlobalBackend>
where
    TGlobalBackend: GlobalDbAdapter,
    TGlobalBackend::Error: Display + IsNotFoundError,
{
    fn verify_claim_proof(
        &self,
        epoch: Epoch,
        claim_proof: &MinotariBurnClaimProof,
    ) -> Result<VerifiedClaim, ClaimProofError> {
        let output_hash = self.burn_output.verify(claim_proof)?;
        self.knowledge_proof
            .verify(&claim_proof.output.features.claim_public_key, claim_proof)?;
        self.output_inclusion
            .verify(epoch, &claim_proof.inclusion_proof, &output_hash)?;
        Ok(VerifiedClaim {
            claim_public_key: claim_proof.output.features.claim_public_key,
        })
    }
}

/// Verifies everything about a claim that needs no L1 header: the burn output rules and the ownership proof. It takes
/// the output's inclusion in an L1 block on trust, which suits nodes that only estimate claims, such as an indexer.
pub struct HeaderlessClaimBurnProofVerifier {
    burn_output: BurnOutputVerifier,
    knowledge_proof: KnowledgeProofVerifier,
}

impl HeaderlessClaimBurnProofVerifier {
    pub fn new(network: Network, sidechain_id: Option<RistrettoPublicKeyBytes>) -> Self {
        Self {
            burn_output: BurnOutputVerifier::new(network, sidechain_id),
            knowledge_proof: KnowledgeProofVerifier::new(network, sidechain_id),
        }
    }
}

impl ClaimProofVerifier for HeaderlessClaimBurnProofVerifier {
    fn verify_claim_proof(
        &self,
        _epoch: Epoch,
        claim_proof: &MinotariBurnClaimProof,
    ) -> Result<VerifiedClaim, ClaimProofError> {
        self.burn_output.verify(claim_proof)?;
        self.knowledge_proof
            .verify(&claim_proof.output.features.claim_public_key, claim_proof)?;
        Ok(VerifiedClaim {
            claim_public_key: claim_proof.output.features.claim_public_key,
        })
    }
}

/// Checks that a claim's burn output is one this chain mints against, and computes its L1 output hash.
///
/// The claim can only describe a `Burn` output whose sidechain feature is a `ConfidentialOutput`: the output hash is
/// computed with those fixed, so a claim of any other output does not prove inclusion. What is left to check is that
/// the output is tagged for this chain.
pub struct BurnOutputVerifier {
    network: Network,
    sidechain_id: Option<RistrettoPublicKeyBytes>,
}

impl BurnOutputVerifier {
    pub fn new(network: Network, sidechain_id: Option<RistrettoPublicKeyBytes>) -> Self {
        Self { network, sidechain_id }
    }

    /// Returns the L1 hash of the claim's burn output
    pub fn verify(&self, claim: &MinotariBurnClaimProof) -> Result<FixedHash, ClaimProofError> {
        let output_sidechain_id = claim.output.features.sidechain_id.as_ref().map(|id| &id.public_key);
        if output_sidechain_id != self.sidechain_id.as_ref() {
            warn!(
                target: LOG_TARGET,
                "Claim burn failed - burn output is tagged for sidechain {:?}, this chain is {:?}",
                output_sidechain_id, self.sidechain_id
            );
            return Err(ClaimProofRejection::Invalid(format!(
                "the burn output is tagged for sidechain {}, but this chain is {}",
                display_sidechain_id(output_sidechain_id),
                display_sidechain_id(self.sidechain_id.as_ref()),
            ))
            .into());
        }

        hash_burn_output(self.network, claim).map_err(|e| {
            warn!(target: LOG_TARGET, "Claim burn failed - malformed burn output: {}", e);
            ClaimProofRejection::Invalid(format!("malformed burn output: {}", e)).into()
        })
    }
}

fn display_sidechain_id(id: Option<&RistrettoPublicKeyBytes>) -> String {
    id.map_or_else(|| "none".to_string(), |id| id.to_string())
}

/// Computes the L1 `TransactionOutput::hash` of a claim's burn output.
///
/// The fields a claim relies on (the features, the commitment and the sender offset public key) are built into L1
/// types and hashed through their own consensus encoding. The opaque fields are consensus encodings already and are
/// hashed as given. The L1 hash is the concatenation of each field's encoding, so a field is pinned to its place in
/// the output only if every encoding ahead of it is self-delimiting. The typed encodings are, and the script's
/// declared length must match its bytes, which pins the sender offset public key that follows it.
fn hash_burn_output(network: Network, claim: &MinotariBurnClaimProof) -> Result<FixedHash, String> {
    let BurnOutput {
        version,
        features,
        rangeproof_hash,
        script,
        sender_offset_public_key,
        metadata_signature,
        covenant,
        encrypted_data,
        minimum_value_promise,
    } = &claim.output;

    check_script_length(script.as_slice())?;
    let version = TransactionOutputVersion::try_from(*version).map_err(|e| format!("bad output version: {}", e))?;
    let range_proof_type = match features.range_proof_type {
        0 => RangeProofType::BulletProofPlus,
        1 => RangeProofType::RevealedValue,
        other => return Err(format!("bad range proof type {}", other)),
    };
    let sidechain_id = features
        .sidechain_id
        .as_ref()
        .map(|id| {
            Ok::<_, String>(SideChainId::new(
                compressed_public_key("sidechain id", &id.public_key)?,
                compressed_signature("sidechain id knowledge proof", &id.knowledge_proof)?,
            ))
        })
        .transpose()?;
    let features = OutputFeatures {
        version: OutputFeaturesVersion::try_from(features.version)
            .map_err(|e| format!("bad output features version: {}", e))?,
        output_type: OutputType::Burn,
        maturity: features.maturity,
        coinbase_extra: CoinBaseExtra::default(),
        sidechain_feature: Some(SideChainFeature {
            data: SideChainFeatureData::ConfidentialOutput(ConfidentialOutputData {
                claim_public_key: compressed_public_key("claim public key", &features.claim_public_key)?,
            }),
            sidechain_id,
        }),
        range_proof_type,
    };
    let commitment = CompressedCommitment::from_canonical_bytes(claim.commitment.as_bytes())
        .map_err(|e| format!("bad commitment: {}", e))?;

    let hash = DomainSeparatedConsensusHasher::<TransactionHashDomain, Blake2b<U32>>::new_with_network(
        "transaction_output",
        network.as_byte(),
    )
    .chain(&version)
    .chain(&features)
    .chain(&commitment)
    .chain(&FixedHash::from(rangeproof_hash.into_array()))
    .chain(&Encoded(script.as_slice()))
    .chain(&compressed_public_key(
        "sender offset public key",
        sender_offset_public_key,
    )?)
    .chain(&Encoded(metadata_signature.as_slice()))
    .chain(&Encoded(covenant.as_slice()))
    .chain(&Encoded(encrypted_data.as_slice()))
    .chain(&MicroMinotari::from(*minimum_value_promise))
    .finalize();
    Ok(hash.into())
}

/// Checks that a consensus encoded script is its varint length prefix followed by exactly that many bytes
fn check_script_length(script: &[u8]) -> Result<(), String> {
    let mut declared = 0u64;
    for (i, byte) in script.iter().enumerate().take(10) {
        declared |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            let body = script.len() - (i + 1);
            return if u64::try_from(body).ok() == Some(declared) {
                Ok(())
            } else {
                Err(format!("the script declares {} bytes but carries {}", declared, body))
            };
        }
    }
    Err("the script has no valid length prefix".to_string())
}

/// Bytes that are already a consensus encoding, written to a consensus hasher as they are
struct Encoded<'a>(&'a [u8]);

impl borsh::BorshSerialize for Encoded<'_> {
    fn serialize<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        writer.write_all(self.0)
    }
}

fn compressed_public_key(field: &str, key: &RistrettoPublicKeyBytes) -> Result<CompressedPublicKey, String> {
    CompressedPublicKey::from_canonical_bytes(key.as_bytes()).map_err(|e| format!("bad {}: {}", field, e))
}

fn compressed_signature(field: &str, sig: &SchnorrSignatureBytes) -> Result<CompressedSignature, String> {
    Ok(CompressedSignature::new(
        CompressedPublicKey::from_canonical_bytes(sig.public_nonce().as_bytes())
            .map_err(|e| format!("bad {} nonce: {}", field, e))?,
        PrivateKey::from_canonical_bytes(sig.signature().as_bytes()).map_err(|e| format!("bad {}: {}", field, e))?,
    ))
}

/// Verifies that an output hash is in an L1 block whose header this node holds, against the header's
/// `block_output_mr`.
pub struct OutputInclusionVerifier<TGlobalBackend> {
    global_db: GlobalDb<TGlobalBackend>,
}

impl<TGlobalBackend> OutputInclusionVerifier<TGlobalBackend> {
    pub fn new(global_db: GlobalDb<TGlobalBackend>) -> Self {
        Self { global_db }
    }
}

impl<TGlobalBackend> OutputInclusionVerifier<TGlobalBackend>
where
    TGlobalBackend: GlobalDbAdapter,
    TGlobalBackend::Error: Display + IsNotFoundError,
{
    pub fn verify(
        &self,
        epoch: Epoch,
        proof: &BurnOutputInclusionProof,
        output_hash: &FixedHash,
    ) -> Result<(), ClaimProofError> {
        // Only headers from epochs before the current one count: a node that has reached an epoch holds every header
        // of the epochs before it from its configured scan `start_height` on, but each node scans the current
        // epoch's headers at its own pace, so they would disagree on whether one is present.
        let Some(max_header_epoch) = epoch.checked_sub(1) else {
            warn!(target: LOG_TARGET, "Claim burn failed - no base layer header is claimable in epoch 0");
            return Err(ClaimProofRejection::NotYetValid(
                "no base layer header is claimable in epoch 0. The burn is claimable in a later epoch.".to_string(),
            )
            .into());
        };
        let block_header = {
            let mut tx = self.global_db.create_transaction().map_err(|e| {
                warn!(target: LOG_TARGET, "Claim burn failed - could not create DB transaction: {}", e);
                ClaimProofError::VerifierFault(format!("could not create DB transaction: {}", e))
            })?;
            self.global_db
                .block_headers(&mut tx)
                .get_by_hash(max_header_epoch, &proof.block_hash)
                .optional()
                .map_err(|e| {
                    warn!(target: LOG_TARGET, "Claim burn failed - could not fetch block header: {}", e);
                    ClaimProofError::VerifierFault(format!("could not fetch block header: {}", e))
                })?
        };
        let block_header = block_header.ok_or_else(|| {
            warn!(
                target: LOG_TARGET,
                "Claim burn failed - block header not found for hash {} in an epoch before {}",
                proof.block_hash, epoch
            );
            // A header from this epoch, or one not yet synced, is claimable in a later epoch, so the same proof
            // can still verify. This is the only failure here that depends on when it is checked.
            ClaimProofRejection::NotYetValid(format!(
                "block header not found for hash {} in an epoch before {}. The claim may be invalid, or the burn may \
                 only be claimable in a later epoch.",
                proof.block_hash, epoch
            ))
        })?;

        verify_output_inclusion(proof, output_hash, &block_header.block_output_merkle_root).map_err(|e| {
            warn!(target: LOG_TARGET, "Claim burn failed - invalid output inclusion proof: {}", e);
            ClaimProofRejection::Invalid(format!("invalid output inclusion proof: {}", e)).into()
        })
    }
}

/// Folds `output_hash` up to `block_output_mr`: first to the normal output MMR root, then from that root, which must
/// be the last leaf of the block output MMR, to the block output MMR root.
fn verify_output_inclusion(
    proof: &BurnOutputInclusionProof,
    output_hash: &FixedHash,
    block_output_mr: &FixedHash,
) -> Result<(), String> {
    let normal_output_mr = FixedHash::from(proof.normal_output_mr.into_array());
    verify_mmr_proof(
        "normal output",
        &proof.normal_output_proof,
        &normal_output_mr,
        output_hash,
    )?;

    let block_output_proof = &proof.block_output_proof;
    let last_leaf = usize::try_from(block_output_proof.mmr_size)
        .ok()
        .and_then(checked_n_leaves)
        .and_then(|n_leaves| n_leaves.checked_sub(1))
        .ok_or_else(|| format!("invalid block output MMR size {}", block_output_proof.mmr_size))?;
    if u64::try_from(last_leaf).ok() != Some(block_output_proof.leaf_index) {
        return Err(format!(
            "the normal output MMR root is at leaf {} of the block output MMR, not its last leaf {}",
            block_output_proof.leaf_index, last_leaf
        ));
    }
    verify_mmr_proof("block output", block_output_proof, block_output_mr, &normal_output_mr)
}

fn verify_mmr_proof(mmr: &str, proof: &MmrInclusionProof, root: &FixedHash, leaf: &FixedHash) -> Result<(), String> {
    if proof.path.len() > MAX_MMR_PROOF_HASHES || proof.peaks.len() > MAX_MMR_PROOF_HASHES {
        return Err(format!("the {} MMR proof has too many hashes", mmr));
    }
    let to_usize = |value: u64, name: &str| {
        usize::try_from(value).map_err(|_| format!("the {} MMR proof {} {} is out of range", mmr, name, value))
    };
    let merkle_proof = tari_mmr::MerkleProof {
        mmr_size: to_usize(proof.mmr_size, "size")?,
        path: proof.path.iter().map(|hash| hash.as_slice().to_vec()).collect(),
        peaks: proof.peaks.iter().map(|hash| hash.as_slice().to_vec()).collect(),
    };
    merkle_proof
        .verify_leaf::<InputMmrHasherBlake256>(
            root.as_slice(),
            leaf.as_slice(),
            LeafIndex(to_usize(proof.leaf_index, "leaf index")?),
        )
        .map_err(|e| format!("the {} MMR proof does not verify: {}", mmr, e))
}

pub struct KnowledgeProofVerifier {
    network: Network,
    /// This chain's own burnt-utxo sidechain id (the L1 deployment key's public key), bound into
    /// the ownership-proof challenge so a proof signed for another sidechain cannot be replayed
    /// here. `None` for the default chain that has no deployment key. See tari-ootle#445.
    sidechain_id: Option<RistrettoPublicKeyBytes>,
}

impl KnowledgeProofVerifier {
    pub fn new(network: Network, sidechain_id: Option<RistrettoPublicKeyBytes>) -> Self {
        Self { network, sidechain_id }
    }

    pub fn verify(
        &self,
        claimant: &RistrettoPublicKeyBytes,
        claim: &MinotariBurnClaimProof,
    ) -> Result<(), ClaimProofError> {
        let MinotariBurnClaimProof {
            commitment,
            ownership_proof: proof_of_knowledge,
            value,
            ..
        } = claim;

        // `claimant` is the stealth claim public key `S = H(r·P)·G + P` that the burn output names. The L1 wallet
        // binds the proof to it, and the engine requires `S` to sign the claim transaction.
        //
        // `sidechain_id` binds the proof to THIS chain's configured burnt-utxo sidechain id, so a
        // proof signed for another sidechain/application cannot be replayed here (tari-ootle#445).
        // It is the verifier's own identity, never taken from the (attacker-supplied) proof. The
        // `Option<&[u8]>` encoding mirrors the L1 signer's `Option<CompressedPublicKey>` borsh.
        // NOTE: .as_bytes() used because the tari_crypto borsh implementations serialize fixed length bytes as variable
        // length bytes of size 32
        let sidechain_id = self.sidechain_id.as_ref().map(|id| id.as_bytes());
        let message = ownership_proof_hasher64(self.network)
            .chain(&commitment.as_bytes())
            .chain(&claimant.as_bytes())
            .chain(&sidechain_id)
            .finalize();

        let commitment = PedersenCommitment::convert_from_byte_type(commitment).map_err(|e| {
            warn!(target: LOG_TARGET, "Claim burn failed - malformed commitment: {}", e);
            format!("malformed commitment: {}", e)
        })?;

        let proof_of_knowledge = RistrettoSchnorr::convert_from_byte_type(proof_of_knowledge).map_err(|e| {
            warn!(target: LOG_TARGET, "Claim burn failed - malformed proof of knowledge: {}", e);
            format!("malformed proof of knowledge: {}", e)
        })?;

        let value_commit = get_commitment_factory().commit_value(&RistrettoSecretKey::default(), *value);
        // k.G = C - v.H
        let signer_pk = commitment.as_public_key() - value_commit.as_public_key();

        if !proof_of_knowledge.verify(&signer_pk, message) {
            warn!(target: LOG_TARGET, "Claim burn failed - signature verification failed");
            return Err(ClaimProofRejection::Invalid("invalid proof of knowledge signature".to_string()).into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{
        commitment::HomomorphicCommitmentFactory,
        keys::{PublicKey as _, SecretKey as _},
        ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
    };
    use tari_engine_types::{
        confidential::{
            BurnOutput,
            BurnOutputFeatures,
            BurnOutputInclusionProof,
            MinotariBurnClaimProof,
            MmrInclusionProof,
        },
        crypto::get_commitment_factory,
    };
    use tari_ootle_common_types::base_layer_hashing::ownership_proof_hasher64;
    use tari_ootle_transaction::Network;
    use tari_template_lib::types::{
        Hash32,
        crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
    };

    use super::KnowledgeProofVerifier;

    fn one_byte<const N: usize>() -> bounded_vec::BoundedVec<u8, 1, N> {
        bounded_vec::BoundedVec::<u8, 1, N>::from_vec(vec![0]).expect("valid bounded vec")
    }

    /// Mints a `MinotariBurnClaimProof` whose `ownership_proof` Schnorr signature is bound to
    /// `claimant_pk` (the key the message commits to in `H(commitment ‖ claimant_pk ‖ sidechain_id)`)
    /// and to the target `sidechain_id`.
    fn build_proof(
        network: Network,
        value: u64,
        claimant_pk: RistrettoPublicKeyBytes,
        sidechain_id: Option<&RistrettoPublicKeyBytes>,
    ) -> MinotariBurnClaimProof {
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let commitment = get_commitment_factory().commit_value(&mask, value);
        let commitment_bytes = commitment.to_byte_type();

        let sidechain_id = sidechain_id.map(|id| id.as_bytes());
        let message = ownership_proof_hasher64(network)
            .chain(&commitment_bytes.as_bytes())
            .chain(&claimant_pk.as_bytes())
            .chain(&sidechain_id)
            .finalize();
        let signature = RistrettoSchnorr::sign(&mask, &message[..], &mut rand::rng()).expect("sign with random nonce");

        // Dummy burn output and inclusion proof: KnowledgeProofVerifier doesn't read them.
        let mmr_proof = || MmrInclusionProof {
            leaf_index: 0,
            mmr_size: 1,
            path: vec![],
            peaks: vec![],
        };
        MinotariBurnClaimProof {
            commitment: commitment_bytes,
            ownership_proof: signature.to_byte_type(),
            value,
            output: BurnOutput {
                version: 0,
                features: BurnOutputFeatures {
                    version: 0,
                    maturity: 0,
                    claim_public_key: RistrettoPublicKeyBytes::zero(),
                    sidechain_id: None,
                    range_proof_type: 0,
                },
                rangeproof_hash: Hash32::zero(),
                script: one_byte(),
                sender_offset_public_key: RistrettoPublicKeyBytes::zero(),
                metadata_signature: one_byte(),
                covenant: one_byte(),
                encrypted_data: one_byte(),
                minimum_value_promise: 0,
            },
            inclusion_proof: BurnOutputInclusionProof {
                block_hash: Hash32::zero(),
                normal_output_proof: mmr_proof(),
                normal_output_mr: Hash32::zero(),
                block_output_proof: mmr_proof(),
            },
        }
    }

    #[test]
    fn verifies_against_the_claimant_it_is_bound_to() {
        let network = Network::LocalNet;
        let (_c_sec, c_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());

        let proof = build_proof(network, 2_000, c_pub.to_byte_type(), None);
        let verifier = KnowledgeProofVerifier::new(network, None);

        verifier
            .verify(&c_pub.to_byte_type(), &proof)
            .expect("proof should verify against the key it is bound to");
    }

    #[test]
    fn rejects_when_claimant_does_not_match_signed_binding() {
        let network = Network::LocalNet;
        let (_c_sec, c_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (_wrong_sec, wrong_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());

        // Signed against c_pub but the runtime passes wrong_pub.
        let proof = build_proof(network, 3_000, c_pub.to_byte_type(), None);

        let verifier = KnowledgeProofVerifier::new(network, None);
        let result = verifier.verify(&wrong_pub.to_byte_type(), &proof);
        assert!(result.is_err(), "expected verification to reject mismatched claimant");
    }

    #[test]
    fn known_answer_challenge_matches_l1_encoding() {
        // Cross-repo encoding guard. This challenge MUST byte-match the L1 signer
        // (tari's `commitment_signature` ConfidentialOutputHasher) for identical inputs, or
        // burn-claim ownership proofs will silently fail to verify. The expected hash below was
        // produced by an equivalent known-answer test in the tari repo. Inputs: commitment =
        // claimant = Ristretto basepoint, sidechain_id = None, network byte 0x26 (Esmeralda).
        const BP: [u8; 32] = [
            0xe2, 0xf2, 0xae, 0x0a, 0x6a, 0xbc, 0x4e, 0x71, 0xa8, 0x84, 0xa9, 0x61, 0xc5, 0x00, 0x51, 0x5f, 0x58, 0xe3,
            0x0b, 0x6a, 0xa5, 0x82, 0xdd, 0x8d, 0xb6, 0xa6, 0x59, 0x45, 0xe0, 0x8d, 0x2d, 0x76,
        ];
        let commitment = PedersenCommitmentBytes::from(BP);
        let claimant = RistrettoPublicKeyBytes::from_bytes(&BP).unwrap();
        let sidechain_id: Option<RistrettoPublicKeyBytes> = None;
        let sc = sidechain_id.as_ref().map(|id| id.as_bytes());
        let challenge = ownership_proof_hasher64(Network::Esmeralda)
            .chain(&commitment.as_bytes())
            .chain(&claimant.as_bytes())
            .chain(&sc)
            .finalize();
        let hex: String = challenge.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex,
            "cd025aa3c5331a92927850d9fd5ac3581419b7da8b6ef42dda57fcad07d49b76e0629ccad07ad85568a92e4cec5560fe380c60db2800b9d2407fb68fa3a892a7",
            "burn-claim ownership-proof challenge encoding drifted from the L1 signer"
        );
    }

    #[test]
    fn verifies_with_matching_sidechain_id() {
        let network = Network::LocalNet;
        let (_c_sec, c_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (_sc_sec, sc_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let sidechain_id = sc_pub.to_byte_type();

        let proof = build_proof(network, 4_000, c_pub.to_byte_type(), Some(&sidechain_id));
        let verifier = KnowledgeProofVerifier::new(network, Some(sidechain_id));

        verifier
            .verify(&c_pub.to_byte_type(), &proof)
            .expect("proof bound to this chain's sidechain id should verify");
    }

    #[test]
    fn rejects_replay_onto_a_different_sidechain() {
        let network = Network::LocalNet;
        let (_c_sec, c_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (_signed_sec, signed_sc) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (_other_sec, other_sc) = RistrettoPublicKey::random_keypair(&mut rand::rng());

        // Proof signed for `signed_sc`, but this chain's verifier is configured with `other_sc`.
        let proof = build_proof(network, 5_000, c_pub.to_byte_type(), Some(&signed_sc.to_byte_type()));
        let verifier = KnowledgeProofVerifier::new(network, Some(other_sc.to_byte_type()));

        let result = verifier.verify(&c_pub.to_byte_type(), &proof);
        assert!(result.is_err(), "replay onto a different sidechain must be rejected");
    }

    #[test]
    fn rejects_unbound_proof_when_chain_expects_a_sidechain_id() {
        let network = Network::LocalNet;
        let (_c_sec, c_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let (_sc_sec, sc_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());

        // Proof carries no sidechain binding (None) but this chain expects one: the `Option` tag
        // alone must change the challenge.
        let proof = build_proof(network, 6_000, c_pub.to_byte_type(), None);
        let verifier = KnowledgeProofVerifier::new(network, Some(sc_pub.to_byte_type()));

        let result = verifier.verify(&c_pub.to_byte_type(), &proof);
        assert!(
            result.is_err(),
            "an unbound proof must not verify on a chain that expects a sidechain id"
        );
    }
}

#[cfg(test)]
mod burn_output_proof_tests {
    use ootle_byte_type::ToByteType;
    use tari_common_types::{
        burn_proof::{BurnOutputProof, MmrInclusionProof as L1MmrInclusionProof, OutputHashPreimage},
        epoch::VnEpoch,
        types::{ComAndPubSignature, CompressedCommitment, CompressedPublicKey, PrivateKey},
    };
    use tari_crypto::{
        commitment::HomomorphicCommitmentFactory,
        keys::{PublicKey as _, SecretKey as _},
        ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
        tari_utilities::ByteArray,
    };
    use tari_engine::traits::{ClaimProofError, ClaimProofRejection, ClaimProofVerifier};
    use tari_engine_types::{
        confidential::{
            BurnOutput,
            BurnOutputFeatures,
            BurnOutputInclusionProof,
            BurnSidechainId,
            MAX_MMR_PROOF_HASHES,
            MinotariBurnClaimProof,
            MmrInclusionProof,
        },
        crypto::get_commitment_factory,
    };
    use tari_hashing::hashers::InputMmrHasherBlake256;
    use tari_mmr::{Hash, MerkleMountainRange, MerkleProof, common::LeafIndex};
    use tari_ootle_common_types::{Epoch, base_layer_hashing::ownership_proof_hasher64};
    use tari_ootle_storage::global::{BlockHeaderModel, DbFactory, GlobalDb};
    use tari_ootle_storage_sqlite::{SqliteDbFactory, global::SqliteGlobalDbAdapter};
    use tari_ootle_transaction::Network;
    use tari_script::script;
    use tari_sidechain::FixedHash;
    use tari_template_lib::types::{
        Hash32,
        crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
    };
    use tari_transaction_components::{
        MicroMinotari,
        transaction_components::{
            ConfidentialOutputData,
            EncryptedData,
            OutputFeatures,
            OutputType,
            RangeProofType,
            SideChainFeature,
            SideChainFeatureData,
            SideChainId,
            TransactionOutput,
            TransactionOutputVersion,
            ValidatorNodeExit,
            burn_output_proof::BurnOutputProofExt,
            covenants::Covenant,
        },
    };
    use tempfile::TempDir;

    use super::{
        HeaderlessClaimBurnProofVerifier,
        TariClaimBurnProofVerifier,
        compressed_signature,
        hash_burn_output,
        verify_output_inclusion,
    };
    use crate::burn_claim_proof::{burn_output_proof_from_l1, claim_proof_from_l1};

    type Adapter = SqliteGlobalDbAdapter<RistrettoPublicKeyBytes>;
    type OutputMmr = MerkleMountainRange<InputMmrHasherBlake256, Vec<Hash>>;

    const VALUE: u64 = 1_000;

    /// The L1 hashes outputs under its process-wide network, so the tests hash under the same one
    fn network() -> Network {
        let l1_network = tari_common::configuration::Network::get_current_or_user_setting_or_default();
        Network::try_from(l1_network.as_byte()).unwrap()
    }

    fn open_db(dir: &TempDir, migrate: bool) -> GlobalDb<Adapter> {
        let factory = SqliteDbFactory::<RistrettoPublicKeyBytes>::new(dir.path().join("global.sqlite"));
        if migrate {
            factory.migrate().unwrap();
        }
        factory.get_or_create_global_db().unwrap()
    }

    fn random_public_key() -> CompressedPublicKey {
        CompressedPublicKey::new_from_pk(RistrettoPublicKey::random_keypair(&mut rand::rng()).1)
    }

    fn sidechain_id(secret: &PrivateKey, claim_public_key: &CompressedPublicKey) -> SideChainId {
        let signature = RistrettoSchnorr::sign(secret, claim_public_key.as_bytes(), &mut rand::rng()).unwrap();
        SideChainId::new(
            CompressedPublicKey::new_from_pk(RistrettoPublicKey::from_secret_key(secret)),
            tari_common_types::types::CompressedSignature::new(
                CompressedPublicKey::new_from_pk(signature.get_public_nonce().clone()),
                signature.get_signature().clone(),
            ),
        )
    }

    fn confidential_output_feature(sidechain_id: Option<SideChainId>) -> Option<SideChainFeature> {
        Some(SideChainFeature {
            data: SideChainFeatureData::ConfidentialOutput(ConfidentialOutputData {
                claim_public_key: random_public_key(),
            }),
            sidechain_id,
        })
    }

    struct L1Burn {
        output: TransactionOutput,
        mask: RistrettoSecretKey,
    }

    fn l1_output(output_type: OutputType, sidechain_feature: Option<SideChainFeature>) -> L1Burn {
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let commitment = get_commitment_factory().commit_value(&mask, VALUE);
        let output = TransactionOutput::new(
            TransactionOutputVersion::V1,
            OutputFeatures {
                output_type,
                maturity: 7,
                sidechain_feature,
                range_proof_type: RangeProofType::RevealedValue,
                ..Default::default()
            },
            CompressedCommitment::from_canonical_bytes(commitment.as_bytes()).unwrap(),
            None,
            script!(PushPubKey(Box::new(random_public_key()))).unwrap(),
            random_public_key(),
            ComAndPubSignature::default(),
            Covenant::default(),
            EncryptedData::default(),
            MicroMinotari::from(VALUE),
        );
        L1Burn { output, mask }
    }

    fn l1_burn(sidechain_id: Option<SideChainId>) -> L1Burn {
        l1_output(OutputType::Burn, confidential_output_feature(sidechain_id))
    }

    /// The claim's view of an L1 output. The claim cannot express a non-burn output or a sidechain feature other
    /// than `ConfidentialOutput`, so for those it carries a random claim key.
    fn burn_output(output: &TransactionOutput) -> BurnOutput {
        let features = &output.features;
        let sidechain_feature = features.sidechain_feature.as_ref();
        let claim_public_key = match sidechain_feature.map(|f| &f.data) {
            Some(SideChainFeatureData::ConfidentialOutput(data)) => data.claim_public_key.clone(),
            _ => random_public_key(),
        };
        BurnOutput {
            version: output.version.as_u8(),
            features: BurnOutputFeatures {
                version: features.version.as_u8(),
                maturity: features.maturity,
                claim_public_key: RistrettoPublicKeyBytes::from_bytes(claim_public_key.as_bytes()).unwrap(),
                sidechain_id: sidechain_feature
                    .and_then(|f| f.sidechain_id.as_ref())
                    .map(|id| BurnSidechainId {
                        public_key: RistrettoPublicKeyBytes::from_bytes(id.public_key().as_bytes()).unwrap(),
                        knowledge_proof: tari_template_lib::types::crypto::SchnorrSignatureBytes::from_bytes(
                            &[
                                id.knowledge_proof().get_compressed_public_nonce().as_bytes(),
                                id.knowledge_proof().get_signature().as_bytes(),
                            ]
                            .concat(),
                        )
                        .unwrap(),
                    }),
                range_proof_type: features.range_proof_type.as_byte(),
            },
            rangeproof_hash: Hash32::zero(),
            script: bytes(borsh::to_vec(&output.script).unwrap()),
            sender_offset_public_key: RistrettoPublicKeyBytes::from_bytes(output.sender_offset_public_key.as_bytes())
                .unwrap(),
            metadata_signature: bytes(borsh::to_vec(&output.metadata_signature).unwrap()),
            covenant: bytes(borsh::to_vec(&output.covenant).unwrap()),
            encrypted_data: bytes(borsh::to_vec(&output.encrypted_data).unwrap()),
            minimum_value_promise: output.minimum_value_promise.as_u64(),
        }
    }

    fn bytes<const N: usize>(v: Vec<u8>) -> bounded_vec::BoundedVec<u8, 1, N> {
        bounded_vec::BoundedVec::<u8, 1, N>::from_vec(v).unwrap()
    }

    fn mmr_proof(mmr: &OutputMmr, leaf_index: usize) -> MmrInclusionProof {
        let proof = MerkleProof::for_leaf_node(mmr, LeafIndex(leaf_index)).unwrap();
        let hashes = |hashes: Vec<Hash>| {
            hashes
                .into_iter()
                .map(|h| Hash32::from_array(h.try_into().unwrap()))
                .collect()
        };
        MmrInclusionProof {
            leaf_index: leaf_index as u64,
            mmr_size: proof.mmr_size as u64,
            path: hashes(proof.path),
            peaks: hashes(proof.peaks),
        }
    }

    fn filler_hash(tag: u8, i: usize) -> Vec<u8> {
        let mut hash = [tag; 32];
        hash[..8].copy_from_slice(&(i as u64).to_le_bytes());
        hash.to_vec()
    }

    /// Builds the block output MMRs the way the L1 `calculate_mmr_roots` does: coinbase output hashes are leaves of
    /// the block output MMR; every other output hash is a leaf of the normal output MMR, whose root is then pushed as
    /// the last leaf of the block output MMR. Returns the inclusion proof of `output` at `index` among `num_normal`
    /// normal outputs, and the block output MR.
    fn block_with(
        output: &TransactionOutput,
        num_coinbases: usize,
        num_normal: usize,
        index: usize,
    ) -> (BurnOutputInclusionProof, FixedHash) {
        let mut block_output_mmr = OutputMmr::new(Vec::new());
        let mut normal_output_mmr = OutputMmr::new(Vec::new());
        for i in 0..num_coinbases {
            block_output_mmr.push(filler_hash(0xc0, i)).unwrap();
        }
        for i in 0..num_normal {
            let hash = if i == index {
                output.hash().to_vec()
            } else {
                filler_hash(0x80, i)
            };
            normal_output_mmr.push(hash).unwrap();
        }
        let normal_output_mr = normal_output_mmr.get_merkle_root().unwrap();
        block_output_mmr.push(normal_output_mr.clone()).unwrap();
        let block_output_mr = FixedHash::try_from(block_output_mmr.get_merkle_root().unwrap()).unwrap();

        let proof = BurnOutputInclusionProof {
            block_hash: Hash32::from_array(rand::random()),
            normal_output_proof: mmr_proof(&normal_output_mmr, index),
            normal_output_mr: Hash32::from_array(normal_output_mr.try_into().unwrap()),
            block_output_proof: mmr_proof(&block_output_mmr, num_coinbases),
        };
        (proof, block_output_mr)
    }

    /// A claim of `burn` on the chain with `chain_sidechain_id`, with an ownership proof bound to the output's claim
    /// key
    fn claim_of(
        burn: &L1Burn,
        inclusion_proof: BurnOutputInclusionProof,
        chain_sidechain_id: Option<&RistrettoPublicKeyBytes>,
    ) -> MinotariBurnClaimProof {
        let output = burn_output(&burn.output);
        let claimant = output.features.claim_public_key;
        claim_with(burn, output, inclusion_proof, &claimant, chain_sidechain_id)
    }

    /// A claim of `burn` as `output`, with an ownership proof bound to `claimant`
    fn claim_with(
        burn: &L1Burn,
        output: BurnOutput,
        inclusion_proof: BurnOutputInclusionProof,
        claimant: &RistrettoPublicKeyBytes,
        chain_sidechain_id: Option<&RistrettoPublicKeyBytes>,
    ) -> MinotariBurnClaimProof {
        let commitment =
            PedersenCommitmentBytes::from(<[u8; 32]>::try_from(burn.output.commitment.as_bytes()).unwrap());
        let sidechain_id = chain_sidechain_id.map(|id| id.as_bytes());
        let message = ownership_proof_hasher64(network())
            .chain(&commitment.as_bytes())
            .chain(&claimant.as_bytes())
            .chain(&sidechain_id)
            .finalize();
        let ownership_proof = RistrettoSchnorr::sign(&burn.mask, &message[..], &mut rand::rng()).unwrap();
        MinotariBurnClaimProof {
            commitment,
            ownership_proof: ownership_proof.to_byte_type(),
            value: VALUE,
            output,
            inclusion_proof,
        }
    }

    struct Chain {
        _dir: TempDir,
        verifier: TariClaimBurnProofVerifier<Adapter>,
        sidechain_id: Option<RistrettoPublicKeyBytes>,
    }

    impl Chain {
        fn new(sidechain_id: Option<RistrettoPublicKeyBytes>) -> Self {
            let dir = TempDir::new().unwrap();
            let verifier = TariClaimBurnProofVerifier::new(network(), sidechain_id, open_db(&dir, true));
            Self {
                _dir: dir,
                verifier,
                sidechain_id,
            }
        }

        fn add_header(&self, epoch: Epoch, block_hash: Hash32, block_output_mr: FixedHash) {
            let db = &self.verifier.output_inclusion.global_db;
            let mut tx = db.create_transaction().unwrap();
            db.block_headers(&mut tx)
                .insert(BlockHeaderModel {
                    epoch,
                    height: 1,
                    block_hash: FixedHash::from(block_hash.into_array()),
                    block_output_merkle_root: block_output_mr,
                    validator_node_merkle_root: FixedHash::zero(),
                })
                .unwrap();
            db.commit(tx).unwrap();
        }

        /// Mines `burn` in a block with a header in `header_epoch` and returns a claim of it
        fn mine(&self, burn: &L1Burn, header_epoch: Epoch) -> MinotariBurnClaimProof {
            let (proof, block_output_mr) = block_with(&burn.output, 2, 5, 3);
            self.add_header(header_epoch, proof.block_hash, block_output_mr);
            claim_of(burn, proof, self.sidechain_id.as_ref())
        }

        fn verify(
            &self,
            epoch: Epoch,
            claim: &MinotariBurnClaimProof,
        ) -> Result<RistrettoPublicKeyBytes, ClaimProofError> {
            self.verifier
                .verify_claim_proof(epoch, claim)
                .map(|verified| verified.claim_public_key)
        }
    }

    fn assert_invalid(result: Result<RistrettoPublicKeyBytes, ClaimProofError>, reason: &str) {
        let err = result.expect_err(reason);
        assert!(
            matches!(err, ClaimProofError::Rejected(ClaimProofRejection::Invalid(_))),
            "{reason}: expected Invalid, got {err:?}"
        );
    }

    #[test]
    fn output_hash_matches_the_l1_output_hash() {
        let secret = PrivateKey::random(&mut rand::rng());
        let claim_public_key = random_public_key();
        for burn in [
            l1_burn(None),
            l1_burn(Some(sidechain_id(&secret, &claim_public_key))),
            l1_output(OutputType::Standard, None),
        ] {
            let claim = claim_of(&burn, block_with(&burn.output, 0, 1, 0).0, None);
            let hash = hash_burn_output(network(), &claim).unwrap();
            if burn.output.features.output_type == OutputType::Burn {
                assert_eq!(hash.into_array(), burn.output.hash().into_array());
            } else {
                assert_ne!(
                    hash.into_array(),
                    burn.output.hash().into_array(),
                    "a claim cannot express a non-burn output"
                );
            }
        }
    }

    #[test]
    fn opaque_fields_hash_as_their_consensus_encoding() {
        // The L1 hasher consumes each field through its borsh encoding, so writing that encoding as raw bytes must
        // give the same hash. Exercise a non-trivial script, covenant and encrypted data.
        let mut burn = l1_burn(None);
        burn.output.script = script!(Nop Nop PushPubKey(Box::new(random_public_key()))).unwrap();
        burn.output.encrypted_data = EncryptedData::from_bytes(&[7u8; 120]).unwrap();
        let claim = claim_of(&burn, block_with(&burn.output, 0, 1, 0).0, None);
        assert_eq!(
            hash_burn_output(network(), &claim).unwrap().into_array(),
            burn.output.hash().into_array()
        );
    }

    #[test]
    fn accepts_a_burn_among_coinbases_and_normal_outputs() {
        let chain = Chain::new(None);
        let burn = l1_burn(None);
        let claim = chain.mine(&burn, Epoch(4));

        let claim_public_key = chain.verify(Epoch(5), &claim).unwrap();
        assert_eq!(claim_public_key, claim.output.features.claim_public_key);
    }

    #[test]
    fn accepts_a_burn_that_is_the_only_normal_output() {
        let chain = Chain::new(None);
        let burn = l1_burn(None);
        for num_coinbases in [0, 1, 3] {
            let (proof, block_output_mr) = block_with(&burn.output, num_coinbases, 1, 0);
            chain.add_header(Epoch(4), proof.block_hash, block_output_mr);
            let claim = claim_of(&burn, proof, None);
            chain.verify(Epoch(5), &claim).unwrap();
        }
    }

    #[test]
    fn accepts_a_claim_converted_from_the_minotari_proof() {
        let chain = Chain::new(None);
        let burn = l1_burn(None);
        let (inclusion, block_output_mr) = block_with(&burn.output, 2, 5, 3);
        chain.add_header(Epoch(4), inclusion.block_hash, block_output_mr);
        let expected = claim_of(&burn, inclusion.clone(), None);

        let fixed = |hash: &Hash32| tari_common_types::types::FixedHash::from(hash.into_array());
        let l1_mmr_proof = |proof: &MmrInclusionProof| L1MmrInclusionProof {
            leaf_index: proof.leaf_index,
            mmr_size: proof.mmr_size,
            path: proof.path.iter().map(fixed).collect(),
            peaks: proof.peaks.iter().map(fixed).collect(),
        };
        let output_proof = BurnOutputProof {
            block_hash: fixed(&inclusion.block_hash),
            block_height: 1,
            output: OutputHashPreimage::from(&burn.output),
            normal_output_proof: l1_mmr_proof(&inclusion.normal_output_proof),
            normal_output_mr: fixed(&inclusion.normal_output_mr),
            block_output_proof: l1_mmr_proof(&inclusion.block_output_proof),
        };
        output_proof
            .verify(&tari_common_types::types::FixedHash::from(block_output_mr.into_array()))
            .unwrap();
        let l1_claim = tari_sidechain::BurnClaimProof {
            burn_public_key: random_public_key(),
            ownership_proof: compressed_signature("ownership proof", &expected.ownership_proof).unwrap(),
            output_proof: burn_output_proof_from_l1(output_proof),
            value: VALUE,
        };

        let claim = claim_proof_from_l1(&l1_claim).unwrap();
        assert_eq!(claim, expected);
        chain.verify(Epoch(5), &claim).unwrap();
    }

    #[test]
    fn verifies_inclusion_for_varied_block_shapes() {
        let burn = l1_burn(None);
        // (coinbases, normal outputs, burn index)
        for (num_coinbases, num_normal, index) in [(0, 1, 0), (1, 2, 1), (2, 3, 2), (5, 7, 3), (8, 16, 15), (3, 33, 0)]
        {
            let (proof, block_output_mr) = block_with(&burn.output, num_coinbases, num_normal, index);
            verify_output_inclusion(
                &proof,
                &FixedHash::from(burn.output.hash().into_array()),
                &block_output_mr,
            )
            .unwrap_or_else(|e| panic!("({num_coinbases}, {num_normal}, {index}): {e}"));
        }
    }

    #[test]
    fn rejects_inclusion_against_the_wrong_root() {
        let chain = Chain::new(None);
        let burn = l1_burn(None);
        let (proof, _) = block_with(&burn.output, 2, 5, 3);
        chain.add_header(Epoch(4), proof.block_hash, FixedHash::from([1u8; 32]));
        let claim = claim_of(&burn, proof, None);

        assert_invalid(chain.verify(Epoch(5), &claim), "the wrong root must be rejected");
    }

    #[test]
    fn rejects_a_non_burn_output() {
        let chain = Chain::new(None);
        let output = l1_output(OutputType::Standard, confidential_output_feature(None));
        let claim = chain.mine(&output, Epoch(4));

        assert_invalid(chain.verify(Epoch(5), &claim), "a non-burn output must be rejected");
    }

    #[test]
    fn rejects_a_burn_without_a_sidechain_feature() {
        let chain = Chain::new(None);
        let burn = l1_output(OutputType::Burn, None);
        let claim = chain.mine(&burn, Epoch(4));

        assert_invalid(chain.verify(Epoch(5), &claim), "an untagged burn must be rejected");
    }

    #[test]
    fn rejects_a_burn_with_another_sidechain_feature() {
        let chain = Chain::new(None);
        let exit = ValidatorNodeExit::signed(&PrivateKey::random(&mut rand::rng()), 0, None, VnEpoch(0), VnEpoch(1));
        let burn = l1_output(
            OutputType::Burn,
            Some(SideChainFeature {
                data: SideChainFeatureData::ValidatorNodeExit(exit),
                sidechain_id: None,
            }),
        );
        let claim = chain.mine(&burn, Epoch(4));

        assert_invalid(
            chain.verify(Epoch(5), &claim),
            "a burn whose sidechain feature is not a ConfidentialOutput must be rejected",
        );
    }

    #[test]
    fn rejects_a_tagged_burn_on_the_default_chain() {
        let chain = Chain::new(None);
        let secret = PrivateKey::random(&mut rand::rng());
        let burn = l1_burn(Some(sidechain_id(&secret, &random_public_key())));
        let claim = chain.mine(&burn, Epoch(4));

        assert_invalid(
            chain.verify(Epoch(5), &claim),
            "a burn tagged for a sidechain must be rejected on the default chain",
        );
    }

    #[test]
    fn rejects_a_burn_tagged_for_another_chain_or_untagged_on_a_tagged_chain() {
        let this_secret = PrivateKey::random(&mut rand::rng());
        let this_id = RistrettoPublicKey::from_secret_key(&this_secret).to_byte_type();
        let chain = Chain::new(Some(this_id));

        let accepted = l1_burn(Some(sidechain_id(&this_secret, &random_public_key())));
        let claim = chain.mine(&accepted, Epoch(4));
        chain.verify(Epoch(5), &claim).unwrap();

        let other_secret = PrivateKey::random(&mut rand::rng());
        let other_chain = l1_burn(Some(sidechain_id(&other_secret, &random_public_key())));
        let claim = chain.mine(&other_chain, Epoch(4));
        assert_invalid(
            chain.verify(Epoch(5), &claim),
            "a burn tagged for another chain must be rejected",
        );

        let untagged = l1_burn(None);
        let claim = chain.mine(&untagged, Epoch(4));
        assert_invalid(
            chain.verify(Epoch(5), &claim),
            "an untagged burn must be rejected on a tagged chain",
        );
    }

    #[test]
    fn rejects_an_ownership_proof_bound_to_another_key() {
        let chain = Chain::new(None);
        let burn = l1_burn(None);
        let (proof, block_output_mr) = block_with(&burn.output, 2, 5, 3);
        chain.add_header(Epoch(4), proof.block_hash, block_output_mr);
        // The burner knows the commitment opening, so it can bind a valid ownership proof to its own key
        let burner_key = RistrettoPublicKey::random_keypair(&mut rand::rng()).1.to_byte_type();
        let claim = claim_with(&burn, burn_output(&burn.output), proof, &burner_key, None);

        assert_invalid(
            chain.verify(Epoch(5), &claim),
            "an ownership proof not bound to the output's claim key must be rejected",
        );
    }

    #[test]
    fn rejects_a_commitment_mismatch() {
        let chain = Chain::new(None);
        let burn = l1_burn(None);
        let other = l1_burn(None);
        // The ownership proof opens `other`'s commitment, but the output mined is `burn`'s
        let mut claim = chain.mine(&burn, Epoch(4));
        let other_claim = claim_of(&other, claim.inclusion_proof.clone(), None);
        claim.commitment = other_claim.commitment;
        claim.ownership_proof = other_claim.ownership_proof;

        assert_invalid(
            chain.verify(Epoch(5), &claim),
            "a claim whose commitment is not the output's must be rejected",
        );
    }

    #[test]
    fn rejects_a_tampered_opaque_field() {
        let chain = Chain::new(None);
        let burn = l1_burn(None);
        let mut claim = chain.mine(&burn, Epoch(4));
        claim.output.covenant = bytes(vec![1, 0]);

        assert_invalid(chain.verify(Epoch(5), &claim), "a tampered output must be rejected");
    }

    #[test]
    fn the_sender_offset_public_key_is_pinned_to_its_place_in_the_output() {
        let chain = Chain::new(None);
        let burn = l1_burn(None);
        let claim = chain.mine(&burn, Epoch(4));
        chain.verify(Epoch(5), &claim).unwrap();

        // Move the script/sender offset boundary one key encoding later. The metadata signature starts with a key
        // encoded the same way, so the preimage, and so the hash, is unchanged.
        let encoded_key = borsh::to_vec(&burn.output.sender_offset_public_key).unwrap();
        let prefix_len = encoded_key.len() - 32;
        let metadata_signature = claim.output.metadata_signature.as_slice();
        assert_eq!(metadata_signature[..prefix_len], encoded_key[..prefix_len]);
        let mut shifted = claim.clone();
        shifted.output.script = bytes([claim.output.script.as_slice(), &encoded_key].concat());
        shifted.output.sender_offset_public_key =
            RistrettoPublicKeyBytes::from_bytes(&metadata_signature[prefix_len..encoded_key.len()]).unwrap();
        shifted.output.metadata_signature = bytes(metadata_signature[encoded_key.len()..].to_vec());

        assert_invalid(
            chain.verify(Epoch(5), &shifted),
            "a claim that moves the sender offset public key must be rejected",
        );
    }

    #[test]
    fn script_length_prefix_must_match_its_bytes() {
        for (script, ok) in [
            (vec![0], true),
            (vec![2, 1, 2], true),
            (vec![2, 1], false),
            (vec![1, 1, 2], false),
            ([vec![0x80, 0x20], vec![0; 4096]].concat(), true),
            (vec![0x80], false),
            (vec![0xff; 11], false),
        ] {
            assert_eq!(super::check_script_length(&script).is_ok(), ok, "{script:?}");
        }
    }

    #[test]
    fn rejects_a_normal_output_root_that_is_not_the_last_leaf() {
        let burn = l1_burn(None);
        let (mut proof, block_output_mr) = block_with(&burn.output, 2, 3, 1);
        proof.block_output_proof.leaf_index = 1;
        let err = verify_output_inclusion(
            &proof,
            &FixedHash::from(burn.output.hash().into_array()),
            &block_output_mr,
        )
        .unwrap_err();
        assert!(err.contains("last leaf"), "{err}");
    }

    #[test]
    fn malformed_mmr_proofs_are_errors() {
        let burn = l1_burn(None);
        let hash = FixedHash::from(burn.output.hash().into_array());
        let (proof, block_output_mr) = block_with(&burn.output, 2, 5, 3);
        let tampered = |f: &dyn Fn(&mut BurnOutputInclusionProof)| {
            let mut proof = proof.clone();
            f(&mut proof);
            proof
        };
        let cases = [
            tampered(&|p| p.block_output_proof.mmr_size = u64::MAX),
            tampered(&|p| p.block_output_proof.mmr_size = 0),
            tampered(&|p| p.normal_output_proof.mmr_size = u64::MAX),
            tampered(&|p| p.normal_output_proof.mmr_size = 0),
            tampered(&|p| p.normal_output_proof.leaf_index = u64::MAX),
            tampered(&|p| p.normal_output_proof.leaf_index = 1 << 62),
            tampered(&|p| p.normal_output_proof.path = vec![Hash32::zero(); MAX_MMR_PROOF_HASHES + 1]),
            tampered(&|p| p.normal_output_proof.path = vec![Hash32::zero(); MAX_MMR_PROOF_HASHES]),
            tampered(&|p| p.normal_output_proof.peaks = vec![Hash32::zero(); MAX_MMR_PROOF_HASHES]),
            tampered(&|p| p.normal_output_proof.path.clear()),
            tampered(&|p| p.block_output_proof.peaks.push(Hash32::zero())),
        ];
        for (i, proof) in cases.iter().enumerate() {
            verify_output_inclusion(proof, &hash, &block_output_mr).expect_err(&format!("case {i} must be rejected"));
        }
    }

    #[test]
    fn a_header_from_the_current_epoch_is_not_yet_valid() {
        let chain = Chain::new(None);
        let claim = chain.mine(&l1_burn(None), Epoch(5));

        let err = chain.verify(Epoch(5), &claim).unwrap_err();
        assert!(
            matches!(err, ClaimProofError::Rejected(ClaimProofRejection::NotYetValid(_))),
            "expected NotYetValid, got {err:?}"
        );
        chain.verify(Epoch(6), &claim).unwrap();
    }

    #[test]
    fn nothing_is_claimable_in_epoch_zero() {
        let chain = Chain::new(None);
        let claim = chain.mine(&l1_burn(None), Epoch(0));

        let err = chain.verify(Epoch(0), &claim).unwrap_err();
        assert!(
            matches!(err, ClaimProofError::Rejected(ClaimProofRejection::NotYetValid(_))),
            "expected NotYetValid, got {err:?}"
        );
    }

    #[test]
    fn a_database_failure_is_a_verifier_fault() {
        let chain = Chain::new(None);
        let claim = chain.mine(&l1_burn(None), Epoch(4));
        // An unmigrated database has no block_headers table, so the lookup fails with a SQL error.
        let dir = TempDir::new().unwrap();
        let verifier = TariClaimBurnProofVerifier::new(network(), None, open_db(&dir, false));

        let err = verifier.verify_claim_proof(Epoch(5), &claim).unwrap_err();
        assert!(
            matches!(err, ClaimProofError::VerifierFault(_)),
            "expected VerifierFault, got {err:?}"
        );
    }

    #[test]
    fn headerless_verifier_checks_everything_but_inclusion() {
        let verifier = HeaderlessClaimBurnProofVerifier::new(network(), None);

        let burn = l1_burn(None);
        let claim = claim_of(&burn, block_with(&burn.output, 0, 1, 0).0, None);
        let verified = verifier.verify_claim_proof(Epoch(0), &claim).unwrap();
        assert_eq!(verified.claim_public_key, claim.output.features.claim_public_key);

        let secret = PrivateKey::random(&mut rand::rng());
        let tagged = l1_burn(Some(sidechain_id(&secret, &random_public_key())));
        let claim = claim_of(&tagged, block_with(&tagged.output, 0, 1, 0).0, None);
        let err = verifier.verify_claim_proof(Epoch(0), &claim).unwrap_err();
        assert!(
            matches!(err, ClaimProofError::Rejected(ClaimProofRejection::Invalid(_))),
            "expected Invalid, got {err:?}"
        );

        let other_key = RistrettoPublicKey::random_keypair(&mut rand::rng()).1.to_byte_type();
        let claim = claim_with(
            &burn,
            burn_output(&burn.output),
            block_with(&burn.output, 0, 1, 0).0,
            &other_key,
            None,
        );
        verifier
            .verify_claim_proof(Epoch(0), &claim)
            .expect_err("the ownership proof must still be checked");
    }
}
