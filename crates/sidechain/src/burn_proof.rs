//  Copyright 2026. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_crypto::ristretto::{
    CompressedRistrettoComAndPubSig,
    CompressedRistrettoSchnorr,
    pedersen::CompressedPedersenCommitment,
};

use crate::{
    serde as serializers,
    types::{CompressedPublicKey, FixedHash},
};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CompleteClaimBurnProof {
    pub claim_proof: BurnClaimProof,
    #[serde(with = "serializers::base64")]
    pub encrypted_data: Vec<u8>,
    /// The L1 epoch the burn was mined in (`block_height / vn_epoch_length`). Lets an L2 claimant defer the
    /// claim until L2 has synced past this epoch, avoiding a premature, permanently-rejected submission.
    pub mined_in_epoch: u64,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct BurnClaimProof {
    /// The L2 account public key (`P`) the burn is intended for
    pub burn_public_key: CompressedPublicKey,
    pub ownership_proof: CompressedRistrettoSchnorr,
    /// Proves that the burn output was mined in an L1 block. The output carries the commitment, the sender offset
    /// public key and the claim public key (in its features).
    pub output_proof: BurnOutputProof,
    pub value: u64,
}

/// Proves that a burn output was mined in a block, against that block's `block_output_mr`.
///
/// The block output MMR has the coinbase output hashes as leaves, followed by one last leaf: the root of the MMR of
/// every other output hash in the block (the normal output MMR). Burn outputs are never coinbases, so the proof is in
/// two levels: the output hash in the normal output MMR, then the normal output MMR root in the block output MMR.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BurnOutputProof {
    /// The hash of the block the output was mined in
    #[serde(with = "serializers::hex")]
    pub block_hash: FixedHash,
    /// The height of the block the output was mined in
    pub block_height: u64,
    /// The burn output
    pub output: OutputHashPreimage,
    /// The inclusion proof of the output hash in the normal output MMR
    pub normal_output_proof: MmrInclusionProof,
    /// The root of the normal output MMR
    #[serde(with = "serializers::hex")]
    pub normal_output_mr: FixedHash,
    /// The inclusion proof of `normal_output_mr` in the block output MMR (as its last leaf, for an honest proof)
    pub block_output_proof: MmrInclusionProof,
}

/// An inclusion proof of a leaf in a Merkle mountain range, in a self-describing form so that consumers can verify it
/// without decoding the bincode encoding of `tari_mmr::MerkleProof`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MmrInclusionProof {
    /// The index of the leaf in the MMR
    pub leaf_index: u64,
    /// The size (node count) of the MMR the proof was created for
    pub mmr_size: u64,
    /// The sibling path from the leaf up to its local peak
    #[serde(with = "serializers::hex_seq")]
    pub path: Vec<FixedHash>,
    /// The MMR peaks, excluding the local peak of the leaf
    #[serde(with = "serializers::hex_seq")]
    pub peaks: Vec<FixedHash>,
}

/// The fields of a base layer `TransactionOutput` that its hash commits to, in hash order. The range proof is carried
/// only as its hash, which is all the output hash commits to.
///
/// The fields with types that live in `tari_transaction_components` (`features`, `script`, `covenant` and
/// `encrypted_data`) are carried as their consensus (borsh) encoding, which is exactly what the output hash consumes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutputHashPreimage {
    pub version: u8,
    /// Consensus encoded `OutputFeatures`
    #[serde(with = "serializers::hex")]
    pub features: Vec<u8>,
    pub commitment: CompressedPedersenCommitment,
    #[serde(with = "serializers::hex")]
    pub rangeproof_hash: FixedHash,
    /// Consensus encoded `TariScript`
    #[serde(with = "serializers::hex")]
    pub script: Vec<u8>,
    pub sender_offset_public_key: CompressedPublicKey,
    pub metadata_signature: CompressedRistrettoComAndPubSig,
    /// Consensus encoded `Covenant`
    #[serde(with = "serializers::hex")]
    pub covenant: Vec<u8>,
    /// Consensus encoded `EncryptedData`
    #[serde(with = "serializers::hex")]
    pub encrypted_data: Vec<u8>,
    pub minimum_value_promise: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mmr_proof() -> MmrInclusionProof {
        MmrInclusionProof {
            leaf_index: 0,
            mmr_size: 1,
            path: vec![],
            peaks: vec![],
        }
    }

    #[test]
    fn it_round_trips_as_json() {
        let proof = CompleteClaimBurnProof {
            claim_proof: BurnClaimProof {
                burn_public_key: Default::default(),
                ownership_proof: Default::default(),
                output_proof: BurnOutputProof {
                    block_hash: Default::default(),
                    block_height: 100,
                    output: OutputHashPreimage {
                        version: 1,
                        features: vec![1, 2],
                        commitment: Default::default(),
                        rangeproof_hash: Default::default(),
                        script: vec![3],
                        sender_offset_public_key: Default::default(),
                        metadata_signature: Default::default(),
                        covenant: vec![0],
                        encrypted_data: vec![9; 10],
                        minimum_value_promise: 0,
                    },
                    normal_output_proof: mmr_proof(),
                    normal_output_mr: Default::default(),
                    block_output_proof: mmr_proof(),
                },
                value: 12_345,
            },
            encrypted_data: vec![9, 9, 9],
            mined_in_epoch: 42,
        };
        let json = serde_json::to_string(&proof).unwrap();
        let parsed: CompleteClaimBurnProof = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.mined_in_epoch, 42);
        assert_eq!(parsed.claim_proof.output_proof, proof.claim_proof.output_proof);
    }
}
