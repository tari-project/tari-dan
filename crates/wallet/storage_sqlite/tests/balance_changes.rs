//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashSet,
    path::Path,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use diesel::{Connection, QueryableByName, RunQueryDsl, SqliteConnection, sql_query, sql_types::Text};
use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine_types::{
    Utxo,
    resource::Resource,
    resource_container::ResourceContainer,
    substate::{Substate, SubstateDiff, SubstateId},
    vault::Vault,
};
use tari_ootle_common_types::{Epoch, VersionedSubstateIdRef};
use tari_ootle_transaction::{Transaction, args};
use tari_ootle_wallet_sdk::{
    models::{
        BalanceChangeSnapshot,
        BalanceChangeSource,
        BalanceChangeSourceType,
        KeyBranch,
        KeyId,
        OutputStatus,
        StealthOutputModel,
        VaultModel,
        WalletLockId,
    },
    storage::{CommittableStore, ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_template_lib_types::{
    Amount,
    ComponentAddress,
    EncryptedData,
    Metadata,
    ResourceAddress,
    ResourceType,
    SubstateOwnerRule,
    VaultId,
    access_rules::ResourceAccessRules,
    constants::TOKEN_SYMBOL,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
    stealth::SpendAuthorization,
};

fn build_transaction(seed: u64) -> Transaction {
    Transaction::builder_localnet(Epoch(1))
        .allocate_component_address("component")
        .put_last_instruction_output_on_workspace("bucket")
        .call_method("component", "new", args!["bucket"])
        .build_and_seal(&RistrettoSecretKey::from(seed))
}

fn account_address() -> ComponentAddress {
    ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8ffffffff").unwrap()
}

fn vault_address(seed: u8) -> VaultId {
    format!("vault_{seed:064x}").parse().unwrap()
}

fn resource_address(seed: u8) -> ResourceAddress {
    format!("resource_{seed:064x}").parse().unwrap()
}

#[derive(QueryableByName)]
struct SqlText {
    #[diesel(sql_type = Text)]
    sql: String,
}

fn setup_store() -> (SqliteWalletStore, VaultId, VaultId, ResourceAddress, ResourceAddress) {
    setup_store_at(":memory:")
}

fn setup_store_at(path: impl AsRef<Path>) -> (SqliteWalletStore, VaultId, VaultId, ResourceAddress, ResourceAddress) {
    let store = SqliteWalletStore::try_open(path).unwrap();
    store.run_migrations().unwrap();

    let account_address = account_address();
    let first_vault = vault_address(1);
    let second_vault = vault_address(2);
    let first_resource = resource_address(1);
    let second_resource = resource_address(2);
    let owner_public_key = RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(1000)).to_byte_type();
    let mut tx = store.create_write_tx().unwrap();
    tx.accounts_insert(
        Some("test"),
        &account_address,
        KeyId::derived(KeyBranch::ViewOnlyKey, 0),
        Some(KeyId::derived(KeyBranch::Account, 0)),
        &owner_public_key,
        &HashSet::new(),
        Epoch::zero(),
        true,
        true,
    )
    .unwrap();
    for (id, resource_address, resource_type, token_symbol, divisibility) in [
        (
            first_vault,
            first_resource,
            ResourceType::Fungible,
            Some("COIN".to_string()),
            6,
        ),
        (
            second_vault,
            second_resource,
            ResourceType::NonFungible,
            Some("NFT".to_string()),
            0,
        ),
    ] {
        tx.resources_upsert(
            &resource_address,
            &Resource::new(
                resource_type,
                SubstateOwnerRule::None,
                ResourceAccessRules::new(),
                Metadata::from([(TOKEN_SYMBOL, token_symbol.clone().unwrap_or_default())]),
                None,
                None,
                divisibility,
                false,
            ),
        )
        .unwrap();
        tx.vaults_insert(VaultModel {
            account_address,
            id,
            vault_version: 0,
            resource_address,
            resource_type,
            confidential_balance: Amount::zero(),
            revealed_balance: Amount::zero(),
            locked_revealed_balance: Amount::zero(),
            token_symbol,
            divisibility,
        })
        .unwrap();
    }
    tx.commit().unwrap();
    drop(tx);

    (store, first_vault, second_vault, first_resource, second_resource)
}

fn balance_change_snapshot(
    current: &VaultModel,
    vault_version: u64,
    revealed_after: Amount,
    confidential_after: Amount,
) -> BalanceChangeSnapshot {
    BalanceChangeSnapshot {
        account_address: current.account_address,
        vault_address: Some(current.id),
        vault_version: Some(vault_version),
        resource_address: current.resource_address,
        token_symbol: current.token_symbol.clone(),
        divisibility: current.divisibility,
        revealed_before: current.revealed_balance,
        revealed_after,
        confidential_before: current.confidential_balance,
        confidential_after,
    }
}

fn temporary_database_path() -> std::path::PathBuf {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!(
        "tari-ootle-balance-changes-{}-{unique}.sqlite",
        std::process::id()
    ))
}

