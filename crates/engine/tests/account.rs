//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_engine::runtime::{ActionIdent, RuntimeError};
use tari_ootle_transaction::{Epoch, Transaction, args};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::types::{Amount, OwnerRule, access_rules::ComponentAccessRules, constants::TARI_TOKEN, rule};
use tari_template_test_tooling::{
    TemplateTest,
    support::assert_error::{assert_access_denied_for_action, assert_reject_reason},
    xtr_faucet_component,
};

const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn basic_faucet_transfer() {
    let mut template_test = TemplateTest::new(CRATE_PATH, Vec::<&str>::new());

    // Create sender and receiver accounts
    let (sender_address, sender_proof, _) = template_test.create_funded_account();
    let (receiver_address, _, _) = template_test.create_empty_account();

    let result = template_test
        .build_and_execute(
            Transaction::builder_localnet(Epoch(1))
                .call_method(sender_address, "withdraw", args![TARI_TOKEN, 100])
                .put_last_instruction_output_on_workspace("foo_bucket")
                .call_method(receiver_address, "deposit", args![Workspace("foo_bucket")])
                .call_method(sender_address, "balance", args![TARI_TOKEN])
                .call_method(receiver_address, "balance", args![TARI_TOKEN]),
            // Sender proof needed to withdraw
            vec![sender_proof],
        )
        .unwrap_success();

    assert_eq!(
        result.finalize.execution_results[3].decode::<Amount>().unwrap(),
        Amount::from(999_999_900u64)
    );
    assert_eq!(
        result.finalize.execution_results[4].decode::<Amount>().unwrap(),
        Amount::from(100u64)
    );
}

#[test]
fn withdraw_from_account_prevented() {
    let mut test = TemplateTest::new(CRATE_PATH, Vec::<&str>::new());

    // Create sender and receiver accounts
    let (source_account, _, _) = test.create_funded_account();

    let (dest_address, non_owning_token, non_owning_key) = test.create_empty_account();

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(source_account, "withdraw", args![TARI_TOKEN, 100])
            .put_last_instruction_output_on_workspace("stolen_coins")
            .call_method(dest_address, "deposit", args![Workspace("stolen_coins")])
            .build_and_seal(&non_owning_key),
        // VNs provide the token that signed the transaction, which in this case is the non_owning_token
        vec![non_owning_token],
    );

    assert_access_denied_for_action(reason, ActionIdent::ComponentCallMethod {
        component_address: source_account,
        method: "withdraw".to_string(),
    });

    let vaults = test
        .read_only_state_store()
        .get_vaults_for_account(dest_address)
        .unwrap();
    assert!(!vaults.contains_key(&TARI_TOKEN));
}

#[test]
fn attempt_to_overwrite_account() {
    let mut test = TemplateTest::new_builtin_only();

    // Create initial account with faucet funds
    let (source_account, source_account_proof, source_account_sk) = test.create_funded_account();

    let null: Option<()> = None;
    let overwriting_tx = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            // `CreateAccount` is idempotent, so a direct call into the template is the only shape an overwrite
            // attempt can take
            .call_function(
                ACCOUNT_TEMPLATE_ADDRESS,
                "create",
                args![source_account_proof, null, null, null],
            )
            // Signed by source account so that it can pay the fees for the new account creation
            .build_and_seal(&source_account_sk),
        vec![source_account_proof],
    );

    assert_reject_reason(overwriting_tx, "The account constructor cannot be called directly");

    let store = test.read_only_state_store();
    let account = store.get_account(source_account).unwrap();
    let vault = account.get_vault_by_resource(&TARI_TOKEN).unwrap();
    // Double check that the source account was not overwritten due to the address collision, if it was, then we'd have
    // no vaults
    let vault = store.get_vault(&vault.vault_id()).expect("no vaults");
    assert_eq!(vault.balance(), TemplateTest::FUNDED_ACCOUNT_INITIAL_BALANCE);
}

