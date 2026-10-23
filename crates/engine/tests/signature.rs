//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey as _,
    ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
};
use tari_ootle_common_types::{
    RistrettoSchnorrBlake2bVerifier,
    crypto::create_key_pair_from_seed,
    substate_type::SubstateType,
};
use tari_ootle_sr25519::SigningContext;
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    amount,
    crypto::{NoSignatureDomain, PublicKey, Signature},
};
use tari_template_test_tooling::{
    TemplateTest,
    support::{assert_error::assert_reject_reason, stealth, stealth::NO_INPUTS},
};

const TEMPLATE_PATHS: &[&str] = &["tests/templates/signature"];
const TEMPLATE_NAME: &str = "SignatureTest";
const MESSAGE: &[u8] = b"Some message that binds to something important";
const INITIAL_SUPPLY: Amount = amount!(1000000000000000000000);
const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

const TEST_DOMAIN: &[u8] = b"tari.test.signature domain for tests";
fn sign_it(secret: &RistrettoSecretKey) -> Signature<NoSignatureDomain> {
    sign_it_with(secret, MESSAGE)
}

fn sign_it_with(secret: &RistrettoSecretKey, message: &[u8]) -> Signature<NoSignatureDomain> {
    let (nonce, nonce_pub) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let public_key = RistrettoPublicKey::from_secret_key(secret);
    let challenge = RistrettoSchnorrBlake2bVerifier::compute_challenge(
        TEST_DOMAIN,
        message,
        &public_key.to_byte_type(),
        &nonce_pub.to_byte_type(),
    );
    let sig = RistrettoSchnorr::sign_raw_uniform(secret, nonce, &challenge).unwrap();
    sig.to_byte_type().into()
}

/// A standard sr25519 signature under the template's signature domain as the signing context.
fn sign_sr25519(secret: &RistrettoSecretKey, domain: &[u8], message: &[u8]) -> Signature<NoSignatureDomain> {
    SigningContext::new(domain).sign(secret, message).into()
}

fn setup(allow_list: Vec<PublicKey>) -> (TemplateTest, ComponentAddress) {
    let mut test = TemplateTest::new(CRATE_PATH, TEMPLATE_PATHS);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let transaction = Transaction::builder_localnet(Epoch(1))
        .call_function(template_addr, "new", args![allow_list])
        .build_and_seal(test.secret_key());

    test.execute_expect_success(transaction, vec![]);

    let faucet = test.get_previous_output_address(SubstateType::Component);

    (test, faucet.as_component_address().unwrap())
}

#[test]
fn claim_with_valid_signature() {
    let (s1, p1) = create_key_pair_from_seed(1);
    let p1 = PublicKey::from(p1.to_byte_type());
    let (mut test, faucet) = setup(vec![p1]);

    let vault_id = test
        .get_previous_output_address(SubstateType::Vault)
        .as_vault_id()
        .unwrap();

    let transfer = stealth::generate_transfer_data(NO_INPUTS, 1_000_000_000_000u64, Some(1_000_000_000_000), 0);
    let signature = sign_it(&s1);
    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "claim_funds", args![p1, signature, transfer.statement])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    let diff = result.finalize.any_accept().unwrap();
    let utxos = diff
        .up_iter()
        .filter_map(|(_, substate)| substate.substate_value().as_utxo())
        .collect::<Vec<_>>();
    assert_eq!(utxos.len(), 1);
    assert!(utxos[0].output().is_some());
    let vault = test.read_only_state_store().get_vault(&vault_id).unwrap();
    assert_eq!(vault.balance(), INITIAL_SUPPLY - amount!(1000000000000));
}

#[test]
fn multi_claim() {
    let (s1, p1) = create_key_pair_from_seed(1);
    let (s2, p2) = create_key_pair_from_seed(2);
    let p1 = PublicKey::from(p1.to_byte_type());
    let p2 = PublicKey::from(p2.to_byte_type());
    let (mut test, faucet) = setup(vec![p1, p2]);

    let transfer1 = stealth::generate_transfer_data(NO_INPUTS, 1000u64, Some(1000), 0);
    let transfer2 = stealth::generate_transfer_data(NO_INPUTS, 1000u64, Some(1000), 0);
    let sig1 = sign_it(&s1);
    let sig2 = sign_it(&s2);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "claim_funds", args![p1, sig1, transfer1.statement])
            .call_method(faucet, "claim_funds", args![p2, sig2, transfer2.statement])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
}

