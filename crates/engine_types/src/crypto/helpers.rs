//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use lazy_static::lazy_static;
use ootle_byte_type::FromByteType;
use tari_crypto::{
    commitment::HomomorphicCommitmentFactory,
    ristretto::{
        RistrettoSecretKey,
        bulletproofs_plus::BulletproofsPlusService,
        pedersen::{PedersenCommitment, extended_commitment_factory::ExtendedPedersenCommitmentFactory},
    },
    tari_utilities::ByteArray,
};
use tari_template_lib::types::{Amount, crypto::SchnorrSignatureBytes};

use crate::hashing::EngineSchnorrSignature;

// Note that the BP-plus implementation currently does not support bit lengths over 64
const BP_BIT_LENGTH: usize = u64::BITS as usize;

/// Largest bulletproof aggregation factor a statement may use. Must be a power of two and must have a matching
/// service below, since a proof over `n` commitments is verified at `n.next_power_of_two()`.
pub const MAX_LAZY_BP_AGG_FACTORS: usize = 16;

lazy_static! {
    /// Static reference to the default commitment factory. Each instance of CommitmentFactory requires a number of heap allocations.
    static ref COMMITMENT_FACTORY: ExtendedPedersenCommitmentFactory = ExtendedPedersenCommitmentFactory::default();
    /// Static reference to the default range proof service. Each instance of RangeProofService requires a number of heap allocations.
    static ref RANGE_PROOF_AGG_1_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(BP_BIT_LENGTH, 1, ExtendedPedersenCommitmentFactory::default()).unwrap();
    static ref RANGE_PROOF_AGG_2_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(BP_BIT_LENGTH, 2, ExtendedPedersenCommitmentFactory::default()).unwrap();
    static ref RANGE_PROOF_AGG_4_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(BP_BIT_LENGTH, 4, ExtendedPedersenCommitmentFactory::default()).unwrap();
    static ref RANGE_PROOF_AGG_8_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(BP_BIT_LENGTH, 8, ExtendedPedersenCommitmentFactory::default()).unwrap();
    static ref RANGE_PROOF_AGG_16_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(BP_BIT_LENGTH, 16, ExtendedPedersenCommitmentFactory::default()).unwrap();
}

pub fn get_static_range_proof_service(aggregation_factor: usize) -> &'static BulletproofsPlusService {
    match aggregation_factor.next_power_of_two() {
        1 => &RANGE_PROOF_AGG_1_SERVICE,
        2 => &RANGE_PROOF_AGG_2_SERVICE,
        4 => &RANGE_PROOF_AGG_4_SERVICE,
        8 => &RANGE_PROOF_AGG_8_SERVICE,
        16 => &RANGE_PROOF_AGG_16_SERVICE,
        _ => panic!(
            "Unsupported BP aggregation factor {}. Expected 1/2/4/8 or 16",
            aggregation_factor
        ),
    }
}

pub fn get_commitment_factory() -> &'static ExtendedPedersenCommitmentFactory {
    &COMMITMENT_FACTORY
}

/// Creates a Pedersen commitment to the given amount using the provided mask.
/// This construction does not check that the amount is within any specific range for bulletproofs+.
/// however is still a completely valid Pedersen commitment (256-bit vs 128-bit amount).
pub fn commit_amount_unchecked(mask: &RistrettoSecretKey, amount: Amount) -> PedersenCommitment {
    let v = convert_amount_to_secret(&amount);
    get_commitment_factory().commit(mask, &v)
}

/// Creates a Pedersen commitment to the given amount using the provided mask.
///
/// # Returns
///
/// Returns `None` if the amount exceeds `u64::MAX`, otherwise returns a `PedersenCommitment`.
/// This restriction is due to the underlying Bulletproofs+ implementation only supporting 64-bit range proofs.
pub fn commit_amount(mask: &RistrettoSecretKey, amount: Amount) -> Option<PedersenCommitment> {
    if amount > u64::MAX {
        return None;
    }

    Some(commit_amount_unchecked(mask, amount))
}

/// Creates a Pedersen commitment to the given u64 amount using the provided mask.
pub fn commit_u64_amount(mask: &RistrettoSecretKey, amount: u64) -> PedersenCommitment {
    get_commitment_factory().commit_value(mask, amount)
}

/// Converts a `Amount` to a `RistrettoSecretKey`.
pub fn convert_amount_to_secret(amount: &Amount) -> RistrettoSecretKey {
    let mut val_bytes = [0u8; 32];
    val_bytes[..Amount::BYTE_SIZE].copy_from_slice(&amount.to_le_bytes());
    RistrettoSecretKey::from_canonical_bytes(&val_bytes)
        .expect("MSB in 256-bit integer is always zero and < ell (Ristretto base point) therefore canonical")
}

pub fn try_decode_to_signature(signature: &SchnorrSignatureBytes) -> Option<EngineSchnorrSignature> {
    signature.try_from_byte_type().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homomorphic() {
        let amount1 = Amount::MAX;
        let commitment1 = commit_amount_unchecked(&Default::default(), amount1);
        let amount2 = Amount::from_u64(1_000);
        let commitment2 = commit_amount_unchecked(&Default::default(), amount2);

        let resulting_commitment = commitment1.as_public_key() - commitment2.as_public_key();

        let check = commit_amount_unchecked(&Default::default(), amount1 - amount2);
        assert_eq!(resulting_commitment, *check.as_public_key());
    }

    #[test]
    fn endianness() {
        // Check that the endianness used in RistrettoSecretKey (dalek Scalar) is the same as
        // convert_amount_to_secret i.e. Little-Endian.
        let amount = Amount::from(199999999999999999u128);
        let v1 = convert_amount_to_secret(&amount);

        assert_eq!(v1.as_bytes()[..Amount::BYTE_SIZE], amount.to_le_bytes());

        let v2 = RistrettoSecretKey::from(1000u64);
        let sub = Amount::from_le_slice((v1 - v2).as_bytes()).unwrap();
        let expected = amount - Amount::from_u64(1000);
        assert_eq!(sub, expected);
    }
}
