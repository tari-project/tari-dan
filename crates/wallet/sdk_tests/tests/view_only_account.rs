//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey as _,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine_types::{Utxo, UtxoOutput, crypto::OutputBody};
use tari_ootle_address::RistrettoOotleAddress;
use tari_ootle_common_types::Epoch;
use tari_ootle_wallet_sdk::{
    crypto::{memo::Memo, pay_to::PayTo},
    models::{AccountAndViewKeys, AccountWithAddress, KeyBranch, KeyId, OutputStatus},
};
use tari_template_lib::types::{
    Hash32,
    UtxoAddress,
    access_rules::AccessRule,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
    stealth::SpendAuthorization,
};

use crate::support::{PanicNetworkInterface, Test};

/// A sender wallet and a watch wallet holding the cold wallet's account 0 as a view-only account.
struct ViewOnlyTest {
    sender: Test,
    watch: Test,
    cold_address: RistrettoOotleAddress,
    account: AccountWithAddress,
}

fn setup() -> ViewOnlyTest {
    let sender = Test::new();
    let cold = Test::without_accounts(PanicNetworkInterface);
    let watch = Test::without_accounts(PanicNetworkInterface);

    let cold_address = cold.sdk().key_manager_api().derive_account_address(0).unwrap().address;
    let view_secret_key = cold
        .sdk()
        .key_manager_api()
        .get_key(KeyId::derived(KeyBranch::ViewOnlyKey, 0))
        .unwrap()
        .secret;

    let account = watch
        .sdk()
        .accounts_api()
        .add_view_only_account(
            Some("watch"),
            &cold_address.account_key.to_byte_type(),
            &view_secret_key,
            Some(Epoch::zero()),
            true,
        )
        .unwrap();
    assert_eq!(account.owner_key_id(), None);
    assert_eq!(*account.address(), cold_address.to_byte_type());

    ViewOnlyTest {
        sender,
        watch,
        cold_address,
        account,
    }
}

/// An output the sender pays to the cold account, encrypted and tagged for its view key.
fn output_to_cold_account(test: &ViewOnlyTest, value: u64, memo: &Memo, pay_to: PayTo) -> (UtxoAddress, Utxo) {
    let output = test
        .sender
        .sdk()
        .stealth_outputs_api()
        .create_output_witness(
            &test.cold_address,
            value,
            &STEALTH_TARI_RESOURCE_ADDRESS,
            None,
            Some(memo),
            pay_to,
        )
        .unwrap();
    let commitment: PedersenCommitmentBytes = output.witness.to_commitment().to_byte_type();
    let utxo = Utxo::new(UtxoOutput {
        output: OutputBody {
            public_nonce: output.witness.sender_public_nonce.to_byte_type(),
            encrypted_data: output.witness.encrypted_data.clone(),
            minimum_value_promise: 0,
            viewable_balance: None,
        },
        auth: output.auth,
        tag: output.tag,
    });
    (UtxoAddress::new(STEALTH_TARI_RESOURCE_ADDRESS, commitment.into()), utxo)
}

#[test]
fn view_only_account_decrypts_incoming_stealth_output() {
    let test = setup();
    let sender_address = test
        .sender
        .sdk()
        .key_manager_api()
        .derive_account_address(0)
        .unwrap()
        .address;
    let memo = Memo::new_sender_address(
        sender_address.account_key.to_byte_type().into_array(),
        sender_address.view_only_key.to_byte_type().into_array(),
        b"bridge-uuid",
    )
    .unwrap();
    let (address, utxo) = output_to_cold_account(&test, 1234, &memo, PayTo::StealthPublicKey);

    test.watch
        .sdk()
        .stealth_outputs_api()
        .verify_and_update_outputs([(address, &utxo)])
        .unwrap();

    let outputs = test
        .watch
        .sdk()
        .stealth_outputs_api()
        .utxos_get_many(
            &STEALTH_TARI_RESOURCE_ADDRESS,
            Some(test.account.component_address()),
            None,
        )
        .unwrap();
    assert_eq!(outputs.len(), 1);
    let output = &outputs[0];
    assert_eq!(output.value, 1234);
    assert!(matches!(output.status, OutputStatus::Unspent), "{:?}", output.status);
    assert_eq!(output.owner_key_id, None);
    let (sender_account_key, sender_view_key, pay_ref) = output
        .memo
        .as_ref()
        .and_then(|m| m.as_sender_address())
        .expect("sender address memo");
    assert_eq!(sender_account_key, sender_address.account_key.to_byte_type().as_bytes());
    assert_eq!(sender_view_key, sender_address.view_only_key.to_byte_type().as_bytes());
    assert_eq!(pay_ref, b"bridge-uuid");
}

