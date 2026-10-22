//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use std::str::FromStr;

use tari_crypto::ristretto::RistrettoSecretKey;
use tari_ootle_common_types::{Epoch, SubstateVersion};
use tari_ootle_transaction::{Transaction, TransactionId, args};
use tari_ootle_wallet_sdk::{
    models::{BalanceChangeSource, BalanceChangeSourceType, KeyBranch, KeyId, OutputStatus, StealthOutputModel},
    storage::{WalletStoreWriter, WriteableWalletStore},
};
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    EncryptedData,
    ResourceAddress,
    ResourceType,
    VaultId,
    constants::TARI_TOKEN,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
    stealth::SpendAuthorization,
};

use crate::support::Test;

fn build_transaction() -> Transaction {
    Transaction::builder_localnet(Epoch(1))
        .allocate_component_address("component")
        .put_last_instruction_output_on_workspace("bucket")
        .call_method("component", "new", args!["bucket"])
        .build_and_seal(&RistrettoSecretKey::from(1u64))
}

fn stealth_output(account: ComponentAddress, value: u64, seed: u8) -> StealthOutputModel {
    StealthOutputModel {
        owner_account: account,
        resource_address: TARI_TOKEN,
        commitment: PedersenCommitmentBytes::from_array([seed; PedersenCommitmentBytes::length()]),
        value,
        sender_public_nonce: RistrettoPublicKeyBytes::from_bytes(
            &[seed.wrapping_add(1); RistrettoPublicKeyBytes::length()],
        )
        .unwrap(),
        view_only_key_id: KeyId::derived(KeyBranch::ViewOnlyKey, 0),
        owner_key_id: Some(KeyId::derived(KeyBranch::Account, 0)),
        encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
        tag_byte: UtxoTag::new(u32::from(seed)),
        memo: None,
        auth: SpendAuthorization::Key(RistrettoPublicKeyBytes::default()),
        minimum_value_promise: 0,
        status: OutputStatus::Unspent,
        is_burnt: false,
        is_frozen: false,
        is_on_chain: true,
        is_condition_spendable: true,
        lock_id: None,
    }
}

fn resource_address(seed: u8) -> ResourceAddress {
    format!("resource_{seed:064x}").parse().unwrap()
}

#[test]
fn transaction_changes_can_span_accounts_and_vaults() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let second_account =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8aaaaaaaa")
            .unwrap();
    let second_vault =
        VaultId::from_str("vault_0000000000000000000000000000000000000000000000000000000000000002").unwrap();
    accounts
        .add_account(
            Some("second"),
            &second_account,
            KeyId::derived(KeyBranch::ViewOnlyKey, 1),
            KeyId::derived(KeyBranch::Account, 1),
            Epoch::zero(),
            true,
            false,
        )
        .unwrap();
    accounts
        .add_vault(
            second_account,
            second_vault,
            SubstateVersion::ZERO,
            TARI_TOKEN,
            ResourceType::Fungible,
            Some("XTR".to_string()),
            6,
        )
        .unwrap();

    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    test.store()
        .with_write_tx(|tx| {
            tx.transactions_insert(
                &transaction,
                None,
                &[Test::test_account_address(), second_account],
                false,
            )
        })
        .unwrap();

    let source = BalanceChangeSource::Transaction { transaction_id };
    assert!(
        accounts
            .update_vault_balance_and_record_change(
                Test::test_vault_address(),
                SubstateVersion::new(1),
                Amount::from(25u64),
                Amount::from(5u64),
                source
            )
            .unwrap()
    );
    assert!(
        accounts
            .update_vault_balance_and_record_change(
                second_vault,
                SubstateVersion::new(1),
                Amount::from(9u64),
                Amount::zero(),
                source
            )
            .unwrap()
    );

    for (account, expected_delta) in [(Test::test_account_address(), "25"), (second_account, "9")] {
        let page = accounts
            .get_balance_changes(&account, 0, 10, None, Some(&transaction_id), None)
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.changes[0].revealed_delta, expected_delta);
        assert_eq!(page.changes[0].source, source);
    }
}

