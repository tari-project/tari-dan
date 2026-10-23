//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_engine_types::confidential::{
    BurnOutput,
    BurnOutputFeatures,
    BurnOutputInclusionProof,
    BurnSidechainId,
    MinotariBurnClaimProof,
    MmrInclusionProof as ClaimMmrInclusionProof,
};
use tari_sidechain::{BurnClaimProof, BurnOutputProof, FixedHash, MmrInclusionProof, OutputHashPreimage};
use tari_template_lib::types::Hash32;
use tari_transaction_components::transaction_components::{OutputFeatures, OutputType, SideChainFeatureData};

/// Builds the Ootle claim of an L1 burn from the burn claim proof a Minotari wallet produces
pub fn claim_proof_from_l1(proof: &BurnClaimProof) -> Result<MinotariBurnClaimProof, String> {
    let BurnOutputProof {
        block_hash,
        output,
        normal_output_proof,
        normal_output_mr,
        block_output_proof,
        ..
    } = &proof.output_proof;

    let features: OutputFeatures =
        borsh::from_slice(&output.features).map_err(|e| format!("invalid burn output features: {e}"))?;
    if features.output_type != OutputType::Burn {
        return Err(format!("the output is a {} output, not a burn", features.output_type));
    }
    let sidechain_feature = features
        .sidechain_feature
        .as_ref()
        .ok_or("the burn output has no sidechain feature, so it names no claimant")?;
    let SideChainFeatureData::ConfidentialOutput(confidential_output) = &sidechain_feature.data else {
        return Err("the burn output's sidechain feature is not a confidential output".to_string());
    };

    Ok(MinotariBurnClaimProof {
        commitment: output.commitment.to_byte_type(),
        ownership_proof: proof.ownership_proof.to_byte_type(),
        value: proof.value,
        output: BurnOutput {
            version: output.version,
            features: BurnOutputFeatures {
                version: features.version.as_u8(),
                maturity: features.maturity,
                claim_public_key: confidential_output.claim_public_key.to_byte_type(),
                sidechain_id: sidechain_feature.sidechain_id().map(|id| BurnSidechainId {
                    public_key: id.public_key().to_byte_type(),
                    knowledge_proof: id.knowledge_proof().to_byte_type(),
                }),
                range_proof_type: features.range_proof_type.as_byte(),
            },
            rangeproof_hash: Hash32::from_array(output.rangeproof_hash.into_array()),
            script: bytes("script", output.script.clone())?,
            sender_offset_public_key: output.sender_offset_public_key.to_byte_type(),
            metadata_signature: bytes(
                "metadata signature",
                borsh::to_vec(&output.metadata_signature).map_err(|e| e.to_string())?,
            )?,
            covenant: bytes("covenant", output.covenant.clone())?,
            encrypted_data: bytes("encrypted data", output.encrypted_data.clone())?,
            minimum_value_promise: output.minimum_value_promise,
        },
        inclusion_proof: BurnOutputInclusionProof {
            block_hash: Hash32::from_array(block_hash.into_array()),
            normal_output_proof: mmr_proof(normal_output_proof),
            normal_output_mr: Hash32::from_array(normal_output_mr.into_array()),
            block_output_proof: mmr_proof(block_output_proof),
        },
    })
}

fn bytes<const N: usize>(field: &str, bytes: Vec<u8>) -> Result<bounded_vec::BoundedVec<u8, 1, N>, String> {
    let len = bytes.len();
    bounded_vec::BoundedVec::<u8, 1, N>::from_vec(bytes)
        .map_err(|_| format!("burn output {field} length {len} is out of range"))
}

fn mmr_proof(proof: &MmrInclusionProof) -> ClaimMmrInclusionProof {
    let hashes = |hashes: &[tari_sidechain::FixedHash]| {
        hashes
            .iter()
            .map(|hash| Hash32::from_array(hash.into_array()))
            .collect()
    };
    ClaimMmrInclusionProof {
        leaf_index: proof.leaf_index,
        mmr_size: proof.mmr_size,
        path: hashes(&proof.path),
        peaks: hashes(&proof.peaks),
    }
}

/// Converts the burn output proof a Minotari wallet or base node returns over gRPC into the form a
/// [`BurnClaimProof`] carries. The two have the same fields and the same JSON encoding.
pub fn burn_output_proof_from_l1(proof: tari_sidechain_l1::BurnOutputProof) -> BurnOutputProof {
    let hash = |hash: tari_sidechain_l1::FixedHash| FixedHash::from(<[u8; 32]>::from(hash));
    let mmr = |proof: tari_sidechain_l1::MmrInclusionProof| MmrInclusionProof {
        leaf_index: proof.leaf_index,
        mmr_size: proof.mmr_size,
        path: proof.path.into_iter().map(hash).collect(),
        peaks: proof.peaks.into_iter().map(hash).collect(),
    };
    let output = proof.output;
    BurnOutputProof {
        block_hash: hash(proof.block_hash),
        block_height: proof.block_height,
        output: OutputHashPreimage {
            version: output.version,
            features: output.features,
            commitment: output.commitment,
            rangeproof_hash: hash(output.rangeproof_hash),
            script: output.script,
            sender_offset_public_key: output.sender_offset_public_key,
            metadata_signature: output.metadata_signature,
            covenant: output.covenant,
            encrypted_data: output.encrypted_data,
            minimum_value_promise: output.minimum_value_promise,
        },
        normal_output_proof: mmr(proof.normal_output_proof),
        normal_output_mr: hash(proof.normal_output_mr),
        block_output_proof: mmr(proof.block_output_proof),
    }
}

