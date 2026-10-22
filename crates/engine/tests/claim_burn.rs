//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey as _,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine::traits::{ClaimProofError, ClaimProofRejection, ClaimProofVerifier, VerifiedClaim};
use tari_engine_types::{
    commit_result::ExecutionFailureCode,
    confidential::{
        BurnOutput,
        BurnOutputFeatures,
        BurnOutputInclusionProof,
        ClaimBurnOutputData,
        MinotariBurnClaimProof,
        MmrInclusionProof,
    },
};
use tari_ootle_transaction::{Epoch, Transaction};
use tari_template_lib::types::{
    EncryptedData,
    Hash32,
    constants::TARI_TOKEN,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, SchnorrSignatureBytes},
};
use tari_template_test_tooling::{TemplateTest, support::stealth, wallet_crypto::MaskAndValue};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

struct FixedOutcomeVerifier(ClaimProofError);

impl ClaimProofVerifier for FixedOutcomeVerifier {
    fn verify_claim_proof(
        &self,
        _epoch: Epoch,
        _claim: &MinotariBurnClaimProof,
    ) -> Result<VerifiedClaim, ClaimProofError> {
        Err(self.0.clone())
    }
}

/// Accepts every claim, as a claim whose ownership proof and output inclusion verify
struct AcceptingVerifier;

impl ClaimProofVerifier for AcceptingVerifier {
    fn verify_claim_proof(
        &self,
        _epoch: Epoch,
        claim: &MinotariBurnClaimProof,
    ) -> Result<VerifiedClaim, ClaimProofError> {
        Ok(VerifiedClaim {
            claim_public_key: claim.output.features.claim_public_key,
        })
    }
}

fn keypair() -> (RistrettoSecretKey, RistrettoPublicKeyBytes) {
    let (secret, public) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    (secret, public.to_byte_type())
}

/// A claim of a burn output made out to `claim_public_key`
fn claim_to(claim_public_key: RistrettoPublicKeyBytes) -> (MinotariBurnClaimProof, ClaimBurnOutputData) {
    let (mut proof, output_data) = claim();
    proof.output.features.claim_public_key = claim_public_key;
    (proof, output_data)
}

fn one_byte<const N: usize>() -> bounded_vec::BoundedVec<u8, 1, N> {
    bounded_vec::BoundedVec::<u8, 1, N>::from_vec(vec![0]).unwrap()
}

fn claim() -> (MinotariBurnClaimProof, ClaimBurnOutputData) {
    let mmr_proof = || MmrInclusionProof {
        leaf_index: 0,
        mmr_size: 1,
        path: vec![],
        peaks: vec![],
    };
    let proof = MinotariBurnClaimProof {
        commitment: PedersenCommitmentBytes::zero(),
        ownership_proof: SchnorrSignatureBytes::zero(),
        value: 1_000,
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
    };
    let output_data = ClaimBurnOutputData {
        encrypted_data: EncryptedData::empty(),
    };
    (proof, output_data)
}

fn verifier_fault() -> FixedOutcomeVerifier {
    FixedOutcomeVerifier(ClaimProofError::VerifierFault("database unavailable".to_string()))
}

#[test]
fn a_verifier_fault_in_the_fee_intent_aborts_execution() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    test.set_claim_proof_verifier(verifier_fault());
    let (proof, output_data) = claim();

    let err = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .with_fee_instructions_builder(|builder| builder.claim_burn(proof, output_data))
                .build_and_seal(test.secret_key()),
            vec![],
        )
        .expect_err("a verifier fault must not produce a receipt");
    assert!(err.is_node_fault(), "expected a node fault, got {err}");
}

#[test]
fn a_verifier_fault_in_the_main_intent_aborts_execution() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    test.set_claim_proof_verifier(verifier_fault());
    let (proof, output_data) = claim();

    let err = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .claim_burn(proof, output_data)
                .build_and_seal(test.secret_key()),
            vec![],
        )
        .expect_err("a verifier fault must not produce a receipt");
    assert!(err.is_node_fault(), "expected a node fault, got {err}");
}

#[test]
fn an_invalid_claim_is_rejected_with_a_receipt() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    test.set_claim_proof_verifier(FixedOutcomeVerifier(
        ClaimProofRejection::Invalid("bad proof".to_string()).into(),
    ));
    let (proof, output_data) = claim();

    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .claim_burn(proof, output_data)
                .build_and_seal(test.secret_key()),
            vec![],
        )
        .expect("an invalid claim is a verdict on the transaction");
    let reason = result.finalize.any_reject().expect("an invalid claim is rejected");
    assert_eq!(
        reason.execution_failure_code(),
        Some(ExecutionFailureCode::InvalidProof),
        "unexpected reject: {reason}"
    );
}

#[test]
fn a_claim_sealed_by_the_claim_key_is_accepted() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    test.set_claim_proof_verifier(AcceptingVerifier);
    let (claim_secret, claim_public_key) = keypair();
    let (proof, output_data) = claim_to(claim_public_key);

    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .claim_burn(proof, output_data)
                .build_and_seal(&claim_secret),
            vec![],
        )
        .unwrap();
    if let Some(reason) = result.finalize.any_reject() {
        panic!("a claim sealed by the claim key must be accepted: {reason}");
    }
}