#[test]
fn scan_and_recovery_are_deduplicated_and_duplicate_version_cannot_update_balance() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let vault = Test::test_vault_address();

    assert!(
        accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(1),
                Amount::from(10u64),
                Amount::zero(),
                BalanceChangeSource::Scan
            )
            .unwrap()
    );
    assert!(
        !accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(1),
                Amount::from(10u64),
                Amount::zero(),
                BalanceChangeSource::Scan
            )
            .unwrap()
    );
    assert!(
        accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(2),
                Amount::from(12u64),
                Amount::zero(),
                BalanceChangeSource::Recovery,
            )
            .unwrap()
    );
    let page = accounts
        .get_balance_changes(&Test::test_account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.changes[0].source, BalanceChangeSource::Recovery);
    assert_eq!(page.changes[1].source, BalanceChangeSource::Scan);

    let result = accounts.update_vault_balance_and_record_change(
        vault,
        SubstateVersion::new(2),
        Amount::from(99u64),
        Amount::zero(),
        BalanceChangeSource::Transaction {
            transaction_id: TransactionId::default(),
        },
    );
    assert!(!result.unwrap());
    assert_eq!(
        accounts.get_vault_balance(&vault).unwrap().revealed,
        Amount::from(12u64)
    );
    assert_eq!(
        accounts
            .get_balance_changes(&Test::test_account_address(), 0, 10, None, None, None)
            .unwrap()
            .total,
        2
    );
}

#[test]
fn scan_same_version_updates_existing_history_and_vault_snapshot() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let vault = Test::test_vault_address();

    assert!(
        accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(1),
                Amount::from(10u64),
                Amount::zero(),
                BalanceChangeSource::Scan,
            )
            .unwrap()
    );
    assert!(
        accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(1),
                Amount::from(15u64),
                Amount::from(5u64),
                BalanceChangeSource::Scan,
            )
            .unwrap()
    );

    let page = accounts
        .get_balance_changes(&Test::test_account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.changes[0].revealed_delta, "15");
    assert_eq!(page.changes[0].confidential_delta, "5");

    let balance = accounts.get_vault_balance(&vault).unwrap();
    assert_eq!(balance.revealed, Amount::from(15u64));
    assert_eq!(balance.confidential, Amount::from(5u64));
}

#[test]
fn stealth_balance_helper_is_account_and_resource_scoped() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let second_account =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8bbbbbbbb")
            .unwrap();
    let second_vault =
        VaultId::from_str("vault_00000000000000000000000000000000000000000000000000000000000000bb").unwrap();
    accounts
        .add_account(
            Some("second"),
            &second_account,
            KeyId::derived(KeyBranch::ViewOnlyKey, 1),
            KeyId::derived(KeyBranch::Account, 1),
            Epoch::zero(),
            true,
            false,
        )
        .unwrap();
    accounts
        .add_vault(
            second_account,
            second_vault,
            SubstateVersion::ZERO,
            TARI_TOKEN,
            ResourceType::Stealth,
            Some("TEST".to_string()),
            6,
        )
        .unwrap();

    let outputs = test.sdk().stealth_outputs_api();
    outputs
        .add_output(&stealth_output(Test::test_account_address(), 10, 10))
        .unwrap();
    outputs.add_output(&stealth_output(second_account, 99, 11)).unwrap();

    assert_eq!(
        outputs
            .get_unspent_balance_for_account_resource(&Test::test_account_address(), &TARI_TOKEN)
            .unwrap(),
        Amount::from(10u64)
    );
    assert_eq!(
        outputs
            .get_unspent_balance_for_account_resource(&second_account, &TARI_TOKEN)
            .unwrap(),
        Amount::from(99u64)
    );
}

#[test]
fn resource_balance_history_does_not_require_a_vault() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let second_account =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8cccccccc")
            .unwrap();
    accounts
        .add_account(
            Some("second"),
            &second_account,
            KeyId::derived(KeyBranch::ViewOnlyKey, 1),
            KeyId::derived(KeyBranch::Account, 1),
            Epoch::zero(),
            true,
            false,
        )
        .unwrap();

    assert!(
        accounts
            .record_resource_balance_change(
                second_account,
                TARI_TOKEN,
                Amount::zero(),
                Amount::from(25u64),
                BalanceChangeSource::Scan,
            )
            .unwrap()
    );
    assert!(
        accounts
            .record_resource_balance_change(
                second_account,
                TARI_TOKEN,
                Amount::zero(),
                Amount::from(40u64),
                BalanceChangeSource::Scan,
            )
            .unwrap()
    );

    let page = accounts
        .get_balance_changes(&second_account, 0, 10, Some(&TARI_TOKEN), None, None)
        .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.changes[0].vault_address, None);
    assert_eq!(page.changes[0].confidential_delta, "15");
    assert_eq!(page.changes[1].confidential_delta, "25");
}

