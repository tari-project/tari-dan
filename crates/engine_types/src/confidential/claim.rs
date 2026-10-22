//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, fmt::Display};

use serde::{Deserialize, Serialize};
use tari_template_lib::types::{
    EncryptedData,
    Hash32,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, SchnorrSignatureBytes},
};

/// The largest consensus encoding of an L1 `TariScript`: a 2-byte varint length and up to 4096 script bytes.
pub const MAX_BURN_OUTPUT_SCRIPT_LEN: usize = 4098;
/// The largest consensus encoding of an L1 `Covenant`: a 2-byte varint length and up to 4096 covenant bytes.
pub const MAX_BURN_OUTPUT_COVENANT_LEN: usize = 4098;
/// The largest consensus encoding of an L1 output's `EncryptedData`: a `u32` length and up to 336 bytes.
pub const MAX_BURN_OUTPUT_ENCRYPTED_DATA_LEN: usize = 340;
/// The largest consensus encoding of an L1 `ComAndPubSignature`.
pub const MAX_BURN_OUTPUT_METADATA_SIGNATURE_LEN: usize = 256;
/// The most hashes an MMR inclusion proof's `path` or `peaks` can hold. An MMR indexed by a `u64` has at most 64
/// peaks and a path of at most 63 siblings.
pub const MAX_MMR_PROOF_HASHES: usize = 64;

/// Proves that an L1 burn output exists and that the claimant can open its commitment.
///
/// The output is proven by recomputing its L1 hash from [`BurnOutput`] and folding that hash up to the
/// `block_output_mr` of the L1 header named by [`BurnOutputInclusionProof::block_hash`].
#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    PartialEq,
    borsh::BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MinotariBurnClaimProof {
    /// The commitment of the burn output
    #[n(0)]
    pub commitment: PedersenCommitmentBytes,
    #[n(1)]
    pub ownership_proof: SchnorrSignatureBytes,
    #[n(2)]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    pub value: u64,
    #[n(3)]
    pub output: BurnOutput,
    #[n(4)]
    pub inclusion_proof: BurnOutputInclusionProof,
}

impl Display for MinotariBurnClaimProof {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "MinotariBurnClaimProof (commitment: {}, claim_public_key: {}, block_hash: {}, value: {})",
            self.commitment, self.output.features.claim_public_key, self.inclusion_proof.block_hash, self.value,
        )
    }
}

/// The fields of an L1 burn output that its hash commits to, in hash order, less the commitment, which is
/// [`MinotariBurnClaimProof::commitment`].
///
/// The range proof is carried as its hash, which is all the output hash commits to. `script`, `metadata_signature`,
/// `covenant` and `encrypted_data` are their L1 consensus (borsh) encodings, hashed as given and never decoded.
#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    PartialEq,
    borsh::BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BurnOutput {
    #[n(0)]
    pub version: u8,
    #[n(1)]
    pub features: BurnOutputFeatures,
    #[n(2)]
    #[borsh(serialize_with = "serialize_bytes")]
    pub rangeproof_hash: Hash32,
    #[n(3)]
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[borsh(serialize_with = "serialize_bytes")]
    #[cbor(with = "bounded_vec_bytes")]
    pub script: bounded_vec::BoundedVec<u8, 1, MAX_BURN_OUTPUT_SCRIPT_LEN>,
    /// The public nonce `R` the output was burnt with
    #[n(4)]
    pub sender_offset_public_key: RistrettoPublicKeyBytes,
    #[n(5)]
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[borsh(serialize_with = "serialize_bytes")]
    #[cbor(with = "bounded_vec_bytes")]
    pub metadata_signature: bounded_vec::BoundedVec<u8, 1, MAX_BURN_OUTPUT_METADATA_SIGNATURE_LEN>,
    #[n(6)]
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[borsh(serialize_with = "serialize_bytes")]
    #[cbor(with = "bounded_vec_bytes")]
    pub covenant: bounded_vec::BoundedVec<u8, 1, MAX_BURN_OUTPUT_COVENANT_LEN>,
    #[n(7)]
    #[serde(with = "ootle_serde::base64")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[borsh(serialize_with = "serialize_bytes")]
    #[cbor(with = "bounded_vec_bytes")]
    pub encrypted_data: bounded_vec::BoundedVec<u8, 1, MAX_BURN_OUTPUT_ENCRYPTED_DATA_LEN>,
    #[n(8)]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    pub minimum_value_promise: u64,
}

/// The `OutputFeatures` of an L1 burn output that Ootle mints against.
///
/// Only the fields that vary between such outputs are carried. The rest are fixed: the output type is `Burn`, the
/// coinbase extra is empty, and the sidechain feature is a `ConfidentialOutput` naming `claim_public_key`.
#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    PartialEq,
    borsh::BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BurnOutputFeatures {
    #[n(0)]
    pub version: u8,
    #[n(1)]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    pub maturity: u64,
    /// The stealth key `S` the burn is made out to. Only its holder may claim the burn.
    #[n(2)]
    pub claim_public_key: RistrettoPublicKeyBytes,
    /// The sidechain the burn is tagged for, with the deployment key's signature of `claim_public_key`. `None` is
    /// the default Ootle chain.
    #[n(3)]
    pub sidechain_id: Option<BurnSidechainId>,
    #[n(4)]
    pub range_proof_type: u8,
}

