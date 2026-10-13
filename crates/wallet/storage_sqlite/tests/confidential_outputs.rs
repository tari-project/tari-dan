//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Finalizing a lock settles its confidential outputs against the transaction's diff: an input is
//! spent only if the diff downs it and a change output is kept only if the diff ups it. Everything
//! else the lock holds is released, as for a rejected transaction.

use std::{collections::HashSet, str::FromStr};

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine_types::{
    confidential_output::ConfidentialOutput,
    crypto::OutputBody,
    substate::{Substate, SubstateDiff, SubstateId},
};
use tari_ootle_common_types::{Epoch, SubstateVersion, optional::IsNotFoundError};
use tari_ootle_wallet_sdk::{
    models::{ConfidentialOutputModel, KeyBranch, KeyId, OutputStatus, VaultModel, WalletLockId},
    storage::{CommittableStore, ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
use tari_template_lib_types::{
    Amount,
    ComponentAddress,
    ConfidentialOutputAddress,
    EncryptedData,
    ResourceAddress,
    ResourceType,
    VaultId,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
};

fn account_address() -> ComponentAddress {
    ComponentAddress::from_str("component_91bef6af37bfb39b20260275c37a9e8acfc0517127284cd8f05944c8ffffffff").unwrap()
}

fn vault_address() -> VaultId {
    format!("vault_{:064x}", 1).parse().unwrap()
}

fn resource_address() -> ResourceAddress {
    format!("resource_{:064x}", 1).parse().unwrap()
}

fn commitment(seed: u8) -> PedersenCommitmentBytes {
    PedersenCommitmentBytes::from_array([seed; PedersenCommitmentBytes::length()])
}

fn output_address(seed: u8) -> SubstateId {
    SubstateId::ConfidentialOutput(ConfidentialOutputAddress::new(resource_address(), commitment(seed)))
}

fn confidential_output(
    seed: u8,
    value: u64,
    status: OutputStatus,
    lock_id: Option<WalletLockId>,
) -> ConfidentialOutputModel {
    ConfidentialOutputModel {
        account_address: account_address(),
        vault_id: vault_address(),
        commitment: commitment(seed),
        value: Amount::from(value),
        sender_public_nonce: None,
        view_only_key_id: KeyId::derived(KeyBranch::ViewOnlyKey, 0),
        owner_key_id: Some(KeyId::derived(KeyBranch::Account, 0)),
        encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
        public_asset_tag: None,
        memo: None,
        status,
        lock_id,
    }
}

fn confidential_output_substate() -> Substate {
    Substate::new(SubstateVersion::ZERO, ConfidentialOutput {
        output: OutputBody {
            public_nonce: RistrettoPublicKeyBytes::default(),
            encrypted_data: EncryptedData::try_from(vec![0; EncryptedData::min_size()]).unwrap(),
            minimum_value_promise: 0,
            viewable_balance: None,
        },
        is_frozen: false,
    })
}

const INPUT: u8 = 1;
const CHANGE: u8 = 2;

/// Creates a store holding one spendable confidential output, then locks it as the input of a
/// transfer and records that transfer's change output under the same lock.
fn setup_locked_transfer() -> (SqliteWalletStore, WalletLockId) {
    let store = SqliteWalletStore::try_open(":memory:").unwrap();
    store.run_migrations().unwrap();

    let owner_public_key = RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(1000)).to_byte_type();
    let mut tx = store.create_write_tx().unwrap();
    tx.accounts_insert(
        Some("test"),
        &account_address(),
        KeyId::derived(KeyBranch::ViewOnlyKey, 0),
        Some(KeyId::derived(KeyBranch::Account, 0)),
        &owner_public_key,
        &HashSet::new(),
        Epoch::zero(),
        true,
        true,
    )
    .unwrap();
    tx.vaults_insert(VaultModel {
        account_address: account_address(),
        id: vault_address(),
        vault_version: SubstateVersion::ZERO,
        resource_address: resource_address(),
        resource_type: ResourceType::Confidential,
        confidential_balance: Amount::from(1_000u64),
        revealed_balance: Amount::zero(),
        locked_revealed_balance: Amount::zero(),
        token_symbol: None,
        divisibility: 6,
    })
    .unwrap();
    tx.confidential_outputs_insert(confidential_output(INPUT, 1_000, OutputStatus::Unspent, None))
        .unwrap();

    let lock_id = tx.locks_create(None).unwrap();
    let locked = tx
        .confidential_outputs_lock_smallest_amount(&vault_address(), lock_id)
        .unwrap();
    assert_eq!(locked.commitment, commitment(INPUT));
    tx.confidential_outputs_insert(confidential_output(
        CHANGE,
        400,
        OutputStatus::LockedUnconfirmed,
        Some(lock_id),
    ))
    .unwrap();
    tx.commit().unwrap();
    drop(tx);

    (store, lock_id)
}

fn finalize(store: &SqliteWalletStore, lock_id: WalletLockId, diff: &SubstateDiff) {
    let mut tx = store.create_write_tx().unwrap();
    tx.locks_unlock_finalized(lock_id, diff).unwrap();
    tx.commit().unwrap();
}

fn output_status(store: &SqliteWalletStore, seed: u8) -> Option<&'static str> {
    let mut tx = store.create_read_tx().unwrap();
    match tx.confidential_outputs_get_by_commitment(&vault_address(), &commitment(seed)) {
        Ok(output) => {
            assert!(output.lock_id.is_none(), "a finalized output must not keep its lock");
            Some(output.status.as_key_str())
        },
        Err(err) if err.is_not_found_error() => None,
        Err(err) => panic!("unexpected error: {err}"),
    }
}

#[test]
fn finalize_spends_downed_inputs_and_confirms_upped_change() {
    let (store, lock_id) = setup_locked_transfer();

    let mut diff = SubstateDiff::new();
    diff.down(output_address(INPUT), SubstateVersion::ZERO);
    diff.up(output_address(CHANGE), confidential_output_substate());
    finalize(&store, lock_id, &diff);

    assert_eq!(output_status(&store, INPUT), Some(OutputStatus::Spent.as_key_str()));
    assert_eq!(output_status(&store, CHANGE), Some(OutputStatus::Unspent.as_key_str()));
}

#[test]
fn finalize_releases_outputs_the_diff_does_not_touch() {
    // An accepted result whose diff leaves the wallet's confidential outputs alone, e.g. a
    // transaction that paid its fee but whose main intent was rejected.
    let (store, lock_id) = setup_locked_transfer();

    finalize(&store, lock_id, &SubstateDiff::new());

    assert_eq!(
        output_status(&store, INPUT),
        Some(OutputStatus::Unspent.as_key_str()),
        "an input the diff does not down is still spendable"
    );
    assert_eq!(
        output_status(&store, CHANGE),
        None,
        "a change output the diff does not up was never created"
    );
}