#[test]
fn create_account_is_idempotent() {
    let mut test = TemplateTest::new_builtin_only();

    // Create initial account with faucet funds
    let (source_account, source_account_proof, source_account_sk) = test.create_funded_account();
    let source_account_pk = RistrettoPublicKey::from_secret_key(&source_account_sk);

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            // Create component with the same ID
            .create_account(source_account_pk.to_byte_type())
            // Signed by source account so that it can pay the fees for the new account creation
            .build_and_seal(&source_account_sk),
        vec![source_account_proof],
    );

    assert!(
        result
            .finalize
            .events
            .iter()
            .all(|e| e.topic() != "std.component.created")
    );

    let store = test.read_only_state_store();
    let account = store.get_account(source_account).unwrap();
    let vault = account.get_vault_by_resource(&TARI_TOKEN).unwrap();
    // Double check that the source account was not overwritten due to the address collision, if it was, then we'd have
    // no vaults
    let vault = store.get_vault(&vault.vault_id()).expect("no vaults");
    assert_eq!(vault.balance(), TemplateTest::FUNDED_ACCOUNT_INITIAL_BALANCE);
}

#[test]
fn create_account_is_idempotent_with_deposit() {
    let mut test = TemplateTest::new_builtin_only();

    // Create initial account with faucet funds
    let (source_account, source_account_proof, source_account_sk) = test.create_empty_account();
    let source_account_pk = RistrettoPublicKey::from_secret_key(&source_account_sk);

    let result = test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            // Create component with the same ID
            .create_account(source_account_pk.to_byte_type())
            .put_last_instruction_output_on_workspace("account")
            .call_method(xtr_faucet_component(), "take", args![Workspace("account")])
            // Signed by source account so that it can pay the fees for the new account creation
            .build_and_seal(&source_account_sk),
        vec![source_account_proof],
    );

    assert!(
        result
            .finalize
            .events
            .iter()
            .all(|e| e.topic() != "std.component.created")
    );

    let store = test.read_only_state_store();
    let account = store.get_account(source_account).unwrap();
    let vault = account.get_vault_by_resource(&TARI_TOKEN).unwrap();
    let vault = store.get_vault(&vault.vault_id()).unwrap();
    assert_eq!(vault.balance(), 1_000_000_000u64);
}

#[test]
fn gasless() {
    let mut test = TemplateTest::new_builtin_only();
    test.enable_fees();

    // Create initial account with faucet funds
    let (fee_account, fee_account_proof, fee_account_sk) = test.create_funded_account();
    let (user_account, user_account_proof, user_account_sk) = test.create_funded_account();
    let (user2_account, _, _) = test.create_empty_account();

    let fee_account_pk = RistrettoPublicKey::from_secret_key(&fee_account_sk);

    test.execute_expect_success(
        test.transaction()
            .pay_fee_from_component(fee_account, 2000u64)
            .call_method(user_account, "withdraw", args![TARI_TOKEN, 100])
            .put_last_instruction_output_on_workspace("b")
            .call_method(user2_account, "deposit", args![Workspace("b")])
            .finish()
            .add_signer(&fee_account_pk.to_byte_type(), &user_account_sk)
            .seal(&fee_account_sk),
        vec![fee_account_proof, user_account_proof],
    );

    let vaults = test
        .read_only_state_store()
        .get_vaults_for_account(user2_account)
        .unwrap();
    assert_eq!(vaults.get(&TARI_TOKEN).unwrap().balance(), 100u64);
}

#[test]
fn custom_access_rules() {
    let mut test = TemplateTest::new_builtin_only();

    // First we create a account with a custom rule that anyone can withdraw
    let (owner_proof, public_key, secret_key) = test.create_owner_proof();

    let access_rules = ComponentAccessRules::new()
        .add_method_rule("balance", rule!(allow_all))
        .add_method_rule("get_balances", rule!(allow_all))
        .add_method_rule("deposit", rule!(allow_all))
        // We are going to make it so anyone can withdraw
        .default(rule!(allow_all));

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            // Create component with the same ID
            .create_account_custom::<&str>(
                public_key.to_byte_type(),
                None,
                Some(access_rules),
                None
            )
            .put_last_instruction_output_on_workspace("account")
            .call_method(xtr_faucet_component(), "take", args![Workspace("account")])
            .build_and_seal(&secret_key),
        vec![owner_proof],
    );
    let user_account = *test
        .read_only_state_store()
        .all_accounts()
        .unwrap()
        .keys()
        .next()
        .unwrap();

    // We create another account and we we will withdraw from the custom one
    let (user2_account, user2_account_proof, user2_secret_key) = test.create_funded_account();
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(user_account, "withdraw", args![TARI_TOKEN, 100])
            .put_last_instruction_output_on_workspace("b")
            .call_method(user2_account, "deposit", args![Workspace("b")])
            .build_and_seal(&user2_secret_key),
        vec![user2_account_proof],
    );
}

