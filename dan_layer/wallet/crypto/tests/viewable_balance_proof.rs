//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Instant;

use rand::rngs::OsRng;
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{pedersen::PedersenCommitment, RistrettoPublicKey, RistrettoSecretKey},
};
use tari_dan_wallet_crypto::{create_confidential_proof, AlwaysMissLookupTable, ConfidentialProofStatement};
use tari_engine_types::confidential::validate_elgamal_verifiable_balance_proof;
use tari_template_lib::models::Amount;
use tari_utilities::ByteArray;

fn create_output_statement(value: Amount, view_key: &RistrettoPublicKey) -> ConfidentialProofStatement {
    let mask = RistrettoSecretKey::random(&mut OsRng);
    ConfidentialProofStatement {
        amount: value,
        mask,
        sender_public_nonce: Default::default(),
        minimum_value_promise: 0,
        encrypted_data: Default::default(),
        reveal_amount: Default::default(),
        resource_view_key: Some(view_key.clone()),
    }
}

fn keypair_from_seed(seed: u8) -> (RistrettoSecretKey, RistrettoPublicKey) {
    let secret_key = RistrettoSecretKey::from_canonical_bytes(&[seed; 32]).unwrap();
    let public_key = RistrettoPublicKey::from_secret_key(&secret_key);
    (secret_key, public_key)
}

#[test]
fn it_allows_no_balance_proof_for_no_view_key() {
    let commitment = PedersenCommitment::from_public_key(&RistrettoPublicKey::default());
    let proof = validate_elgamal_verifiable_balance_proof(&commitment, None, None).unwrap();
    assert!(proof.is_none());
}

#[test]
fn it_errors_no_balance_proof_with_view_key() {
    let (_, view_key) = keypair_from_seed(1);
    let output_statement = create_output_statement(123.into(), &view_key);

    let proof = create_confidential_proof(&output_statement, None).unwrap();
    let output_statement = proof.output_statement.as_ref().unwrap();
    let viewable_balance_proof = proof
        .output_statement
        .as_ref()
        .unwrap()
        .viewable_balance_proof
        .as_ref()
        .unwrap();
    let commitment = PedersenCommitment::from_canonical_bytes(output_statement.commitment.as_ref()).unwrap();
    validate_elgamal_verifiable_balance_proof(&commitment, None, Some(viewable_balance_proof)).unwrap_err();
}

#[test]
fn it_errors_with_balance_proof_and_no_view_key() {
    let commitment = PedersenCommitment::from_public_key(&RistrettoPublicKey::default());
    validate_elgamal_verifiable_balance_proof(&commitment, Some(&RistrettoPublicKey::default()), None).unwrap_err();
}

#[test]
fn it_generates_a_valid_proof() {
    let (view_key_secret, view_key) = keypair_from_seed(1);
    let output_statement = create_output_statement(123.into(), &view_key);

    let timer = Instant::now();
    let proof = create_confidential_proof(&output_statement, None).unwrap();
    let gen_proof_time = timer.elapsed();

    let output_statement = proof.output_statement.as_ref().unwrap();
    let viewable_balance_proof = proof
        .output_statement
        .as_ref()
        .unwrap()
        .viewable_balance_proof
        .as_ref()
        .unwrap();
    let commitment = PedersenCommitment::from_canonical_bytes(output_statement.commitment.as_ref()).unwrap();
    let timer = Instant::now();
    let proof = validate_elgamal_verifiable_balance_proof(&commitment, Some(&view_key), Some(viewable_balance_proof))
        .unwrap()
        .unwrap();
    let validate_proof_time = timer.elapsed();

    let timer = Instant::now();
    let balance = proof
        .brute_force_balance(&view_key_secret, 0..=1000, &mut AlwaysMissLookupTable)
        .unwrap();
    let brute_force_time = timer.elapsed();
    assert_eq!(balance, Some(123));

    println!("Generate proof time: {:?}", gen_proof_time);
    println!("Validate proof time: {:?}", validate_proof_time);
    println!("Brute force time: {:?}", brute_force_time);
}
