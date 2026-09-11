//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine::state_store::StateWriter;
use tari_engine_types::{
    ValidatorFeePool,
    substate::{Substate, SubstateId},
};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::ValidatorFeePoolAddress;
use tari_template_test_tooling::TemplateTest;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn test_claim_validator_fees_up_to() {
    let mut test = TemplateTest::new(CRATE_PATH, std::iter::empty::<&str>());
    let (account, _token, private_key) = test.create_funded_account();

    use ootle_byte_type::ToByteType;
    use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
    let public_key = RistrettoPublicKey::from_secret_key(&private_key);
    let pk: tari_template_lib::types::crypto::RistrettoPublicKeyBytes = public_key.to_byte_type();
    let addr = ValidatorFeePoolAddress::from_array(pk.into_array());

    // Setup an initial fee pool with 100 TARI
    let initial_pool = ValidatorFeePool::new(pk, 100);
    test.get_state_store_mut()
        .set_state(SubstateId::ValidatorFeePool(addr), Substate::new(0, initial_pool))
        .unwrap();

    // 1. Claim up to 60 TARI
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .claim_validator_fees_up_to(addr, 60u64)
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .build_and_seal(&private_key),
        vec![],
    );

    // Verify amount is 60 and the pool surplus is 40
    let pool_state = test
        .read_only_state_store()
        .get_substate(&SubstateId::ValidatorFeePool(addr))
        .unwrap();
    let pool = pool_state.substate_value().as_validator_fee_pool().unwrap();
    assert_eq!(pool.amount(), 40);

    // To check ValidatorFeeWithdrawal, we can look at the FinalizeResult's fee receipt? No, it's not exposed publicly
    // or it's not a field. Let's just rely on the substate balance assertion and the successful execution.

    // 2. Claim all remaining (up to 1000)
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .claim_validator_fees_up_to(addr, 1000u64)
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Workspace("bucket")])
            .build_and_seal(&private_key),
        vec![],
    );

    let result = test
        .read_only_state_store()
        .get_substate(&SubstateId::ValidatorFeePool(addr));
    assert!(result.is_err(), "ValidatorFeePool should be destroyed when empty");
}

/// A claim must release the fee pool's lock, so a second claim against the same pool in the same transaction can
/// take it.
#[test]
fn test_two_claims_against_one_pool_in_a_transaction() {
    use ootle_byte_type::ToByteType;
    use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};

    let mut test = TemplateTest::new(CRATE_PATH, std::iter::empty::<&str>());
    let (account, _token, private_key) = test.create_funded_account();

    let public_key = RistrettoPublicKey::from_secret_key(&private_key);
    let pk: tari_template_lib::types::crypto::RistrettoPublicKeyBytes = public_key.to_byte_type();
    let addr = ValidatorFeePoolAddress::from_array(pk.into_array());

    test.get_state_store_mut()
        .set_state(
            SubstateId::ValidatorFeePool(addr),
            Substate::new(0, ValidatorFeePool::new(pk, 100)),
        )
        .unwrap();

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .claim_validator_fees_up_to(addr, 60u64)
            .put_last_instruction_output_on_workspace("first")
            .call_method(account, "deposit", args![Workspace("first")])
            .claim_validator_fees_up_to(addr, 40u64)
            .put_last_instruction_output_on_workspace("second")
            .call_method(account, "deposit", args![Workspace("second")])
            .build_and_seal(&private_key),
        vec![],
    );

    let result = test
        .read_only_state_store()
        .get_substate(&SubstateId::ValidatorFeePool(addr));
    assert!(result.is_err(), "both claims emptied the pool, so it is destroyed");
}
