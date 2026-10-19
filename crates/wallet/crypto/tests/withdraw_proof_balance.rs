//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::{keys::SecretKey, ristretto::RistrettoSecretKey};
use tari_engine_types::{
    confidential::MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
    crypto::commit_u64_amount,
    resource_container::ResourceContainer,
};
use tari_ootle_wallet_crypto::{MaskAndValue, OutputWitness, WalletCryptoError, confidential::create_withdraw_proof};
use tari_template_lib_types::{Amount, EncryptedData, ResourceAddress};

fn random_mask() -> RistrettoSecretKey {
    RistrettoSecretKey::random(&mut rand::rng())
}

fn witness(amount: u64) -> OutputWitness {
    OutputWitness {
        amount,
        mask: random_mask(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
        encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
        resource_view_key: None,
    }
}

#[test]
fn it_accepts_a_balanced_proof_with_revealed_output() {
    let input_mask = random_mask();
    let input = MaskAndValue::new(1_000, input_mask.clone());

    let proof = create_withdraw_proof(
        &[input],
        Amount::zero(),
        Some(&witness(400)),
        Amount::from(100u64),
        Some(&witness(500)),
        Amount::zero(),
    )
    .unwrap();

    let input_commitment = commit_u64_amount(&input_mask, 1_000).to_byte_type();
    let resource_address =
        ResourceAddress::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
    let mut container = ResourceContainer::confidential(resource_address, [input_commitment], Amount::zero());
    container.withdraw_confidential(proof, None).unwrap();
}

#[test]
fn it_rejects_change_that_omits_the_revealed_output() {
    let input = MaskAndValue::new(1_000, random_mask());

    let err = create_withdraw_proof(
        &[input],
        Amount::zero(),
        Some(&witness(400)),
        Amount::from(100u64),
        Some(&witness(600)),
        Amount::zero(),
    )
    .unwrap_err();

    assert!(
        matches!(err, WalletCryptoError::UnbalancedWithdraw { .. }),
        "expected UnbalancedWithdraw, got {err:?}"
    );
}

#[test]
fn it_returns_revealed_change_to_the_source_without_a_change_output() {
    let input_mask = random_mask();
    let input = MaskAndValue::new(1_000, input_mask.clone());

    let proof = create_withdraw_proof(
        &[input],
        Amount::from(50u64),
        Some(&witness(400)),
        Amount::zero(),
        None,
        Amount::from(650u64),
    )
    .unwrap();

    let input_commitment = commit_u64_amount(&input_mask, 1_000).to_byte_type();
    let resource_address =
        ResourceAddress::from_hex("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef").unwrap();
    let mut container = ResourceContainer::confidential(resource_address, [input_commitment], Amount::from(200u64));
    let (output, effects) = container.withdraw_confidential(proof, None).unwrap();

    assert_eq!(effects.spent_commitments, vec![input_commitment]);
    assert_eq!(effects.created_outputs.len(), 1);
    assert_eq!(output.unlocked_amount(), Amount::zero());
    // 200 - 50 input revealed + 650 revealed change
    assert_eq!(container.unlocked_amount(), Amount::from(800u64));
}

#[test]
fn it_rejects_outputs_less_than_inputs() {
    let input = MaskAndValue::new(1_000, random_mask());

    let err = create_withdraw_proof(
        &[input],
        Amount::from(5u64),
        Some(&witness(400)),
        Amount::zero(),
        Some(&witness(600)),
        Amount::zero(),
    )
    .unwrap_err();

    assert!(
        matches!(err, WalletCryptoError::UnbalancedWithdraw { .. }),
        "expected UnbalancedWithdraw, got {err:?}"
    );
}