#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    PartialEq,
    borsh::BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BurnSidechainId {
    #[n(0)]
    pub public_key: RistrettoPublicKeyBytes,
    #[n(1)]
    pub knowledge_proof: SchnorrSignatureBytes,
}

/// Proves that a burn output is in an L1 block, against that block's `block_output_mr`.
///
/// The block output MMR has the block's coinbase output hashes as leaves, then one last leaf: the root of the MMR of
/// every other output hash in the block (the normal output MMR). A burn is never a coinbase, so the proof has two
/// levels: the output hash in the normal output MMR, then that MMR's root as the last leaf of the block output MMR.
#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    PartialEq,
    borsh::BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct BurnOutputInclusionProof {
    #[n(0)]
    #[borsh(serialize_with = "serialize_bytes")]
    pub block_hash: Hash32,
    #[n(1)]
    pub normal_output_proof: MmrInclusionProof,
    #[n(2)]
    #[borsh(serialize_with = "serialize_bytes")]
    pub normal_output_mr: Hash32,
    #[n(3)]
    pub block_output_proof: MmrInclusionProof,
}

/// An inclusion proof of a leaf in an L1 Merkle mountain range.
#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    PartialEq,
    borsh::BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct MmrInclusionProof {
    #[n(0)]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    pub leaf_index: u64,
    /// The node count of the MMR the proof was made for
    #[n(1)]
    #[serde(deserialize_with = "ootle_serde::str_number::deserialize")]
    #[cfg_attr(feature = "ts", ts(type = "number | bigint | string"))]
    pub mmr_size: u64,
    /// The sibling hashes from the leaf up to its local peak
    #[n(2)]
    #[serde(deserialize_with = "bounded_hashes::deserialize")]
    #[cbor(with = "bounded_hashes")]
    pub path: Vec<Hash32>,
    /// The MMR peaks, less the leaf's local peak
    #[n(3)]
    #[serde(deserialize_with = "bounded_hashes::deserialize")]
    #[cbor(with = "bounded_hashes")]
    pub peaks: Vec<Hash32>,
}

#[derive(
    Debug,
    Clone,
    Deserialize,
    Serialize,
    PartialEq,
    borsh::BorshSerialize,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ClaimBurnOutputData {
    #[n(0)]
    pub encrypted_data: EncryptedData,
}

/// Adapter for `bounded_vec::BoundedVec<u8, LOW, HIGH>` so it can participate in minicbor derives.
/// On the wire encodes as a CBOR byte string (matches the canonical bytes encoding).
mod bounded_vec_bytes {
    use bounded_vec::BoundedVec;
    use minicbor::{Decoder, Encoder};

    pub fn encode<C, W, const LOW: usize, const HIGH: usize>(
        v: &BoundedVec<u8, LOW, HIGH>,
        e: &mut Encoder<W>,
        _ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>>
    where
        W: minicbor::encode::Write,
    {
        e.bytes(v.as_slice())?;
        Ok(())
    }

    pub fn decode<'b, C, const LOW: usize, const HIGH: usize>(
        d: &mut Decoder<'b>,
        _ctx: &mut C,
    ) -> Result<BoundedVec<u8, LOW, HIGH>, minicbor::decode::Error> {
        let bytes = d.bytes()?;
        <BoundedVec<u8, LOW, HIGH>>::from_vec(bytes.to_vec())
            .map_err(|_| minicbor::decode::Error::message("BoundedVec length out of bounds"))
    }

    pub fn cbor_len<C, const LOW: usize, const HIGH: usize>(v: &BoundedVec<u8, LOW, HIGH>, ctx: &mut C) -> usize {
        minicbor::bytes::cbor_len(v.as_slice(), ctx)
    }
}

/// Codecs for a list of at most [`MAX_MMR_PROOF_HASHES`] hashes. The declared length is untrusted, so it is checked
/// against the bound before anything is allocated.
mod bounded_hashes {
    use std::fmt;

    use minicbor::{CborLen, Decode, Decoder, Encode, Encoder};
    use serde::{Deserializer, de};
    use tari_template_lib::types::Hash32;

    use super::MAX_MMR_PROOF_HASHES;

    pub fn encode<C, W>(
        v: &[Hash32],
        e: &mut Encoder<W>,
        ctx: &mut C,
    ) -> Result<(), minicbor::encode::Error<W::Error>>
    where
        W: minicbor::encode::Write,
    {
        e.array(v.len() as u64)?;
        for hash in v {
            hash.encode(e, ctx)?;
        }
        Ok(())
    }