fn record_change(
    store: &SqliteWalletStore,
    vault_address: VaultId,
    vault_version: u64,
    revealed_after: Amount,
    confidential_after: Amount,
    source: BalanceChangeSource,
) {
    let mut tx = store.create_write_tx().unwrap();
    let current = tx.vaults_get(&vault_address).unwrap();
    assert!(
        tx.balance_changes_insert(
            balance_change_snapshot(&current, vault_version, revealed_after, confidential_after),
            source,
        )
        .unwrap()
    );
    tx.vaults_update(vault_address, vault_version, revealed_after, confidential_after)
        .unwrap();
    tx.commit().unwrap();
}

#[test]
fn records_signed_deltas_metadata_and_filters() {
    let (store, first_vault, second_vault, first_resource, second_resource) = setup_store();
    let transaction = build_transaction(1);
    let transaction_id = transaction.calculate_id();
    let mut tx = store.create_write_tx().unwrap();
    tx.transactions_insert(&transaction, None, &[account_address()], false)
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    record_change(
        &store,
        first_vault,
        1,
        Amount::from(100u64),
        Amount::from(7u64),
        BalanceChangeSource::Transaction { transaction_id },
    );
    record_change(
        &store,
        first_vault,
        2,
        Amount::from(40u64),
        Amount::from(2u64),
        BalanceChangeSource::Scan,
    );
    record_change(
        &store,
        second_vault,
        1,
        Amount::from(1u64),
        Amount::zero(),
        BalanceChangeSource::Recovery,
    );

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.total, 3);
    let changes = page.changes;
    assert_eq!(changes.len(), 3);
    assert!(changes.windows(2).all(|pair| pair[0].id > pair[1].id));

    let nft_change = changes
        .iter()
        .find(|change| change.vault_address == Some(second_vault))
        .unwrap();
    assert_eq!(nft_change.resource_address, second_resource);
    assert_eq!(nft_change.token_symbol.as_deref(), Some("NFT"));
    assert_eq!(nft_change.divisibility, 0);
    assert_eq!(nft_change.revealed_delta, "1");
    assert_eq!(nft_change.source, BalanceChangeSource::Recovery);

    let decrease = changes
        .iter()
        .find(|change| change.vault_address == Some(first_vault) && change.source == BalanceChangeSource::Scan)
        .unwrap();
    assert_eq!(decrease.revealed_delta, "-60");
    assert_eq!(decrease.confidential_delta, "-5");
    assert_eq!(decrease.revealed_after, Amount::from(40u64));

    let by_resource = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, Some(&first_resource), None, None)
        .unwrap();
    assert_eq!(by_resource.changes.len(), 2);
    assert_eq!(by_resource.total, 2);

    let by_transaction = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, Some(&transaction_id), None)
        .unwrap();
    assert_eq!(by_transaction.total, 1);
    assert_eq!(by_transaction.changes[0].revealed_delta, "100");
    assert_eq!(by_transaction.changes[0].confidential_delta, "7");
}