#[test]
fn take_from_bucket() {
    let mut test = TemplateTest::new(CRATE_PATH, Vec::<&str>::new());

    let (alice, _proof, alice_sk) = test.create_funded_account();
    let (bob, _proof, _bob_sk) = test.create_empty_account();

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(alice, "withdraw_all", args![TARI_TOKEN])
            .put_last_instruction_output_on_workspace("coins")
            .take_from_bucket("coins", 100u64, "foo_bucket")
            // Take all remaining coins — tests that an empty bucket does not cause a dangling bucket error
            .take_from_bucket("coins", 999_999_900u64, "bar_bucket")
            .call_method(alice, "deposit", args![Workspace("foo_bucket")])
            .call_method(bob, "deposit", args![Workspace("bar_bucket")])
            .add_signer(&test.to_public_key_bytes(), &alice_sk)
            .seal(test.secret_key()),
        vec![],
    );

    let alice_acc = test.read_only_state_store().get_account(alice).unwrap();
    let bob_acc = test.read_only_state_store().get_account(bob).unwrap();
    let alice_balance = test
        .read_only_state_store()
        .get_vault(&alice_acc.get_vault_by_resource(&TARI_TOKEN).unwrap().vault_id())
        .unwrap()
        .balance();
    let bob_balance = test
        .read_only_state_store()
        .get_vault(&bob_acc.get_vault_by_resource(&TARI_TOKEN).unwrap().vault_id())
        .unwrap()
        .balance();

    assert_eq!(alice_balance, 100u64);
    assert_eq!(bob_balance, 999_999_900u64);
}

#[test]
fn put_into_bucket_merges_same_resource() {
    let mut test = TemplateTest::new(CRATE_PATH, Vec::<&str>::new());

    let (alice, _proof, alice_sk) = test.create_funded_account();
    let (bob, _proof, _bob_sk) = test.create_empty_account();

    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .call_method(alice, "withdraw", args![TARI_TOKEN, 600u64])
            .put_last_instruction_output_on_workspace("first")
            .call_method(alice, "withdraw", args![TARI_TOKEN, 400u64])
            .put_last_instruction_output_on_workspace("second")
            .put_into_bucket("second", "first")
            .call_method(bob, "deposit", args![Workspace("first")])
            .add_signer(&test.to_public_key_bytes(), &alice_sk)
            .seal(test.secret_key()),
        vec![],
    );

    let bob_acc = test.read_only_state_store().get_account(bob).unwrap();
    let bob_balance = test
        .read_only_state_store()
        .get_vault(&bob_acc.get_vault_by_resource(&TARI_TOKEN).unwrap().vault_id())
        .unwrap()
        .balance();
    assert_eq!(bob_balance, 1000u64);
}

#[test]
fn put_into_bucket_rejects_resource_mismatch() {
    let mut test = TemplateTest::new(CRATE_PATH, vec!["tests/templates/faucet"]);

    let faucet_template = test.get_template_address("TestFaucet");
    let (alice, _alice_proof, alice_sk) = test.create_funded_account();

    // Mint a non-TARI fungible resource and fund alice with it.
    test.execute_expect_success(
        Transaction::builder_localnet(Epoch(1))
            .allocate_component_address("faucet_address")
            .call_function(faucet_template, "mint_with_opts", args![
                Amount::from(1_000_000u64),
                "TT".to_string(),
                Workspace("faucet_address")
            ])
            .call_method("faucet_address", "take_free_coins", args![])
            .put_last_instruction_output_on_workspace("free_coins")
            .call_method(alice, "deposit", args![Workspace("free_coins")])
            .build_and_seal(test.secret_key()),
        vec![],
    );

    let resources = test.read_only_state_store().get_all_resources().unwrap();
    let (tt_resource, _) = resources
        .into_iter()
        .find(|(_, r)| r.token_symbol() == Some("TT"))
        .expect("TT resource not found");

    let reason = test.execute_expect_failure(
        Transaction::builder_localnet(Epoch(1))
            .call_method(alice, "withdraw", args![TARI_TOKEN, 100u64])
            .put_last_instruction_output_on_workspace("tari_bucket")
            .call_method(alice, "withdraw", args![tt_resource, 100u64])
            .put_last_instruction_output_on_workspace("tt_bucket")
            // Different resources — engine should refuse the join.
            .put_into_bucket("tt_bucket", "tari_bucket")
            .add_signer(&test.to_public_key_bytes(), &alice_sk)
            .seal(test.secret_key()),
        vec![],
    );

    assert!(
        format!("{reason}").contains("Resource addresses do not match"),
        "expected ResourceAddressMismatch, got: {reason}"
    );
}