#[test]
fn a_claim_co_signed_by_the_claim_key_is_accepted() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    test.set_claim_proof_verifier(AcceptingVerifier);
    let (claim_secret, claim_public_key) = keypair();
    let (proof, output_data) = claim_to(claim_public_key);

    // Another key, e.g. a relayer paying the fee, seals
    let result = test
        .try_execute(
            Transaction::builder_localnet(Epoch(1))
                .claim_burn(proof, output_data)
                .add_signer(&test.to_public_key_bytes(), &claim_secret)
                .seal(test.secret_key()),
            vec![],
        )
        .unwrap();
    if let Some(reason) = result.finalize.any_reject() {
        panic!("a claim co-signed by the claim key must be accepted: {reason}");
    }
}

#[test]
fn a_claim_sealed_by_another_key_is_spendable_by_the_claim_key_alone() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    test.set_claim_proof_verifier(AcceptingVerifier);
    let (claim_secret, claim_public_key) = keypair();
    let (relayer_secret, relayer_public_key) = keypair();
    let opening = MaskAndValue {
        mask: keypair().0,
        value: 1_000,
    };
    let (mut proof, output_data) = claim_to(claim_public_key);
    proof.commitment = opening.to_commitment().to_byte_type();
    proof.value = opening.value;

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .claim_burn(proof, output_data)
            .add_signer(&relayer_public_key, &claim_secret)
            .seal(&relayer_secret),
        vec![],
    );
    let diff = result.finalize.any_accept().unwrap();
    let spend_key = diff
        .up_iter()
        .find_map(|(_, substate)| substate.substate_value().as_utxo())
        .expect("claim_burn mints a UTXO")
        .spender_public_key()
        .copied();
    assert_eq!(spend_key, Some(claim_public_key));

    let transfer = stealth::generate_transfer_data([opening.clone()], 0u64, [opening.value], 0u64);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .stealth_transfer(TARI_TOKEN, transfer.statement)
            .build_and_seal(&claim_secret),
        vec![],
    );
}

#[test]
fn the_burner_cannot_claim_a_burn_made_out_to_someone_else() {
    let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
    // The burner knows the commitment opening, so its ownership proof verifies
    test.set_claim_proof_verifier(AcceptingVerifier);
    let (_recipient_secret, recipient_public_key) = keypair();
    let (burner_secret, burner_public_key) = keypair();
    let (proof, output_data) = claim_to(recipient_public_key);

    for transaction in [
        Transaction::builder_localnet(Epoch(1))
            .claim_burn(proof.clone(), output_data.clone())
            .build_and_seal(&burner_secret),
        Transaction::builder_localnet(Epoch(1))
            .claim_burn(proof, output_data)
            .add_signer(&burner_public_key, test.secret_key())
            .seal(&burner_secret),
    ] {
        let result = test
            .try_execute(transaction, vec![])
            .expect("an unauthorized claim is a verdict on the transaction");
        let reason = result
            .finalize
            .any_reject()
            .expect("a claim without the claim key's signature must be rejected");
        assert_eq!(
            reason.execution_failure_code(),
            Some(ExecutionFailureCode::AccessDenied),
            "unexpected reject: {reason}"
        );
    }
}

/// Holds `ClaimBurnShape::estimate_fee` to what the engine charges the claim a wallet builds.
mod fee_estimate {
    use tari_crypto::keys::SecretKey as _;
    use tari_engine::fees::FeeTable;
    use tari_engine_types::{
        fees::{FeeRates, FeeSource},
        stealth::{ClaimBurnShape, persisted_utxo_bytes},
    };
    use tari_template_lib::types::{Amount, constants::TARI_TOKEN, stealth::StealthTransferStatement};
    use tari_template_test_tooling::{support::stealth, wallet_crypto::MaskAndValue};

    use super::*;

    /// What the burn being claimed is worth. Comfortably above any claim's fee, so the fee is never
    /// the binding constraint on it.
    const CLAIMED_VALUE: u64 = 10_000_000;

    /// The burn every shipped network is configured with: a share of the payment, so it must move
    /// nothing the estimate prices.
    const BURN_RATE_BPS: u16 = 500;

    /// The margin the estimate may carry over the real charge. The tombstone's value and the
    /// receipt's epoch are priced at their widest encodings, which leaves a handful of microtari, and
    /// a claim cannot get back what it reveals over the charge.
    const MAX_OVERSHOOT: u64 = 12;

    /// Stands in for the fee while the claim is built only to be weighed.
    const PLACEHOLDER_FEE: u64 = 1_000_000;

    fn setup() -> TemplateTest {
        let mut test = TemplateTest::new(CRATE_PATH, &[] as &[&str]);
        test.set_claim_proof_verifier(AcceptingVerifier);
        // Price a created substate as the shipped tables do, so the slots the claim creates carry the
        // weight they do on a real network.
        test.set_fee_table(FeeTable {
            per_substate_create_cost: 250,
            ..test.fee_table().clone()
        });
        test.enable_fees();
        test.set_burn_rate_bps(BURN_RATE_BPS);
        test
    }