#[test]
fn rejects_zero_changes_deduplicates_transactions_and_paginates_deterministically() {
    let (store, first_vault, _, _, _) = setup_store();
    let transaction = build_transaction(2);
    let transaction_id = transaction.calculate_id();
    let mut tx = store.create_write_tx().unwrap();
    tx.transactions_insert(&transaction, None, &[account_address()], false)
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_write_tx().unwrap();
    let current = tx.vaults_get(&first_vault).unwrap();
    assert!(
        !tx.balance_changes_insert(
            balance_change_snapshot(&current, 1, current.revealed_balance, current.confidential_balance,),
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );
    tx.rollback().unwrap();
    drop(tx);

    record_change(
        &store,
        first_vault,
        1,
        Amount::from(10u64),
        Amount::zero(),
        BalanceChangeSource::Transaction { transaction_id },
    );
    let mut tx = store.create_write_tx().unwrap();
    let current = tx.vaults_get(&first_vault).unwrap();
    assert!(
        !tx.balance_changes_insert(
            balance_change_snapshot(&current, 2, Amount::from(11u64), Amount::zero()),
            BalanceChangeSource::Transaction { transaction_id },
        )
        .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    record_change(
        &store,
        first_vault,
        2,
        Amount::from(20u64),
        Amount::zero(),
        BalanceChangeSource::Scan,
    );
    record_change(
        &store,
        first_vault,
        3,
        Amount::from(30u64),
        Amount::zero(),
        BalanceChangeSource::Recovery,
    );

    let mut tx = store.create_read_tx().unwrap();
    let first_page = tx
        .balance_changes_get_page_by_account(
            &account_address(),
            0,
            2,
            None,
            None,
            Some(BalanceChangeSourceType::Scan),
        )
        .unwrap();
    assert_eq!(first_page.total, 1);
    assert_eq!(first_page.changes.len(), 1);
    let first_page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 2, None, None, None)
        .unwrap();
    let second_page = tx
        .balance_changes_get_page_by_account(&account_address(), 2, 2, None, None, None)
        .unwrap();
    assert_eq!(first_page.total, 3);
    assert_eq!(second_page.total, 3);
    assert_eq!(first_page.changes.len(), 2);
    assert_eq!(second_page.changes.len(), 1);
    assert!(first_page.changes[0].id > first_page.changes[1].id);
    assert!(first_page.changes[1].id > second_page.changes[0].id);
}

#[test]
fn records_account_resource_change_without_vault() {
    let (store, _, _, _, _) = setup_store();
    let stealth_resource = resource_address(3);
    let mut tx = store.create_write_tx().unwrap();
    tx.resources_upsert(
        &stealth_resource,
        &Resource::new(
            ResourceType::Stealth,
            SubstateOwnerRule::None,
            ResourceAccessRules::new(),
            Metadata::from([(TOKEN_SYMBOL, "STEALTH".to_string())]),
            None,
            None,
            6,
            false,
        ),
    )
    .unwrap();
    assert!(
        tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account_address(),
                vault_address: None,
                vault_version: None,
                resource_address: stealth_resource,
                token_symbol: Some("STEALTH".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::zero(),
                confidential_before: Amount::zero(),
                confidential_after: Amount::from(33u64),
            },
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, Some(&stealth_resource), None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.changes[0].vault_address, None);
    assert_eq!(page.changes[0].confidential_delta, "33");
    assert_eq!(page.changes[0].token_symbol.as_deref(), Some("STEALTH"));
}

#[test]
fn latest_account_resource_history_ignores_vault_backed_rows() {
    let (store, first_vault, _, first_resource, _) = setup_store();
    let mut tx = store.create_write_tx().unwrap();
    assert!(
        tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account_address(),
                vault_address: None,
                vault_version: None,
                resource_address: first_resource,
                token_symbol: Some("COIN".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::zero(),
                confidential_before: Amount::zero(),
                confidential_after: Amount::from(33u64),
            },
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    record_change(
        &store,
        first_vault,
        1,
        Amount::from(100u64),
        Amount::zero(),
        BalanceChangeSource::Scan,
    );

    let mut tx = store.create_read_tx().unwrap();
    let latest = tx
        .balance_changes_get_latest_by_account_resource(&account_address(), &first_resource)
        .unwrap()
        .unwrap();
    assert_eq!(latest.vault_address, None);
    assert_eq!(latest.confidential_after, Amount::from(33u64));
}

#[test]
fn read_indexes_match_address_scoped_history_queries() {
    let path = temporary_database_path();
    let (store, _, _, _, _) = setup_store_at(&path);
    drop(store);

    let mut connection = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    let indexes = sql_query(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name IN ( \
         'account_balance_changes_account_created_idx', 'account_balance_changes_account_resource_created_idx' ) \
         ORDER BY name",
    )
    .load::<SqlText>(&mut connection)
    .unwrap();
    assert_eq!(indexes.len(), 2);
    assert!(indexes[0].sql.contains("(account_address, created_at DESC, id DESC)"));
    assert!(
        indexes[1]
            .sql
            .contains("(account_address, resource_address, created_at DESC, id DESC)")
    );
    drop(connection);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn attributes_only_the_exact_vault_version_across_round_trips_and_recovery() {
    let (store, first_vault, _, _, _) = setup_store();
    let first_transaction = build_transaction(3);
    let first_transaction_id = first_transaction.calculate_id();
    let second_transaction = build_transaction(4);
    let second_transaction_id = second_transaction.calculate_id();
    let mut tx = store.create_write_tx().unwrap();
    tx.transactions_insert(&first_transaction, None, &[account_address()], false)
        .unwrap();
    tx.transactions_insert(&second_transaction, None, &[account_address()], false)
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    for (version, amount) in [(1, 10u64), (2, 20), (3, 10)] {
        record_change(
            &store,
            first_vault,
            version,
            Amount::from(amount),
            Amount::zero(),
            BalanceChangeSource::Scan,
        );
    }

    let mut tx = store.create_write_tx().unwrap();
    assert!(
        tx.balance_changes_attribute_transaction(&first_vault, 1, first_transaction_id)
            .unwrap()
    );
    assert!(
        !tx.balance_changes_attribute_transaction(&first_vault, 99, second_transaction_id)
            .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let first_transaction_page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, Some(&first_transaction_id), None)
        .unwrap();
    assert_eq!(first_transaction_page.total, 1);
    drop(tx);
    record_change(
        &store,
        first_vault,
        4,
        Amount::from(30u64),
        Amount::zero(),
        BalanceChangeSource::Recovery,
    );

    let mut tx = store.create_write_tx().unwrap();
    assert!(
        tx.balance_changes_attribute_transaction(&first_vault, 4, second_transaction_id)
            .unwrap()
    );
    assert!(
        tx.balance_changes_attribute_transaction(&first_vault, 999, second_transaction_id)
            .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let transaction_page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, Some(&second_transaction_id), None)
        .unwrap();
    assert_eq!(transaction_page.total, 1);
    assert_eq!(transaction_page.changes[0].revealed_delta, "20");
    assert_eq!(transaction_page.changes[0].source, BalanceChangeSource::Transaction {
        transaction_id: second_transaction_id,
    });
}

#[test]
fn same_version_scan_updates_after_balance_and_deletes_net_zero_row() {
    let (store, first_vault, _, _, _) = setup_store();
    record_change(
        &store,
        first_vault,
        1,
        Amount::zero(),
        Amount::from(10u64),
        BalanceChangeSource::Scan,
    );

    let mut tx = store.create_write_tx().unwrap();
    let current = tx.vaults_get(&first_vault).unwrap();
    assert!(
        tx.balance_changes_insert(
            balance_change_snapshot(&current, 1, Amount::zero(), Amount::from(15u64)),
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );
    tx.vaults_update(first_vault, 1, Amount::zero(), Amount::from(15u64))
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.changes[0].confidential_before, Amount::zero());
    assert_eq!(page.changes[0].confidential_after, Amount::from(15u64));
    assert_eq!(page.changes[0].confidential_delta, "15");
    let vault = tx.vaults_get(&first_vault).unwrap();
    assert_eq!(vault.vault_version, 1);
    assert_eq!(vault.confidential_balance, Amount::from(15u64));
    drop(tx);

    let mut tx = store.create_write_tx().unwrap();
    let current = tx.vaults_get(&first_vault).unwrap();
    assert!(
        tx.balance_changes_insert(
            balance_change_snapshot(&current, 1, Amount::zero(), Amount::zero()),
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );
    tx.vaults_update(first_vault, 1, Amount::zero(), Amount::zero())
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.total, 0);
    let vault = tx.vaults_get(&first_vault).unwrap();
    assert_eq!(vault.confidential_balance, Amount::zero());
}

#[test]
fn same_version_scan_does_not_clobber_transaction_row() {
    let (store, first_vault, _, _, _) = setup_store();
    let transaction = build_transaction(5);
    let transaction_id = transaction.calculate_id();
    let mut tx = store.create_write_tx().unwrap();
    tx.transactions_insert(&transaction, None, &[account_address()], false)
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    record_change(
        &store,
        first_vault,
        1,
        Amount::from(10u64),
        Amount::zero(),
        BalanceChangeSource::Transaction { transaction_id },
    );

    let mut tx = store.create_write_tx().unwrap();
    let current = tx.vaults_get(&first_vault).unwrap();
    assert!(
        !tx.balance_changes_insert(
            balance_change_snapshot(&current, 1, Amount::from(20u64), Amount::zero()),
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );
    tx.vaults_update(first_vault, 1, Amount::from(20u64), Amount::zero())
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.changes[0].source, BalanceChangeSource::Transaction {
        transaction_id
    });
    assert_eq!(page.changes[0].revealed_after, Amount::from(10u64));
    let vault = tx.vaults_get(&first_vault).unwrap();
    assert_eq!(vault.revealed_balance, Amount::from(20u64));
}

// A finalized spend records its revealed decrease (at lock-finalize) and the account refresh later records
// the same transaction's confidential effect; both must converge onto one row rather than the second being
// dropped by the "never clobber a transaction row" rule.
#[test]
fn same_transaction_merges_revealed_finalize_and_confidential_refresh() {
    let (store, first_vault, _, _, _) = setup_store();
    let transaction = build_transaction(6);
    let transaction_id = transaction.calculate_id();
    let other = build_transaction(7);
    let other_id = other.calculate_id();
    let mut tx = store.create_write_tx().unwrap();
    tx.transactions_insert(&transaction, None, &[account_address()], false)
        .unwrap();
    tx.transactions_insert(&other, None, &[account_address()], false)
        .unwrap();
    tx.commit().unwrap();
    drop(tx);

    let snapshot = |revealed_before: u64, revealed_after: u64, confidential_before: u64, confidential_after: u64| {
        BalanceChangeSnapshot {
            account_address: account_address(),
            vault_address: Some(first_vault),
            vault_version: Some(1),
            resource_address: resource_address(1),
            token_symbol: Some("COIN".to_string()),
            divisibility: 6,
            revealed_before: Amount::from(revealed_before),
            revealed_after: Amount::from(revealed_after),
            confidential_before: Amount::from(confidential_before),
            confidential_after: Amount::from(confidential_after),
        }
    };

    // Lock-finalize records the revealed spend (100 -> 90), confidential untouched.
    let mut tx = store.create_write_tx().unwrap();
    assert!(
        tx.balance_changes_insert(snapshot(100, 90, 0, 0), BalanceChangeSource::Transaction {
            transaction_id
        })
        .unwrap()
    );
    // The account refresh records the same transaction's confidential effect (0 -> 50) at the same version.
    assert!(
        tx.balance_changes_insert(snapshot(90, 90, 0, 50), BalanceChangeSource::Transaction {
            transaction_id
        })
        .unwrap()
    );
    // A different transaction at the same (vault, version) must not overwrite the row.
    assert!(
        !tx.balance_changes_insert(snapshot(90, 80, 50, 50), BalanceChangeSource::Transaction {
            transaction_id: other_id
        })
        .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    let change = &page.changes[0];
    assert_eq!(change.source, BalanceChangeSource::Transaction { transaction_id });
    assert_eq!(change.revealed_before, Amount::from(100u64));
    assert_eq!(change.revealed_after, Amount::from(90u64));
    assert_eq!(change.revealed_delta, "-10");
    assert_eq!(change.confidential_after, Amount::from(50u64));
    assert_eq!(change.confidential_delta, "50");
}

fn stealth_output(
    account: ComponentAddress,
    resource_address: ResourceAddress,
    value: u64,
    seed: u8,
    status: OutputStatus,
    lock_id: Option<WalletLockId>,
) -> StealthOutputModel {
    StealthOutputModel {
        owner_account: account,
        resource_address,
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
        status,
        is_burnt: false,
        is_frozen: false,
        is_on_chain: !matches!(status, OutputStatus::LockedUnconfirmed),
        is_condition_spendable: true,
        lock_id,
    }
}

fn upsert_stealth_resource(tx: &mut impl WalletStoreWriter, resource_address: &ResourceAddress) {
    tx.resources_upsert(
        resource_address,
        &Resource::new(
            ResourceType::Stealth,
            SubstateOwnerRule::None,
            ResourceAccessRules::new(),
            Metadata::from([(TOKEN_SYMBOL, "STEALTH".to_string())]),
            None,
            None,
            6,
            false,
        ),
    )
    .unwrap();
}

// A confidential spend from a vault-less account must be recorded at lock-finalize as a
// transaction-sourced change so the spend is attributed to its transaction; the scanner's later
// pass over the same balance then has nothing new to record.
#[test]
fn lock_finalize_records_confidential_spend_for_vaultless_account() {
    let (store, _, _, _, _) = setup_store();
    let stealth_resource = resource_address(3);
    let account = account_address();
    let transaction = build_transaction(8);
    let transaction_id = transaction.calculate_id();

    let mut tx = store.create_write_tx().unwrap();
    upsert_stealth_resource(&mut tx, &stealth_resource);
    tx.transactions_insert(&transaction, None, &[account], false).unwrap();

    // Prior received balance recorded by an earlier scan.
    assert!(
        tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account,
                vault_address: None,
                vault_version: None,
                resource_address: stealth_resource,
                token_symbol: Some("STEALTH".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::zero(),
                confidential_before: Amount::zero(),
                confidential_after: Amount::from(1_000_000u64),
            },
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );

    let spent = stealth_output(account, stealth_resource, 1_000_000, 7, OutputStatus::Unspent, None);
    tx.stealth_outputs_insert(&spent).unwrap();
    let lock_id = tx.locks_create(None).unwrap();
    tx.locks_link_transaction(lock_id, transaction_id).unwrap();
    tx.stealth_outputs_lock_many(&stealth_resource, &[&spent.commitment], lock_id)
        .unwrap();
    let change = stealth_output(
        account,
        stealth_resource,
        989_343,
        8,
        OutputStatus::LockedUnconfirmed,
        Some(lock_id),
    );
    tx.stealth_outputs_insert(&change).unwrap();

    let mut diff = SubstateDiff::new();
    diff.down(SubstateId::Utxo(spent.to_utxo_address()), 0);
    diff.up(
        SubstateId::Utxo(change.to_utxo_address()),
        Substate::new(0, Utxo {
            output: None,
            is_frozen: false,
        }),
    );
    tx.locks_unlock_finalized(lock_id, &diff).unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account, 0, 10, Some(&stealth_resource), None, None)
        .unwrap();
    assert_eq!(page.total, 2);
    let spend_change = &page.changes[0];
    assert_eq!(spend_change.source, BalanceChangeSource::Transaction { transaction_id });
    assert_eq!(spend_change.vault_address, None);
    assert_eq!(spend_change.confidential_before, Amount::from(1_000_000u64));
    assert_eq!(spend_change.confidential_after, Amount::from(989_343u64));
    assert_eq!(spend_change.confidential_delta, "-10657");
    assert_eq!(spend_change.revealed_delta, "0");
    drop(tx);

    // The scanner later re-derives the same balance from the outputs; that must be a no-op.
    let mut tx = store.create_write_tx().unwrap();
    assert!(
        !tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account,
                vault_address: None,
                vault_version: None,
                resource_address: stealth_resource,
                token_symbol: Some("STEALTH".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::zero(),
                confidential_before: Amount::from(989_343u64),
                confidential_after: Amount::from(989_343u64),
            },
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );
    tx.rollback().unwrap();
}

// A confidential spend from an account whose stealth vault is untouched by the transaction records a
// no-vault transaction row against the vault's balance and keeps the vault in sync, so the scanner's
// later pass re-derives an unchanged balance.
#[test]
fn lock_finalize_records_confidential_spend_for_vault_backed_resource() {
    let (store, _, _, _, _) = setup_store();
    let stealth_resource = resource_address(3);
    let stealth_vault = vault_address(3);
    let account = account_address();
    let transaction = build_transaction(9);
    let transaction_id = transaction.calculate_id();

    let mut tx = store.create_write_tx().unwrap();
    upsert_stealth_resource(&mut tx, &stealth_resource);
    tx.vaults_insert(VaultModel {
        account_address: account,
        id: stealth_vault,
        vault_version: 0,
        resource_address: stealth_resource,
        resource_type: ResourceType::Stealth,
        confidential_balance: Amount::from(1_000_000u64),
        revealed_balance: Amount::from(500u64),
        locked_revealed_balance: Amount::zero(),
        token_symbol: Some("STEALTH".to_string()),
        divisibility: 6,
    })
    .unwrap();
    tx.transactions_insert(&transaction, None, &[account], false).unwrap();

    let spent = stealth_output(account, stealth_resource, 1_000_000, 9, OutputStatus::Unspent, None);
    tx.stealth_outputs_insert(&spent).unwrap();
    let lock_id = tx.locks_create(None).unwrap();
    tx.locks_link_transaction(lock_id, transaction_id).unwrap();
    tx.stealth_outputs_lock_many(&stealth_resource, &[&spent.commitment], lock_id)
        .unwrap();
    let change = stealth_output(
        account,
        stealth_resource,
        900_000,
        10,
        OutputStatus::LockedUnconfirmed,
        Some(lock_id),
    );
    tx.stealth_outputs_insert(&change).unwrap();

    let mut diff = SubstateDiff::new();
    diff.down(SubstateId::Utxo(spent.to_utxo_address()), 0);
    diff.up(
        SubstateId::Utxo(change.to_utxo_address()),
        Substate::new(0, Utxo {
            output: None,
            is_frozen: false,
        }),
    );
    tx.locks_unlock_finalized(lock_id, &diff).unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account, 0, 10, Some(&stealth_resource), None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    let change_row = &page.changes[0];
    assert_eq!(change_row.source, BalanceChangeSource::Transaction { transaction_id });
    assert_eq!(change_row.vault_address, None);
    assert_eq!(change_row.confidential_before, Amount::from(1_000_000u64));
    assert_eq!(change_row.confidential_after, Amount::from(900_000u64));
    assert_eq!(change_row.revealed_delta, "0");
    let vault = tx.vaults_get(&stealth_vault).unwrap();
    assert_eq!(vault.confidential_balance, Amount::from(900_000u64));
    assert_eq!(vault.revealed_balance, Amount::from(500u64));
}

// A transaction that creates the account's vault for the resource (e.g. a self-transfer of UTXOs into a
// revealed balance) keys the confidential movement on that vault at its post-transaction version, so the
// account refresh's revealed record merges onto the same row: one row holds the transaction's net effect.
#[allow(clippy::too_many_lines)]
#[test]
fn lock_finalize_records_on_vault_created_by_transaction() {
    let (store, _, _, _, _) = setup_store();
    let stealth_resource = resource_address(3);
    let new_vault = vault_address(4);
    let account = account_address();
    let transaction = build_transaction(10);
    let transaction_id = transaction.calculate_id();

    let mut tx = store.create_write_tx().unwrap();
    upsert_stealth_resource(&mut tx, &stealth_resource);
    tx.transactions_insert(&transaction, None, &[account], false).unwrap();

    // Prior received balance recorded by an earlier scan.
    assert!(
        tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account,
                vault_address: None,
                vault_version: None,
                resource_address: stealth_resource,
                token_symbol: Some("STEALTH".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::zero(),
                confidential_before: Amount::zero(),
                confidential_after: Amount::from(1_000_000u64),
            },
            BalanceChangeSource::Scan,
        )
        .unwrap()
    );

    let spent = stealth_output(account, stealth_resource, 1_000_000, 11, OutputStatus::Unspent, None);
    tx.stealth_outputs_insert(&spent).unwrap();
    let lock_id = tx.locks_create(None).unwrap();
    tx.locks_link_transaction(lock_id, transaction_id).unwrap();
    tx.stealth_outputs_lock_many(&stealth_resource, &[&spent.commitment], lock_id)
        .unwrap();
    let change = stealth_output(
        account,
        stealth_resource,
        989_343,
        12,
        OutputStatus::LockedUnconfirmed,
        Some(lock_id),
    );
    tx.stealth_outputs_insert(&change).unwrap();

    // The diff commit links the transaction's new vault to the account before locks are finalized.
    tx.substates_upsert_root(
        VersionedSubstateIdRef::new(&SubstateId::Component(account), 1),
        HashSet::new(),
        None,
        None,
    )
    .unwrap();
    tx.substates_upsert_child(
        &SubstateId::Component(account),
        VersionedSubstateIdRef::new(&SubstateId::Vault(new_vault), 0),
        HashSet::new(),
    )
    .unwrap();

    let mut diff = SubstateDiff::new();
    diff.down(SubstateId::Utxo(spent.to_utxo_address()), 0);
    diff.up(
        SubstateId::Utxo(change.to_utxo_address()),
        Substate::new(0, Utxo {
            output: None,
            is_frozen: false,
        }),
    );
    diff.up(
        SubstateId::Vault(new_vault),
        Substate::new(
            0,
            Vault::new(ResourceContainer::stealth(stealth_resource, Amount::from(10_000u64))),
        ),
    );
    tx.locks_unlock_finalized(lock_id, &diff).unwrap();

    // The account refresh later records the same transaction's revealed effect at the same vault version;
    // it must merge onto the finalize row.
    assert!(
        tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account,
                vault_address: Some(new_vault),
                vault_version: Some(0),
                resource_address: stealth_resource,
                token_symbol: Some("STEALTH".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::from(10_000u64),
                confidential_before: Amount::from(989_343u64),
                confidential_after: Amount::from(989_343u64),
            },
            BalanceChangeSource::Transaction { transaction_id },
        )
        .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account, 0, 10, Some(&stealth_resource), None, None)
        .unwrap();
    assert_eq!(page.total, 2);
    let spend_change = &page.changes[0];
    assert_eq!(spend_change.source, BalanceChangeSource::Transaction { transaction_id });
    assert_eq!(spend_change.vault_address, Some(new_vault));
    assert_eq!(spend_change.confidential_before, Amount::from(1_000_000u64));
    assert_eq!(spend_change.confidential_after, Amount::from(989_343u64));
    assert_eq!(spend_change.confidential_delta, "-10657");
    assert_eq!(spend_change.revealed_before, Amount::zero());
    assert_eq!(spend_change.revealed_after, Amount::from(10_000u64));
    assert_eq!(spend_change.revealed_delta, "10000");
}

// The vault substate in the finalized diff is the authoritative post-transaction revealed balance: the
// wallet's revealed lock is only a reservation, and a transaction may withdraw less than was locked (e.g.
// only the fee). The finalize record and the wallet vault must both follow the substate, or the wallet
// balance diverges from chain and the next transaction's row absorbs the difference.
#[test]
fn lock_finalize_records_revealed_movement_from_diff_not_lock_amount() {
    let (store, _, _, _, _) = setup_store();
    let stealth_resource = resource_address(3);
    let stealth_vault = vault_address(6);
    let account = account_address();
    let transaction = build_transaction(13);
    let transaction_id = transaction.calculate_id();

    let mut tx = store.create_write_tx().unwrap();
    upsert_stealth_resource(&mut tx, &stealth_resource);
    tx.vaults_insert(VaultModel {
        account_address: account,
        id: stealth_vault,
        vault_version: 1,
        resource_address: stealth_resource,
        resource_type: ResourceType::Stealth,
        confidential_balance: Amount::from(5_000_000u64),
        revealed_balance: Amount::from(2_000_000u64),
        locked_revealed_balance: Amount::zero(),
        token_symbol: Some("STEALTH".to_string()),
        divisibility: 6,
    })
    .unwrap();
    tx.transactions_insert(&transaction, None, &[account], false).unwrap();

    let lock_id = tx.locks_create(None).unwrap();
    tx.locks_link_transaction(lock_id, transaction_id).unwrap();
    tx.vaults_lock_revealed_funds(lock_id, &stealth_vault, Amount::from(1_000_255u64))
        .unwrap();

    // The transaction only moves the fee out of the vault's revealed balance.
    let mut diff = SubstateDiff::new();
    diff.up(
        SubstateId::Vault(stealth_vault),
        Substate::new(
            2,
            Vault::new(ResourceContainer::stealth(stealth_resource, Amount::from(1_999_745u64))),
        ),
    );
    tx.locks_unlock_finalized(lock_id, &diff).unwrap();
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account, 0, 10, Some(&stealth_resource), None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    let change = &page.changes[0];
    assert_eq!(change.source, BalanceChangeSource::Transaction { transaction_id });
    assert_eq!(change.vault_address, Some(stealth_vault));
    assert_eq!(change.revealed_before, Amount::from(2_000_000u64));
    assert_eq!(change.revealed_after, Amount::from(1_999_745u64));
    assert_eq!(change.revealed_delta, "-255");
    let vault = tx.vaults_get(&stealth_vault).unwrap();
    assert_eq!(vault.revealed_balance, Amount::from(1_999_745u64));
    assert_eq!(vault.vault_version, 2);
}

// The unique (account, resource, transaction) index spans both key shapes: when a transaction's effect is
// first recorded as a vault-less row, a later vault-keyed record of the same transaction (the account
// refresh after the vault appears) must fill its untouched balance dimension on that row rather than being
// dropped — while a re-statement of an already-recorded dimension is a replay and must not modify it.
#[test]
fn same_transaction_vault_record_fills_dimension_on_no_vault_row() {
    let (store, _, _, _, _) = setup_store();
    let stealth_resource = resource_address(3);
    let new_vault = vault_address(5);
    let account = account_address();
    let transaction = build_transaction(12);
    let transaction_id = transaction.calculate_id();

    let mut tx = store.create_write_tx().unwrap();
    upsert_stealth_resource(&mut tx, &stealth_resource);
    tx.transactions_insert(&transaction, None, &[account], false).unwrap();

    // Finalize records the confidential spend before the wallet knows the transaction created a vault.
    assert!(
        tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account,
                vault_address: None,
                vault_version: None,
                resource_address: stealth_resource,
                token_symbol: Some("STEALTH".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::zero(),
                confidential_before: Amount::from(1_000_000u64),
                confidential_after: Amount::from(989_000u64),
            },
            BalanceChangeSource::Transaction { transaction_id },
        )
        .unwrap()
    );
    // The account refresh then records the revealed deposit against the new vault.
    assert!(
        tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account,
                vault_address: Some(new_vault),
                vault_version: Some(0),
                resource_address: stealth_resource,
                token_symbol: Some("STEALTH".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::from(10_000u64),
                confidential_before: Amount::from(989_000u64),
                confidential_after: Amount::from(989_000u64),
            },
            BalanceChangeSource::Transaction { transaction_id },
        )
        .unwrap()
    );
    // A replayed vault-keyed re-statement of the confidential dimension must not modify the row.
    assert!(
        !tx.balance_changes_insert(
            BalanceChangeSnapshot {
                account_address: account,
                vault_address: Some(new_vault),
                vault_version: Some(1),
                resource_address: stealth_resource,
                token_symbol: Some("STEALTH".to_string()),
                divisibility: 6,
                revealed_before: Amount::zero(),
                revealed_after: Amount::zero(),
                confidential_before: Amount::from(1_000_000u64),
                confidential_after: Amount::from(500u64),
            },
            BalanceChangeSource::Transaction { transaction_id },
        )
        .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account, 0, 10, Some(&stealth_resource), None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    let change = &page.changes[0];
    assert_eq!(change.vault_address, None);
    assert_eq!(change.confidential_delta, "-11000");
    assert_eq!(change.revealed_delta, "10000");
}