#[test]
fn custom_ownership_of_another_keys_account_is_refused() {
    let mut test = TemplateTest::new_builtin_only();
    let (_payer_proof, _payer_pk, payer_sk) = test.create_owner_proof();
    let (_victim_proof, victim_pk, _victim_sk) = test.create_owner_proof();

    // The squatter hands itself the owner rule on the address derived from the victim's key.
    let reason = test.execute_expect_failure(
        test.transaction()
            .create_account_custom::<&str>(
                victim_pk.to_byte_type(),
                Some(OwnerRule::ByAccessRule(rule!(allow_all))),
                None,
                None,
            )
            .build_and_seal(&payer_sk),
        vec![],
    );

    assert_reject_reason(reason, RuntimeError::SignerBadgeNotInScope {
        public_key: victim_pk.to_byte_type(),
    });

    // Custom access rules are gated the same way.
    let reason = test.execute_expect_failure(
        test.transaction()
            .create_account_custom::<&str>(
                victim_pk.to_byte_type(),
                None,
                Some(ComponentAccessRules::new().default(rule!(allow_all))),
                None,
            )
            .build_and_seal(&payer_sk),
        vec![],
    );

    assert_reject_reason(reason, RuntimeError::SignerBadgeNotInScope {
        public_key: victim_pk.to_byte_type(),
    });
}

#[test]
fn the_account_constructor_is_not_callable_as_a_function() {
    let mut test = TemplateTest::new_builtin_only();
    let (_payer_proof, _payer_pk, payer_sk) = test.create_owner_proof();
    let (victim_proof, _victim_pk, _victim_sk) = test.create_owner_proof();

    let null: Option<()> = None;
    let reason = test.execute_expect_failure(
        test.transaction()
            .call_function(ACCOUNT_TEMPLATE_ADDRESS, "create", args![
                victim_proof,
                Some(OwnerRule::ByAccessRule(rule!(allow_all))),
                null,
                null
            ])
            .build_and_seal(&payer_sk),
        vec![],
    );

    assert_reject_reason(reason, "The account constructor cannot be called directly");
}

#[test]
fn the_account_constructor_is_not_reachable_by_a_cross_template_call() {
    let mut test = TemplateTest::new(CRATE_PATH, ["tests/templates/shenanigans"]);
    let template = test.get_template_address("Shenanigans");
    let (_payer_proof, _payer_pk, payer_sk) = test.create_owner_proof();
    let (victim_proof, _victim_pk, _victim_sk) = test.create_owner_proof();

    let reason = test.execute_expect_failure(
        test.transaction()
            .call_function(template, "create_account_for", args![victim_proof])
            .build_and_seal(&payer_sk),
        vec![],
    );

    assert_reject_reason(reason, "The account constructor cannot be called directly");
}

#[test]
fn an_account_for_another_key_may_still_be_created_on_the_default_rules() {
    let mut test = TemplateTest::new_builtin_only();
    let (_payer_proof, _payer_pk, payer_sk) = test.create_owner_proof();
    let (_victim_proof, victim_pk, _victim_sk) = test.create_owner_proof();

    test.execute_expect_success(
        test.transaction()
            .create_account(victim_pk.to_byte_type())
            .put_last_instruction_output_on_workspace("account")
            .call_method(xtr_faucet_component(), "take", args![Workspace("account")])
            .build_and_seal(&payer_sk),
        vec![],
    );

    let account = *test
        .read_only_state_store()
        .all_accounts()
        .unwrap()
        .keys()
        .next()
        .unwrap();
    let vaults = test.read_only_state_store().get_vaults_for_account(account).unwrap();
    assert_eq!(vaults.get(&TARI_TOKEN).unwrap().balance(), 1_000_000_000u64);
}

/// A proof over a stealth resource locks and unlocks a stealth container, TARI included.
#[test]
fn a_proof_over_a_stealth_resource_can_be_created_and_dropped() {
    let mut test = TemplateTest::new_builtin_only();
    let (account, account_proof, account_key) = test.create_funded_account();

    test.execute_expect_success(
        test.transaction()
            .call_method(account, "create_proof_for_resource", args![TARI_TOKEN])
            .put_last_instruction_output_on_workspace("proof")
            .drop_all_proofs_in_workspace()
            .build_and_seal(&account_key),
        vec![account_proof],
    );
}
