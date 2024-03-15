//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use lazy_static::lazy_static;
use tari_common_types::types::CommitmentFactory;
use tari_crypto::ristretto::bulletproofs_plus::BulletproofsPlusService;

lazy_static! {
    /// Static reference to the default commitment factory. Each instance of CommitmentFactory requires a number of heap allocations.
    static ref COMMITMENT_FACTORY: CommitmentFactory = CommitmentFactory::default();
    /// Static reference to the default range proof service. Each instance of RangeProofService requires a number of heap allocations.
    static ref RANGE_PROOF_AGG_1_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(64, 1, CommitmentFactory::default()).unwrap();
    static ref RANGE_PROOF_AGG_2_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(64, 2, CommitmentFactory::default()).unwrap();
}

pub fn get_range_proof_service(aggregation_factor: usize) -> &'static BulletproofsPlusService {
    match aggregation_factor {
        1 => &RANGE_PROOF_AGG_1_SERVICE,
        2 => &RANGE_PROOF_AGG_2_SERVICE,
        _ => panic!(
            "Unsupported BP aggregation factor {}. Expected 1 or 2",
            aggregation_factor
        ),
    }
}

pub fn get_commitment_factory() -> &'static CommitmentFactory {
    &COMMITMENT_FACTORY
}

pub mod challenges {
    use tari_common_types::types::{Commitment, PublicKey};
    use tari_template_lib::{
        models::{Amount, ViewableBalanceProofChallengeFields},
        Hash,
    };

    use crate::hashing::{hasher32, hasher64, EngineHashDomainLabel};

    pub fn confidential_commitment_proof64(
        public_key: &PublicKey,
        public_nonce: &PublicKey,
        commitment: &Commitment,
    ) -> [u8; 64] {
        hasher64(EngineHashDomainLabel::ConfidentialProof)
            .chain(&public_key)
            .chain(&public_nonce)
            .chain(commitment.as_public_key())
            .result()
    }

    pub fn confidential_withdraw64(
        excess: &PublicKey,
        public_nonce: &PublicKey,
        input_revealed_amount: Amount,
        output_revealed_amount: Amount,
    ) -> [u8; 64] {
        hasher64(EngineHashDomainLabel::ConfidentialTransfer)
            .chain(excess)
            .chain(public_nonce)
            .chain(&input_revealed_amount)
            .chain(&output_revealed_amount)
            .result()
    }

    pub fn viewable_balance_proof_challenge64(
        commitment: &Commitment,
        view_key: &PublicKey,
        challenge_fields: ViewableBalanceProofChallengeFields<'_>,
    ) -> [u8; 64] {
        hasher64(EngineHashDomainLabel::ViewKey)
            .chain(commitment)
            .chain(view_key)
            .chain(&challenge_fields)
            .result()
    }

    pub fn confidential_commitment_proof32(
        public_key: &PublicKey,
        public_nonce: &PublicKey,
        commitment: &Commitment,
    ) -> Hash {
        hasher32(EngineHashDomainLabel::ConfidentialProof)
            .chain(&public_key)
            .chain(&public_nonce)
            .chain(commitment.as_public_key())
            .result()
    }

    pub fn confidential_withdraw32(excess: &PublicKey, public_nonce: &PublicKey, revealed_amount: Amount) -> Hash {
        hasher32(EngineHashDomainLabel::ConfidentialTransfer)
            .chain(excess)
            .chain(public_nonce)
            .chain(&revealed_amount)
            .result()
    }
}