// A stealth vault created for an account with prior confidential history starts from the last recorded
// balance, so the first vault refresh states only what its transaction changed.
#[test]
fn adding_a_stealth_vault_seeds_confidential_balance_from_history() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let second_account =
        ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8dddddddd")
            .unwrap();
    let second_vault =
        VaultId::from_str("vault_00000000000000000000000000000000000000000000000000000000000000dd").unwrap();
    accounts
        .add_account(
            Some("second"),
            &second_account,
            KeyId::derived(KeyBranch::ViewOnlyKey, 1),
            KeyId::derived(KeyBranch::Account, 1),
            Epoch::zero(),
            true,
            false,
        )
        .unwrap();
    assert!(
        accounts
            .record_resource_balance_change(
                second_account,
                TARI_TOKEN,
                Amount::zero(),
                Amount::from(1_000u64),
                BalanceChangeSource::Scan,
            )
            .unwrap()
    );

    accounts
        .add_vault(
            second_account,
            second_vault,
            SubstateVersion::ZERO,
            TARI_TOKEN,
            ResourceType::Stealth,
            Some("TEST".to_string()),
            6,
        )
        .unwrap();

    let vault = accounts.get_vault_by_resource(&second_account, &TARI_TOKEN).unwrap();
    assert_eq!(vault.confidential_balance, Amount::from(1_000u64));
    assert_eq!(vault.revealed_balance, Amount::zero());
}

#[test]
fn uncached_vaultless_resource_balance_change_is_skipped() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let uncached_resource = resource_address(0xaa);

    assert!(
        !accounts
            .record_resource_balance_change(
                Test::test_account_address(),
                uncached_resource,
                Amount::zero(),
                Amount::from(25u64),
                BalanceChangeSource::Scan,
            )
            .unwrap()
    );

    let page = accounts
        .get_balance_changes(
            &Test::test_account_address(),
            0,
            10,
            Some(&uncached_resource),
            None,
            None,
        )
        .unwrap();
    assert_eq!(page.total, 0);
    assert!(page.changes.is_empty());
}

#[test]
fn transaction_source_attributes_existing_scan_for_exact_vault_version() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let vault = Test::test_vault_address();
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    test.store()
        .with_write_tx(|tx| tx.transactions_insert(&transaction, None, &[Test::test_account_address()], false))
        .unwrap();

    assert!(
        accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(7),
                Amount::from(10u64),
                Amount::zero(),
                BalanceChangeSource::Scan,
            )
            .unwrap()
    );
    let source = BalanceChangeSource::Transaction { transaction_id };
    assert!(
        accounts
            .attribute_balance_change_to_transaction(&vault, SubstateVersion::new(7), transaction_id)
            .unwrap()
    );
    assert!(
        accounts
            .attribute_balance_change_to_transaction(&vault, SubstateVersion::new(99), transaction_id)
            .unwrap()
    );

    let page = accounts
        .get_balance_changes(&Test::test_account_address(), 0, 10, None, Some(&transaction_id), None)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.changes[0].source, source);
    assert_eq!(page.changes[0].transaction_id, Some(transaction_id));
    assert_eq!(page.changes[0].revealed_delta, "10");
    assert_eq!(
        accounts
            .get_balance_changes(
                &Test::test_account_address(),
                0,
                10,
                None,
                None,
                Some(BalanceChangeSourceType::Scan),
            )
            .unwrap()
            .total,
        0
    );
}

#[test]
fn replayed_transaction_cannot_update_a_vault_without_a_new_history_row() {
    let test = Test::new();
    let accounts = test.sdk().accounts_api();
    let vault = Test::test_vault_address();
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    test.store()
        .with_write_tx(|tx| tx.transactions_insert(&transaction, None, &[Test::test_account_address()], false))
        .unwrap();

    let source = BalanceChangeSource::Transaction { transaction_id };
    assert!(
        accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(1),
                Amount::from(10u64),
                Amount::zero(),
                source
            )
            .unwrap()
    );
    assert!(
        accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(2),
                Amount::from(20u64),
                Amount::zero(),
                BalanceChangeSource::Scan,
            )
            .unwrap()
    );
    assert!(
        !accounts
            .update_vault_balance_and_record_change(
                vault,
                SubstateVersion::new(1),
                Amount::from(15u64),
                Amount::zero(),
                source
            )
            .unwrap()
    );

    assert_eq!(
        accounts.get_vault_balance(&vault).unwrap().revealed,
        Amount::from(20u64)
    );
    assert_eq!(
        accounts
            .get_balance_changes(&Test::test_account_address(), 0, 10, None, None, None)
            .unwrap()
            .total,
        2
    );
}