// A transaction output for the sender that is only discovered after the finalize record (it is not held by
// the lock) extends the same transaction row's after-balances, so the row holds the transaction's net
// effect and the scanner has nothing left to record.
#[test]
fn same_transaction_no_vault_records_merge_into_net_effect() {
    let (store, _, _, _, _) = setup_store();
    let stealth_resource = resource_address(3);
    let account = account_address();
    let transaction = build_transaction(11);
    let transaction_id = transaction.calculate_id();

    let mut tx = store.create_write_tx().unwrap();
    upsert_stealth_resource(&mut tx, &stealth_resource);
    tx.transactions_insert(&transaction, None, &[account], false).unwrap();

    let snapshot = |revealed: (u64, u64), confidential: (u64, u64)| BalanceChangeSnapshot {
        account_address: account,
        vault_address: None,
        vault_version: None,
        resource_address: stealth_resource,
        token_symbol: Some("STEALTH".to_string()),
        divisibility: 6,
        revealed_before: Amount::from(revealed.0),
        revealed_after: Amount::from(revealed.1),
        confidential_before: Amount::from(confidential.0),
        confidential_after: Amount::from(confidential.1),
    };

    // Lock-finalize records the spend before the transaction's output to self is discovered.
    assert!(
        tx.balance_changes_insert(
            snapshot((0, 0), (1_000_000, 900_000)),
            BalanceChangeSource::Transaction { transaction_id }
        )
        .unwrap()
    );
    // The discovered output extends the row to the transaction's net effect.
    assert!(
        tx.balance_changes_insert(snapshot((0, 0), (900_000, 989_343)), BalanceChangeSource::Transaction {
            transaction_id
        })
        .unwrap()
    );
    // A re-derivation of the same balances is a no-op.
    assert!(
        !tx.balance_changes_insert(snapshot((0, 0), (989_343, 989_343)), BalanceChangeSource::Transaction {
            transaction_id
        })
        .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account, 0, 10, Some(&stealth_resource), None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    let change = &page.changes[0];
    assert_eq!(change.confidential_before, Amount::from(1_000_000u64));
    assert_eq!(change.confidential_after, Amount::from(989_343u64));
    assert_eq!(change.confidential_delta, "-10657");
    drop(tx);

    // A record that returns the balance to its starting point nets the row to zero and removes it.
    let mut tx = store.create_write_tx().unwrap();
    assert!(
        tx.balance_changes_insert(
            snapshot((0, 0), (989_343, 1_000_000)),
            BalanceChangeSource::Transaction { transaction_id }
        )
        .unwrap()
    );
    tx.commit().unwrap();
    drop(tx);

    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account, 0, 10, Some(&stealth_resource), None, None)
        .unwrap();
    assert_eq!(page.total, 0);
}