#[cfg(test)]
mod tests {
    use tari_common_types::{
        epoch::VnEpoch,
        types::{CompressedPublicKey, PrivateKey},
    };
    use tari_crypto::keys::{PublicKey as _, SecretKey as _};
    use tari_sidechain::CompleteClaimBurnProof;
    use tari_transaction_components::transaction_components::{
        ConfidentialOutputData,
        SideChainFeature,
        ValidatorNodeExit,
    };

    use super::*;

    fn random_public_key() -> CompressedPublicKey {
        CompressedPublicKey::new_from_pk(tari_crypto::ristretto::RistrettoPublicKey::random_keypair(&mut rand::rng()).1)
    }

    fn l1_mmr_proof(leaf_index: u64) -> MmrInclusionProof {
        MmrInclusionProof {
            leaf_index,
            mmr_size: 4,
            path: vec![FixedHash::from([1u8; 32]), FixedHash::from([2u8; 32])],
            peaks: vec![FixedHash::from([3u8; 32])],
        }
    }

    /// A Minotari burn proof file, as JSON, for an output with `features`
    fn proof_file(features: &OutputFeatures) -> String {
        let proof = CompleteClaimBurnProof {
            claim_proof: BurnClaimProof {
                burn_public_key: random_public_key(),
                ownership_proof: Default::default(),
                output_proof: BurnOutputProof {
                    block_hash: FixedHash::from([4u8; 32]),
                    block_height: 100,
                    output: OutputHashPreimage {
                        version: 1,
                        features: borsh::to_vec(features).unwrap(),
                        commitment: Default::default(),
                        rangeproof_hash: FixedHash::from([5u8; 32]),
                        script: vec![1, 0x73],
                        sender_offset_public_key: random_public_key(),
                        metadata_signature: Default::default(),
                        covenant: vec![0],
                        encrypted_data: vec![0; 84],
                        minimum_value_promise: 0,
                    },
                    normal_output_proof: l1_mmr_proof(2),
                    normal_output_mr: FixedHash::from([6u8; 32]),
                    block_output_proof: l1_mmr_proof(1),
                },
                value: 12_345,
            },
            encrypted_data: vec![9; 80],
            mined_in_epoch: 3,
        };
        serde_json::to_string(&proof).unwrap()
    }

    fn convert(json: &str) -> Result<MinotariBurnClaimProof, String> {
        let proof: CompleteClaimBurnProof = serde_json::from_str(json).unwrap();
        claim_proof_from_l1(&proof.claim_proof)
    }

    fn burn_features(sidechain_feature: Option<SideChainFeature>) -> OutputFeatures {
        OutputFeatures {
            output_type: OutputType::Burn,
            sidechain_feature,
            ..Default::default()
        }
    }

    fn confidential_output(claim_public_key: CompressedPublicKey) -> Option<SideChainFeature> {
        Some(SideChainFeature {
            data: SideChainFeatureData::ConfidentialOutput(ConfidentialOutputData { claim_public_key }),
            sidechain_id: None,
        })
    }

    #[test]
    fn converts_a_minotari_proof_file() {
        let claim_public_key = random_public_key();
        let json = proof_file(&burn_features(confidential_output(claim_public_key.clone())));
        let file: CompleteClaimBurnProof = serde_json::from_str(&json).unwrap();

        let claim = convert(&json).unwrap();
        let output = &file.claim_proof.output_proof.output;
        assert_eq!(claim.output.features.claim_public_key, claim_public_key.to_byte_type());
        assert_eq!(claim.output.features.sidechain_id, None);
        assert_eq!(
            claim.output.sender_offset_public_key,
            output.sender_offset_public_key.to_byte_type()
        );
        assert_eq!(claim.output.script.as_slice(), output.script.as_slice());
        assert_eq!(
            claim.output.metadata_signature.as_slice(),
            borsh::to_vec(&output.metadata_signature).unwrap().as_slice()
        );
        assert_eq!(claim.value, 12_345);
        assert_eq!(claim.inclusion_proof.normal_output_proof.leaf_index, 2);
        assert_eq!(claim.inclusion_proof.block_output_proof.path.len(), 2);
        assert_eq!(claim.inclusion_proof.block_hash, Hash32::from_array([4u8; 32]));
    }

    #[test]
    fn rejects_an_output_ootle_cannot_claim() {
        let exit = ValidatorNodeExit::signed(&PrivateKey::random(&mut rand::rng()), 0, None, VnEpoch(0), VnEpoch(1));
        for (features, reason) in [
            (
                OutputFeatures {
                    sidechain_feature: confidential_output(random_public_key()),
                    ..Default::default()
                },
                "not a burn",
            ),
            (burn_features(None), "no sidechain feature"),
            (
                burn_features(Some(SideChainFeature {
                    data: SideChainFeatureData::ValidatorNodeExit(exit.clone()),
                    sidechain_id: None,
                })),
                "not a confidential output",
            ),
        ] {
            let err = convert(&proof_file(&features)).unwrap_err();
            assert!(err.contains(reason), "expected '{reason}', got '{err}'");
        }
    }
}