#[test]
fn bad_signature() {
    let (s1, p1) = create_key_pair_from_seed(1);
    let p1 = PublicKey::from(p1.to_byte_type());
    let (mut test, faucet) = setup(vec![p1]);

    let transfer = stealth::generate_transfer_data(NO_INPUTS, 1000u64, Some(1000), 0);
    let sig1 = sign_it_with(&s1, b"A different message");
    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "claim_funds", args![p1, sig1, transfer.statement])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );

    assert_reject_reason(reason.clone(), "Your signature is invalid, so no funds for you");
}

#[test]
fn check_signature_api() {
    let (s1, p1) = create_key_pair_from_seed(1);
    let (_, p2) = create_key_pair_from_seed(2);
    let p1 = PublicKey::from(p1.to_byte_type());
    let p2 = PublicKey::from(p2.to_byte_type());
    let (mut test, _) = setup(vec![p1, p2]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let good_sig = sign_it(&s1);
    let bad_sig = sign_it_with(&s1, b"A different message");
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "check_sig", args![p1, bad_sig])
            .call_function(template_addr, "check_sig", args![p1, good_sig])
            // Bad public key
            .call_function(template_addr, "check_sig", args![p2, good_sig])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert!(
        !test.return_values()[0].decoded::<bool>().unwrap(),
        "Expected bad_sig to be false"
    );
    assert!(
        test.return_values()[1].decoded::<bool>().unwrap(),
        "Expected good_sig to be true"
    );
    assert!(
        !test.return_values()[2].decoded::<bool>().unwrap(),
        "Expected bad public key to be false"
    );
}

/// `PublicKey::Zero` is a valid encoding any caller can supply, and this verifier has no Ristretto key to check
/// against, so verification fails rather than aborting the executor.
#[test]
fn check_signature_with_a_zero_public_key() {
    let (s1, p1) = create_key_pair_from_seed(1);
    let p1 = PublicKey::from(p1.to_byte_type());
    let (mut test, _) = setup(vec![p1]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    test.execute_expect_success(
        test.transaction()
            .call_function(template_addr, "check_sig", args![PublicKey::Zero, sign_it(&s1)])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    assert!(!test.return_values()[0].decoded::<bool>().unwrap());
}

#[test]
fn claim_with_valid_sr25519_signature() {
    let (s1, p1) = create_key_pair_from_seed(1);
    let p1 = PublicKey::from(p1.to_byte_type());
    let (mut test, faucet) = setup(vec![p1]);

    let transfer = stealth::generate_transfer_data(NO_INPUTS, 1000u64, Some(1000), 0);
    let signature = sign_sr25519(&s1, TEST_DOMAIN, MESSAGE);
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(faucet, "claim_funds", args![p1, signature, transfer.statement])
            .build_and_seal(test.secret_key()),
        vec![test.owner_proof()],
    );
}

#[test]
fn check_sr25519_signature_api() {
    let (s1, p1) = create_key_pair_from_seed(1);
    let (_, p2) = create_key_pair_from_seed(2);
    let p1 = PublicKey::from(p1.to_byte_type());
    let p2 = PublicKey::from(p2.to_byte_type());
    let (mut test, _) = setup(vec![p1, p2]);
    let template_addr = test.get_template_address(TEMPLATE_NAME);

    let good_sig = sign_sr25519(&s1, TEST_DOMAIN, MESSAGE);
    let good_sig_wrong_key = sign_sr25519(&s1, TEST_DOMAIN, MESSAGE);
    let other_domain = sign_sr25519(&s1, b"substrate", MESSAGE);
    let other_message = sign_sr25519(&s1, TEST_DOMAIN, b"A different message");
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_function(template_addr, "check_sig", args![p1, good_sig])
            .call_function(template_addr, "check_sig", args![p1, other_domain])
            .call_function(template_addr, "check_sig", args![p1, other_message])
            .call_function(template_addr, "check_sig", args![p2, good_sig_wrong_key])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let verdicts = (0..4)
        .map(|i| test.return_values()[i].decoded::<bool>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        verdicts,
        [true, false, false, false],
        "only the signature under the template's domain, over its message, by its key verifies"
    );
}