#[test]
fn history_keeps_snapshot_metadata_after_live_rows_are_changed_or_deleted() {
    let path = temporary_database_path();
    let (store, first_vault, _, _, _) = setup_store_at(&path);
    record_change(
        &store,
        first_vault,
        1,
        Amount::from(100u64),
        Amount::zero(),
        BalanceChangeSource::Scan,
    );
    drop(store);

    let mut connection = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    sql_query("UPDATE vaults SET token_symbol = 'CHANGED', divisibility = 9")
        .execute(&mut connection)
        .unwrap();
    drop(connection);

    let store = SqliteWalletStore::try_open(&path).unwrap();
    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.changes[0].token_symbol.as_deref(), Some("COIN"));
    assert_eq!(page.changes[0].divisibility, 6);
    drop(tx);
    drop(store);

    let mut connection = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    sql_query("PRAGMA foreign_keys = ON").execute(&mut connection).unwrap();
    sql_query("DELETE FROM vaults").execute(&mut connection).unwrap();
    sql_query("DELETE FROM accounts").execute(&mut connection).unwrap();
    drop(connection);

    let store = SqliteWalletStore::try_open(&path).unwrap();
    let mut tx = store.create_read_tx().unwrap();
    let page = tx
        .balance_changes_get_page_by_account(&account_address(), 0, 10, None, None, None)
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.changes[0].vault_address, Some(first_vault));
    drop(tx);
    drop(store);
    std::fs::remove_file(path).unwrap();
}