    pub fn decode<'b, C>(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Vec<Hash32>, minicbor::decode::Error> {
        let too_many = || minicbor::decode::Error::message("too many MMR proof hashes");
        let mut hashes = Vec::new();
        match d.array()? {
            Some(len) => {
                let len = usize::try_from(len)
                    .ok()
                    .filter(|len| *len <= MAX_MMR_PROOF_HASHES)
                    .ok_or_else(too_many)?;
                hashes.reserve_exact(len);
                for _ in 0..len {
                    hashes.push(Hash32::decode(d, ctx)?);
                }
            },
            None => loop {
                if d.datatype()? == minicbor::data::Type::Break {
                    d.skip()?;
                    break;
                }
                if hashes.len() == MAX_MMR_PROOF_HASHES {
                    return Err(too_many());
                }
                hashes.push(Hash32::decode(d, ctx)?);
            },
        }
        Ok(hashes)
    }

    pub fn cbor_len<C>(v: &[Hash32], ctx: &mut C) -> usize {
        let len = v.len() as u64;
        v.iter().fold(<u64 as CborLen<C>>::cbor_len(&len, ctx), |total, hash| {
            total + hash.cbor_len(ctx)
        })
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Hash32>, D::Error>
    where D: Deserializer<'de> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Vec<Hash32>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "a sequence of at most {MAX_MMR_PROOF_HASHES} hashes")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where A: de::SeqAccess<'de> {
                let mut hashes = Vec::new();
                while let Some(hash) = seq.next_element()? {
                    if hashes.len() == MAX_MMR_PROOF_HASHES {
                        return Err(de::Error::invalid_length(MAX_MMR_PROOF_HASHES + 1, &self));
                    }
                    hashes.push(hash);
                }
                Ok(hashes)
            }
        }

        deserializer.deserialize_seq(Visitor)
    }
}

pub(crate) fn serialize_bytes<W: borsh::io::Write, T: AsRef<[u8]>>(
    obj: &T,
    writer: &mut W,
) -> Result<(), borsh::io::Error> {
    borsh::BorshSerialize::serialize(obj.as_ref(), writer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mmr_proof(path: usize, peaks: usize) -> MmrInclusionProof {
        MmrInclusionProof {
            leaf_index: 3,
            mmr_size: 7,
            path: vec![Hash32::from_array([1; 32]); path],
            peaks: vec![Hash32::from_array([2; 32]); peaks],
        }
    }

    #[test]
    fn bounded_vec_bytes_cbor_len_matches_the_encoding() {
        #[derive(minicbor::Encode, minicbor::CborLen)]
        struct Wrapper(
            #[n(0)]
            #[cbor(with = "super::bounded_vec_bytes")]
            bounded_vec::BoundedVec<u8, 1, 64>,
        );

        let value = Wrapper(bounded_vec::BoundedVec::<u8, 1, 64>::from_vec(vec![0xff; 64]).unwrap());
        assert_eq!(minicbor::len(&value), tari_bor::encode(&value).unwrap().len());
    }

    #[test]
    fn mmr_proof_round_trips_at_the_hash_bound() {
        let proof = mmr_proof(MAX_MMR_PROOF_HASHES, 0);
        let encoded = tari_bor::encode(&proof).unwrap();
        assert_eq!(tari_bor::decode::<MmrInclusionProof>(&encoded).unwrap(), proof);
        let json = serde_json::to_string(&proof).unwrap();
        assert_eq!(serde_json::from_str::<MmrInclusionProof>(&json).unwrap(), proof);
    }

    #[test]
    fn mmr_proof_rejects_too_many_hashes() {
        for proof in [
            mmr_proof(MAX_MMR_PROOF_HASHES + 1, 0),
            mmr_proof(0, MAX_MMR_PROOF_HASHES + 1),
        ] {
            let encoded = tari_bor::encode(&proof).unwrap();
            tari_bor::decode::<MmrInclusionProof>(&encoded).unwrap_err();
            let json = serde_json::to_string(&proof).unwrap();
            serde_json::from_str::<MmrInclusionProof>(&json).unwrap_err();
        }
    }

    #[test]
    fn mmr_proof_rejects_an_oversized_declared_length_without_allocating() {
        // [0, 0, array(2^63), ...]: the declared length alone must be rejected
        let mut encoded = vec![0x84, 0x00, 0x00, 0x9b];
        encoded.extend_from_slice(&(1u64 << 63).to_be_bytes());
        tari_bor::decode::<MmrInclusionProof>(&encoded).unwrap_err();
    }

    #[test]
    fn mmr_proof_rejects_an_unbounded_indefinite_array() {
        // [0, 0, [_ h, h, ...]] with one hash more than the bound and no break
        let mut encoded = vec![0x84, 0x00, 0x00, 0x9f];
        for _ in 0..=MAX_MMR_PROOF_HASHES {
            encoded.extend_from_slice(&[0x58, 0x20]);
            encoded.extend_from_slice(&[0; 32]);
        }
        tari_bor::decode::<MmrInclusionProof>(&encoded).unwrap_err();
    }

    #[test]
    fn mmr_proof_rejects_truncated_and_trailing_input() {
        let encoded = tari_bor::encode(&mmr_proof(2, 1)).unwrap();
        for len in 0..encoded.len() {
            tari_bor::decode::<MmrInclusionProof>(&encoded[..len]).unwrap_err();
        }
        let mut trailing = encoded;
        trailing.push(0);
        tari_bor::decode_exact::<MmrInclusionProof>(&trailing).unwrap_err();
    }
}