    fn rates(test: &TemplateTest) -> FeeRates {
        test.fee_table().to_rates()
    }

    fn new_claim() -> MaskAndValue {
        MaskAndValue::new(CLAIMED_VALUE, RistrettoSecretKey::random(&mut rand::rng()))
    }

    struct Build {
        transaction: Transaction,
        statement: StealthTransferStatement,
    }

    /// Builds the transaction a wallet builds to claim a burn: claim it, spend the claimed UTXO into
    /// one stealth output revealing `revealed_fee`, and pay the revealed bucket as the fee. The claim
    /// key seals, so the claim, the claimed UTXO and the revealed output all answer to one signer.
    fn build(test: &TemplateTest, claimed: &MaskAndValue, revealed_fee: u64) -> Build {
        let (mut proof, output_data) = claim_to(test.to_public_key_bytes());
        proof.commitment = claimed.to_commitment().to_byte_type();
        proof.value = claimed.value;

        let transfer = stealth::generate_transfer_data(
            [claimed.clone()],
            Amount::zero(),
            [claimed.value - revealed_fee],
            Amount::from(revealed_fee),
        );
        let transaction = Transaction::builder_localnet(Epoch(1))
            .with_fee_instructions_builder(|builder| {
                builder
                    .claim_burn(proof, output_data)
                    .stealth_transfer(TARI_TOKEN, transfer.statement.clone())
                    .put_last_instruction_output_on_workspace("fee")
                    .pay_fee_from_bucket("fee")
            })
            .build_and_seal(test.secret_key());
        Build {
            transaction,
            statement: transfer.statement,
        }
    }

    /// The groups beyond the first that a claim's declared inputs are taken to be pledged to: the TARI
    /// resource and the claim's tombstone in two groups.
    const INPUT_FANOUT: u32 = 1;

    fn shape_of(test: &TemplateTest, build: &Build) -> ClaimBurnShape {
        ClaimBurnShape {
            persisted_output_bytes: build
                .statement
                .outputs_statement
                .outputs
                .iter()
                .map(persisted_utxo_bytes)
                .sum(),
            transaction_weight: build.transaction.calculate_transaction_weight().as_u64(),
            input_bytes: test.declared_input_bytes(&build.transaction),
            input_fanout: INPUT_FANOUT,
        }
    }

    #[test]
    fn the_estimate_bounds_what_the_engine_charges_a_claim() {
        let mut test = setup();
        test.set_input_fanout(INPUT_FANOUT);
        let claimed = new_claim();

        let shape = shape_of(&test, &build(&test, &claimed, PLACEHOLDER_FEE));
        let settled = build(&test, &claimed, shape.estimate_fee(&rates(&test)));
        assert_eq!(
            shape_of(&test, &settled),
            shape,
            "the fee a claim reveals must not move the shape it is priced at"
        );

        // A bucket-paid fee is not refunded, so the claim commits only if the estimate covered the
        // charge, and whatever it did not need is recorded as the overcharge.
        let result = test.execute_expect_success(settled.transaction, vec![]);
        let receipt = result.finalize.fee_receipt;
        assert!(
            receipt.is_paid_in_full(),
            "{shape:?} left {} unpaid",
            receipt.unpaid_debt()
        );
        let overshoot = receipt.total_fee_overcharge();
        assert!(
            overshoot <= MAX_OVERSHOOT,
            "{shape:?} was charged {} and overpaid {overshoot}",
            receipt.total_fees_charged(),
        );
    }

    /// The estimate's host-call count is a constant standing in for an instruction sequence it cannot
    /// see. Reading the charge back at a known per-call rate pins it to what the engine counted.
    #[test]
    fn the_runtime_call_count_matches_the_claim_sequence() {
        let mut test = setup();
        let mut fee_table = FeeTable::zero_rated();
        fee_table.per_module_call_cost = 1;
        test.set_fee_table(fee_table);
        test.set_burn_rate_bps(0);

        let result = test.execute_expect_success(build(&test, &new_claim(), PLACEHOLDER_FEE).transaction, vec![]);
        assert_eq!(
            result.finalize.fee_receipt.fee_breakdown().get(FeeSource::RuntimeCall),
            ClaimBurnShape::RUNTIME_CALLS,
        );
    }

    /// The bound rests on an underpaid claim being rejected. Revealing half the estimate shows that it
    /// is, so the bound is not passing vacuously.
    #[test]
    fn revealing_under_the_charge_rejects_the_claim() {
        let mut test = setup();
        let claimed = new_claim();
        let shape = shape_of(&test, &build(&test, &claimed, PLACEHOLDER_FEE));
        let too_little = shape.estimate_fee(&rates(&test)) / 2;

        let result = test
            .try_execute(build(&test, &claimed, too_little).transaction, vec![])
            .unwrap();
        assert!(
            result.finalize.fee_receipt.unpaid_debt() > 0,
            "revealing {too_little} should have left a debt"
        );
        result.expect_failure();
    }
}
