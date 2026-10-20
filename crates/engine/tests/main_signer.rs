//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_ootle_transaction::{
    MainSigner,
    Transaction,
    TransactionBuilder,
    TransactionSignature,
    args,
    builder::MainIntent,
};
use tari_template_lib::types::{NonFungibleAddress, OwnerRule, SubstateOwnerRule, crypto::RistrettoPublicKeyBytes};
use tari_template_test_tooling::{TemplateTest, support::assert_error::assert_reject_reason};
use tari_utilities::hex::from_hex;

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
const TEMPLATE_PATHS: &[&str] = &["tests/templates/caller_context"];
const TEMPLATE_NAME: &str = "CallerContextTest";

/// Signs `builder` with each of `signers` in order and seals it with the test's key, which the transaction does not
/// authorize.
fn co_signed(
    test: &TemplateTest,
    builder: TransactionBuilder<MainIntent>,
    main_signer: MainSigner,
    signers: &[&RistrettoSecretKey],
) -> Transaction {
    let seal_public_key = test.to_public_key_bytes();
    let unsigned = builder.with_main_signer(main_signer).build_unsigned();
    let signatures = signers
        .iter()
        .map(|secret| TransactionSignature::sign(secret, &seal_public_key, &unsigned))
        .collect();
    unsigned.with_signatures(signatures).seal(test.secret_key())
}

fn signer_proofs(transaction: &Transaction) -> Vec<NonFungibleAddress> {
    transaction
        .signatures()
        .iter()
        .map(|sig| NonFungibleAddress::from_public_key(*sig.public_key()))
        .collect()
}

#[test]
fn the_declared_authorizer_is_the_main_signer_in_any_signature_position() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let (main_secret, main_public) = test.new_key_pair(2);
    let (other_secret, _) = test.new_key_pair(3);
    let main_public = main_public.to_byte_type();

    let transaction = co_signed(
        &test,
        test.transaction()
            .call_function(template, "log_caller_pk", args![])
            .call_function(template, "main_signer_proof", args![])
            .put_last_instruction_output_on_workspace("proof")
            .call_function(template, "check_pk_proof", args![Workspace("proof"), main_public])
            .drop_all_proofs_in_workspace(),
        MainSigner::Authorizer(main_public),
        &[&other_secret, &main_secret],
    );
    assert!(transaction.verify_all_signatures());
    let proofs = signer_proofs(&transaction);
    let result = test.execute_expect_success(transaction, proofs);

    let logged = &result.finalize.logs[0].message;
    assert_eq!(from_hex(logged).unwrap(), main_public.as_bytes());
}

#[test]
fn owned_by_signer_assigns_the_declared_authorizer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (main_secret, main_public) = test.new_key_pair(2);
    let (other_secret, other_public) = test.new_key_pair(3);
    let main_public = main_public.to_byte_type();
    let other_public: RistrettoPublicKeyBytes = other_public.to_byte_type();

    let transaction = co_signed(
        &test,
        test.transaction()
            .create_account_custom(main_public, Some(OwnerRule::OwnedBySigner), None, None::<String>),
        MainSigner::Authorizer(main_public),
        &[&other_secret, &main_secret],
    );
    let proofs = signer_proofs(&transaction);
    let result = test.execute_expect_success(transaction, proofs);

    let diff = result.finalize.result.accept().unwrap();
    let (_, account) = diff
        .up_iter()
        .find(|(id, _)| id.is_component())
        .expect("the account component");
    let owner = account.substate_value().component().unwrap().owner_rule();
    assert_eq!(*owner, SubstateOwnerRule::ByPublicKey(main_public));
    assert_ne!(*owner, SubstateOwnerRule::ByPublicKey(other_public));
}

#[test]
fn owned_by_signer_aborts_without_a_main_signer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let (main_secret, main_public) = test.new_key_pair(2);
    let (other_secret, _) = test.new_key_pair(3);
    let main_public = main_public.to_byte_type();

    let transaction = co_signed(
        &test,
        test.transaction()
            .create_account_custom(main_public, Some(OwnerRule::OwnedBySigner), None, None::<String>),
        MainSigner::None,
        &[&other_secret, &main_secret],
    );
    let proofs = signer_proofs(&transaction);
    let reason = test.execute_expect_failure(transaction, proofs);

    assert_reject_reason(reason, "OwnerRule::OwnedBySigner needs the transaction's main signer");
}

#[test]
fn the_signer_public_key_aborts_without_a_main_signer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let (secret, _) = test.new_key_pair(2);

    let transaction = co_signed(
        &test,
        test.transaction().call_function(template, "log_caller_pk", args![]),
        MainSigner::None,
        &[&secret],
    );
    let proofs = signer_proofs(&transaction);
    let reason = test.execute_expect_failure(transaction, proofs);

    assert_reject_reason(
        reason,
        "CallerContext::transaction_signer_public_key needs the transaction's main signer",
    );
}

#[test]
fn a_main_signer_that_did_not_sign_is_not_the_main_signer() {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template = test.get_template_address(TEMPLATE_NAME);
    let (_, absent_public) = test.new_key_pair(2);
    let (secret, _) = test.new_key_pair(3);

    let transaction = co_signed(
        &test,
        test.transaction().call_function(template, "log_caller_pk", args![]),
        MainSigner::Authorizer(absent_public.to_byte_type()),
        &[&secret],
    );
    assert!(transaction.verify_all_signatures());
    assert_eq!(transaction.main_signer(), None);
    let proofs = signer_proofs(&transaction);
    let reason = test.execute_expect_failure(transaction, proofs);

    assert_reject_reason(
        reason,
        "CallerContext::transaction_signer_public_key needs the transaction's main signer",
    );
}
