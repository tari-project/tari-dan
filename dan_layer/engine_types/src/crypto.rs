//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

// TODO: I think we should move the engine implementations out of this crate.
//       The only reason we put vaults into engine types is because we return a substate value to the client.
//       Buckets are only in engine types because of the vault.
//       I think vaults/buckets should live in the engine since a lot of engine-related code lives in them (and then we
//       can reuse this function). We could use an interface to keep the type in engine types and implementation in the
//       engine. This refactor would need some co-ordination.

use lazy_static::lazy_static;
use tari_common_types::types::{BulletRangeProof, CommitmentFactory, PrivateKey};
use tari_crypto::{
    commitment::{ExtensionDegree, HomomorphicCommitmentFactory},
    errors::RangeProofError,
    extended_range_proof::ExtendedRangeProofService,
    ristretto::bulletproofs_plus::{BulletproofsPlusService, RistrettoExtendedMask, RistrettoExtendedWitness},
};
use tari_template_lib::models::{Amount, ConfidentialProof, ConfidentialStatement};
use tari_utilities::ByteArray;

lazy_static! {
    /// Static reference to the default commitment factory. Each instance of CommitmentFactory requires a number of heap allocations.
    static ref COMMITMENT_FACTORY: CommitmentFactory = CommitmentFactory::default();
    /// Static reference to the default range proof service. Each instance of RangeProofService requires a number of heap allocations.
    static ref RANGE_PROOF_AGG_1_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(64, 1, CommitmentFactory::default()).unwrap();
    static ref RANGE_PROOF_AGG_2_SERVICE: BulletproofsPlusService =
        BulletproofsPlusService::init(64, 2, CommitmentFactory::default()).unwrap();
}

pub fn range_proof_service(aggregation_factor: usize) -> &'static BulletproofsPlusService {
    match aggregation_factor {
        1 => &RANGE_PROOF_AGG_1_SERVICE,
        2 => &RANGE_PROOF_AGG_2_SERVICE,
        _ => panic!(
            "Unsupported BP aggregation factor {}. Expected 1 or 2",
            aggregation_factor
        ),
    }
}

pub fn commitment_factory() -> &'static CommitmentFactory {
    &COMMITMENT_FACTORY
}

pub struct ConfidentialProofStatement {
    pub amount: Amount,
    pub mask: PrivateKey,
    pub minimum_value_promise: u64,
}

impl ConfidentialProofStatement {
    pub fn zero() -> Self {
        Self {
            amount: Amount::zero(),
            mask: PrivateKey::default(),
            minimum_value_promise: 0,
        }
    }
}

pub fn generate_confidential_proof(
    output_statement: ConfidentialProofStatement,
    change_statement: Option<ConfidentialProofStatement>,
) -> Result<ConfidentialProof, RangeProofError> {
    let output_commitment = commitment_to_bytes(&output_statement.mask, output_statement.amount);
    let change_commitment = change_statement
        .as_ref()
        .map(|stmt| commitment_to_bytes(&stmt.mask, stmt.amount));
    let minimum_value_promise = output_statement.minimum_value_promise;
    let change_minimum_value_promise = change_statement
        .as_ref()
        .map(|stmt| stmt.minimum_value_promise)
        .unwrap_or(0);
    let output_range_proof = generate_extended_bullet_proof(output_statement, change_statement)?;

    Ok(ConfidentialProof {
        output_statement: ConfidentialStatement {
            commitment: output_commitment,
            minimum_value_promise,
        },
        change_statement: change_commitment.map(|commitment| ConfidentialStatement {
            commitment,
            minimum_value_promise: change_minimum_value_promise,
        }),
        range_proof: output_range_proof.0,
        revealed_amount: Amount::zero(),
    })
}

fn generate_extended_bullet_proof(
    output_statement: ConfidentialProofStatement,
    change_statement: Option<ConfidentialProofStatement>,
) -> Result<BulletRangeProof, RangeProofError> {
    let mut extended_witnesses = vec![];

    let extended_mask =
        RistrettoExtendedMask::assign(ExtensionDegree::DefaultPedersen, vec![output_statement.mask]).unwrap();

    let mut agg_factor = 1;
    extended_witnesses.push(RistrettoExtendedWitness {
        mask: extended_mask,
        value: output_statement.amount.value() as u64,
        minimum_value_promise: output_statement.minimum_value_promise,
    });
    if let Some(stmt) = change_statement {
        let extended_mask = RistrettoExtendedMask::assign(ExtensionDegree::DefaultPedersen, vec![stmt.mask]).unwrap();
        extended_witnesses.push(RistrettoExtendedWitness {
            mask: extended_mask,
            value: stmt.amount.value() as u64,
            minimum_value_promise: stmt.minimum_value_promise,
        });
        agg_factor = 2;
    }

    let output_range_proof = range_proof_service(agg_factor).construct_extended_proof(extended_witnesses, None)?;
    Ok(BulletRangeProof(output_range_proof))
}

fn commitment_to_bytes(mask: &PrivateKey, amount: Amount) -> [u8; 32] {
    let commitment = COMMITMENT_FACTORY.commit_value(mask, amount.value() as u64);
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(commitment.as_bytes());
    bytes
}

pub mod challenges {
    use tari_common_types::types::{Commitment, PublicKey};
    use tari_template_lib::Hash;

    use crate::hashing::{hasher, EngineHashDomainLabel};

    pub fn confidential_commitment_proof(
        public_key: &PublicKey,
        public_nonce: &PublicKey,
        commitment: &Commitment,
    ) -> Hash {
        hasher(EngineHashDomainLabel::ConfidentialProof)
            .chain(&public_key)
            .chain(&public_nonce)
            .chain(commitment.as_public_key())
            .result()
    }

    pub fn confidential_withdraw(excess: &PublicKey, public_nonce: &PublicKey) -> Hash {
        hasher(EngineHashDomainLabel::ConfidentialTransfer)
            .chain(excess)
            .chain(public_nonce)
            .result()
    }
}