#[test]
fn view_only_account_does_not_accept_output_with_a_foreign_spend_key() {
    let test = setup();
    let (address, mut utxo) = output_to_cold_account(
        &test,
        1234,
        &Memo::new_message("spoof").unwrap(),
        PayTo::StealthPublicKey,
    );
    // Encrypted and tagged for the view key, but spendable by a key the sender chose
    let (_, attacker_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    utxo.output.as_mut().unwrap().auth = SpendAuthorization::Key(attacker_key.to_byte_type());

    // The UTXO recovery path validates with the account's keys and no owner key
    let watch = test.watch.sdk();
    let keys = AccountAndViewKeys {
        account_public_key: *test.account.owner_public_key(),
        account_key: None,
        view_only_key: watch
            .key_manager_api()
            .get_key(test.account.view_only_key_id())
            .unwrap(),
    };
    let validated = watch
        .stealth_outputs_api()
        .validate_utxo(
            std::slice::from_ref(&keys),
            watch.network(),
            STEALTH_TARI_RESOURCE_ADDRESS,
            address.id().into_commitment_bytes(),
            utxo.output().unwrap(),
            false,
        )
        .unwrap()
        .expect("the output decrypts with the view key");
    assert!(
        matches!(validated.status, OutputStatus::Invalid),
        "{:?}",
        validated.status
    );

    watch
        .stealth_outputs_api()
        .verify_and_update_outputs([(address, &utxo)])
        .unwrap();
    let outputs = watch
        .stealth_outputs_api()
        .utxos_get_many(
            &STEALTH_TARI_RESOURCE_ADDRESS,
            Some(test.account.component_address()),
            None,
        )
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(
        matches!(outputs[0].status, OutputStatus::Invalid),
        "{:?}",
        outputs[0].status
    );
    let balance = watch
        .stealth_outputs_api()
        .get_unspent_balance(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap();
    assert_eq!(balance.utxo_count, 0);
}

#[test]
fn view_only_account_does_not_accept_condition_tree_output() {
    let test = setup();
    let (address, utxo) = output_to_cold_account(
        &test,
        1234,
        &Memo::new_message("anyone can spend").unwrap(),
        PayTo::AccessRule(AccessRule::AllowAll),
    );
    assert!(utxo.output().unwrap().auth.spend_key().is_none());

    let watch = test.watch.sdk();
    watch
        .stealth_outputs_api()
        .verify_and_update_outputs([(address, &utxo)])
        .unwrap();
    let outputs = watch
        .stealth_outputs_api()
        .utxos_get_many(
            &STEALTH_TARI_RESOURCE_ADDRESS,
            Some(test.account.component_address()),
            None,
        )
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].value, 1234);
    assert!(
        matches!(outputs[0].status, OutputStatus::Invalid),
        "{:?}",
        outputs[0].status
    );
    let balance = watch
        .stealth_outputs_api()
        .get_unspent_balance(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap();
    assert_eq!(balance.utxo_count, 0);
}

#[test]
fn view_only_account_does_not_accept_key_and_script_output() {
    let test = setup();
    let (address, mut utxo) = output_to_cold_account(
        &test,
        1234,
        &Memo::new_message("reclaimable").unwrap(),
        PayTo::StealthPublicKey,
    );
    // The account's spend key, plus a script path the sender could use to take the output back
    let output = utxo.output.as_mut().unwrap();
    let spend_key = *output.auth.spend_key().unwrap();
    output.auth = SpendAuthorization::KeyAndScript {
        spend_key,
        condition_root: Hash32::from_array([7; 32]),
    };

    let watch = test.watch.sdk();
    watch
        .stealth_outputs_api()
        .verify_and_update_outputs([(address, &utxo)])
        .unwrap();
    let outputs = watch
        .stealth_outputs_api()
        .utxos_get_many(
            &STEALTH_TARI_RESOURCE_ADDRESS,
            Some(test.account.component_address()),
            None,
        )
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert!(
        matches!(outputs[0].status, OutputStatus::Invalid),
        "{:?}",
        outputs[0].status
    );
    let balance = watch
        .stealth_outputs_api()
        .get_unspent_balance(&STEALTH_TARI_RESOURCE_ADDRESS)
        .unwrap();
    assert_eq!(balance.utxo_count, 0);
}

#[test]
fn view_only_account_rejects_invalid_keys_and_future_birthday() {
    let test = Test::without_accounts(PanicNetworkInterface);
    let accounts = test.sdk().accounts_api();
    let (account_secret, account_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let (view_secret, _) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let account_key = account_key.to_byte_type();

    let cases = [
        (
            RistrettoPublicKeyBytes::default(),
            view_secret.clone(),
            None,
            "identity point",
        ),
        (
            account_key,
            RistrettoSecretKey::default(),
            None,
            "view secret key is zero",
        ),
        (account_key, account_secret, None, "view key is the account key"),
        (
            account_key,
            view_secret,
            Some(Epoch(u64::MAX)),
            "birthday epoch is after the current epoch",
        ),
    ];
    for (account_key, view_secret, birthday, reason) in cases {
        let err = accounts
            .add_view_only_account(Some("watch"), &account_key, &view_secret, birthday, false)
            .expect_err(reason);
        assert!(err.is_invalid_view_only_account_error(), "{err}");
        assert!(err.to_string().contains(reason), "{err}");
    }
    assert_eq!(accounts.count().unwrap(), 0);
}
