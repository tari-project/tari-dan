//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod support;

use tari_engine_types::limits;
use tari_ootle_common_types::Epoch;
use tari_ootle_wallet_sdk::{
    apis::stealth_outputs::StealthOutputsApiError,
    models::{KeyBranch, KeyId, OutputStatus, StealthOutputModel, WalletLockId},
};
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    EncryptedData,
    Hash32,
    UtxoAddress,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
    stealth::SpendAuthorization,
};

use crate::support::{Test, resource_address_from_seed};

/// A key-path, on-chain, wallet-owned, currently spendable stealth output for the default test account. Tests mutate
/// individual fields to exercise the rejection paths.
fn stealth_output(value: u64, seed: u8) -> StealthOutputModel {
    StealthOutputModel {
        owner_account: Test::test_account_address(),
        resource_address: STEALTH_TARI_RESOURCE_ADDRESS,
        commitment: PedersenCommitmentBytes::from_array([seed; PedersenCommitmentBytes::length()]),
        value,
        sender_public_nonce: RistrettoPublicKeyBytes::default(),
        view_only_key_id: KeyId::derived(KeyBranch::ViewOnlyKey, 0),
        owner_key_id: Some(KeyId::derived(KeyBranch::Account, 0)),
        encrypted_data: EncryptedData::try_from(vec![0u8; EncryptedData::min_size()]).unwrap(),
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

/// A second wallet account, used to prove that a caller cannot select or lock another account's outputs.
fn other_account_address() -> ComponentAddress {
    "component_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a07fffffffe"
        .parse()
        .unwrap()
}

fn add_other_account(test: &Test) -> ComponentAddress {
    let address = other_account_address();
    test.sdk()
        .accounts_api()
        .add_account(
            Some("other"),
            &address,
            KeyId::derived(KeyBranch::ViewOnlyKey, 1),
            KeyId::derived(KeyBranch::Account, 1),
            Epoch::zero(),
            true,
            false,
        )
        .unwrap();
    address
}

/// Reads a stored output back for assertions about its post-call status and lock.
fn stored_output(test: &Test, owner: &ComponentAddress, commitment: &PedersenCommitmentBytes) -> StealthOutputModel {
    test.sdk()
        .stealth_outputs_api()
        .utxos_get_many(&STEALTH_TARI_RESOURCE_ADDRESS, Some(owner), None)
        .unwrap()
        .into_iter()
        .find(|o| &o.commitment == commitment)
        .expect("output present in store")
}

fn expect_rejected(
    test: &Test,
    account: &ComponentAddress,
    lock_id: WalletLockId,
    addresses: &[UtxoAddress],
) -> StealthOutputsApiError {
    let err = test
        .sdk()
        .stealth_outputs_api()
        .lock_specific_outputs(account, &STEALTH_TARI_RESOURCE_ADDRESS, lock_id, addresses)
        .expect_err("request should be rejected");
    assert!(
        matches!(err, StealthOutputsApiError::InvalidParameter { .. }),
        "expected InvalidParameter, got: {err}"
    );
    err
}

#[test]
fn burnt_outputs_are_excluded_from_the_unspent_balance() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();

    let kept = stealth_output(10, 1);
    let to_burn = stealth_output(100, 2);
    outputs.add_output(&kept).unwrap();
    outputs.add_output(&to_burn).unwrap();

    // Both outputs count while unspent and not burnt.
    assert_eq!(
        outputs
            .get_unspent_balance(&STEALTH_TARI_RESOURCE_ADDRESS)
            .unwrap()
            .balance,
        Amount::from(110u64)
    );
    let account_total: Amount = outputs
        .get_unspent_outputs_by_account(&Test::test_account_address(), false)
        .unwrap()
        .into_iter()
        .map(|o| Amount::from(o.value))
        .sum();
    assert_eq!(account_total, Amount::from(110u64));

    // A burnt output keeps status = Unspent, so it must be excluded by the burnt filter, not the status filter.
    outputs
        .update_utxo_status(&to_burn.to_utxo_address(), Some(true), None, None)
        .unwrap();

    let balance = outputs.get_unspent_balance(&STEALTH_TARI_RESOURCE_ADDRESS).unwrap();
    assert_eq!(balance.balance, Amount::from(10u64));
    assert_eq!(balance.utxo_count, 1);

    let account_outputs = outputs
        .get_unspent_outputs_by_account(&Test::test_account_address(), false)
        .unwrap();
    assert_eq!(account_outputs.len(), 1);
    assert_eq!(
        account_outputs
            .into_iter()
            .map(|o| Amount::from(o.value))
            .sum::<Amount>(),
        Amount::from(10u64)
    );
}

#[test]
fn locks_exactly_the_requested_outputs_and_returns_values() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let a = stealth_output(10, 1);
    let b = stealth_output(20, 2);
    let c = stealth_output(30, 3);
    // A fourth output that is not requested and must remain untouched.
    let untouched = stealth_output(40, 4);
    for output in [&a, &b, &c, &untouched] {
        outputs.add_output(output).unwrap();
    }

    let lock = test.new_lock();
    let selected = outputs
        .lock_specific_outputs(&account, &STEALTH_TARI_RESOURCE_ADDRESS, lock.id(), &[
            a.to_utxo_address(),
            b.to_utxo_address(),
            c.to_utxo_address(),
        ])
        .unwrap();

    // Exactly the requested inputs are returned, carrying their decrypted values.
    assert_eq!(selected.len(), 3);
    assert_eq!(selected.iter().map(|i| i.value).collect::<Vec<_>>(), vec![10, 20, 30]);
    assert_eq!(selected.iter().map(|i| i.commitment).collect::<Vec<_>>(), vec![
        a.commitment,
        b.commitment,
        c.commitment
    ]);

    // The three requested outputs are now locked under this lock.
    for output in [&a, &b, &c] {
        let stored = stored_output(&test, &account, &output.commitment);
        assert!(
            matches!(stored.status, OutputStatus::LockedForSpend),
            "expected LockedForSpend, got {:?}",
            stored.status
        );
        assert_eq!(stored.lock_id, Some(lock.id()));
    }
    // The unrequested output is untouched.
    let stored = stored_output(&test, &account, &untouched.commitment);
    assert!(
        matches!(stored.status, OutputStatus::Unspent),
        "expected Unspent, got {:?}",
        stored.status
    );
    assert_eq!(stored.lock_id, None);
}

#[test]
fn preserves_requested_order() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let a = stealth_output(10, 1);
    let b = stealth_output(20, 2);
    let c = stealth_output(30, 3);
    for output in [&a, &b, &c] {
        outputs.add_output(output).unwrap();
    }

    let lock = test.new_lock();
    // Request order c, a, b — deliberately different from insertion and value order.
    let selected = outputs
        .lock_specific_outputs(&account, &STEALTH_TARI_RESOURCE_ADDRESS, lock.id(), &[
            c.to_utxo_address(),
            a.to_utxo_address(),
            b.to_utxo_address(),
        ])
        .unwrap();

    assert_eq!(selected.iter().map(|i| i.value).collect::<Vec<_>>(), vec![30, 10, 20]);
    assert_eq!(selected.iter().map(|i| i.commitment).collect::<Vec<_>>(), vec![
        c.commitment,
        a.commitment,
        b.commitment
    ]);
}

#[test]
fn rejects_empty_input_list() {
    let test = Test::new();
    let lock = test.new_lock();
    expect_rejected(&test, &Test::test_account_address(), lock.id(), &[]);
}

#[test]
fn rejects_input_count_above_stealth_limit() {
    let test = Test::new();
    let lock = test.new_lock();

    // One more than the maximum. No outputs need to exist: the count is checked before any lookup.
    let too_many = limits::STEALTH_LIMITS.max_inputs + 1;
    let addresses = (0..too_many)
        .map(|i| {
            let mut bytes = [0u8; PedersenCommitmentBytes::length()];
            bytes[0] = (i & 0xff) as u8;
            bytes[1] = ((i >> 8) & 0xff) as u8;
            UtxoAddress::new(
                STEALTH_TARI_RESOURCE_ADDRESS,
                PedersenCommitmentBytes::from_array(bytes).into(),
            )
        })
        .collect::<Vec<_>>();

    expect_rejected(&test, &Test::test_account_address(), lock.id(), &addresses);
}

#[test]
fn rejects_duplicate_input_addresses() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let a = stealth_output(10, 1);
    outputs.add_output(&a).unwrap();

    let lock = test.new_lock();
    expect_rejected(&test, &account, lock.id(), &[a.to_utxo_address(), a.to_utxo_address()]);

    // The output was never locked.
    assert!(matches!(
        stored_output(&test, &account, &a.commitment).status,
        OutputStatus::Unspent
    ));
}

#[test]
fn rejects_wrong_resource_address() {
    let test = Test::new();
    let account = Test::test_account_address();
    let lock = test.new_lock();

    // A UtxoAddress whose resource is not the resource we are selecting for.
    let wrong_resource = resource_address_from_seed(9);
    let address = UtxoAddress::new(
        wrong_resource,
        PedersenCommitmentBytes::from_array([1u8; PedersenCommitmentBytes::length()]).into(),
    );

    expect_rejected(&test, &account, lock.id(), &[address]);
}

#[test]
fn rejects_unknown_output() {
    let test = Test::new();
    let account = Test::test_account_address();
    let lock = test.new_lock();

    // A well-formed address for the correct resource that was never added to the wallet.
    let address = UtxoAddress::new(
        STEALTH_TARI_RESOURCE_ADDRESS,
        PedersenCommitmentBytes::from_array([7u8; PedersenCommitmentBytes::length()]).into(),
    );

    expect_rejected(&test, &account, lock.id(), &[address]);
}

#[test]
fn rejects_output_belonging_to_another_account() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();
    let other = add_other_account(&test);

    let mut foreign = stealth_output(10, 1);
    foreign.owner_account = other;
    outputs.add_output(&foreign).unwrap();

    let lock = test.new_lock();
    // The default account attempts to lock the other account's output.
    expect_rejected(&test, &account, lock.id(), &[foreign.to_utxo_address()]);

    // Account scoping guarantee: the other account's output was not touched.
    let stored = stored_output(&test, &other, &foreign.commitment);
    assert!(
        matches!(stored.status, OutputStatus::Unspent),
        "expected Unspent, got {:?}",
        stored.status
    );
    assert_eq!(stored.lock_id, None);
}

#[test]
fn rejects_view_only_output() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let mut view_only = stealth_output(10, 1);
    view_only.owner_key_id = None;
    outputs.add_output(&view_only).unwrap();

    let lock = test.new_lock();
    expect_rejected(&test, &account, lock.id(), &[view_only.to_utxo_address()]);

    assert!(matches!(
        stored_output(&test, &account, &view_only.commitment).status,
        OutputStatus::Unspent
    ));
}

#[test]
fn rejects_burnt_output() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let mut burnt = stealth_output(10, 1);
    burnt.is_burnt = true;
    outputs.add_output(&burnt).unwrap();

    let lock = test.new_lock();
    expect_rejected(&test, &account, lock.id(), &[burnt.to_utxo_address()]);

    assert!(matches!(
        stored_output(&test, &account, &burnt.commitment).status,
        OutputStatus::Unspent
    ));
}

#[test]
fn rejects_frozen_output() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let mut frozen = stealth_output(10, 1);
    frozen.is_frozen = true;
    outputs.add_output(&frozen).unwrap();

    let lock = test.new_lock();
    expect_rejected(&test, &account, lock.id(), &[frozen.to_utxo_address()]);

    assert!(matches!(
        stored_output(&test, &account, &frozen.commitment).status,
        OutputStatus::Unspent
    ));
}

#[test]
fn rejects_off_chain_output() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    // A corrupt/impossible state: Unspent but flagged off-chain. An Unspent output must be confirmed on-chain, so this
    // is rejected (fail-closed on inconsistent state).
    let mut off_chain = stealth_output(10, 1);
    off_chain.is_on_chain = false;
    outputs.add_output(&off_chain).unwrap();

    let lock = test.new_lock();
    expect_rejected(&test, &account, lock.id(), &[off_chain.to_utxo_address()]);

    assert!(matches!(
        stored_output(&test, &account, &off_chain.commitment).status,
        OutputStatus::Unspent
    ));
}

#[test]
fn rejects_condition_tree_output() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    // A script-path (condition-tree) output: no owned spend key, so not key-path spendable.
    let mut condition_tree = stealth_output(10, 1);
    condition_tree.auth = SpendAuthorization::Script(Hash32::from_array([1u8; Hash32::LENGTH]));
    condition_tree.is_condition_spendable = false;
    outputs.add_output(&condition_tree).unwrap();

    let lock = test.new_lock();
    expect_rejected(&test, &account, lock.id(), &[condition_tree.to_utxo_address()]);

    assert!(matches!(
        stored_output(&test, &account, &condition_tree.commitment).status,
        OutputStatus::Unspent
    ));
}

#[test]
fn rejects_output_already_locked_under_another_lock() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let a = stealth_output(10, 1);
    outputs.add_output(&a).unwrap();

    let lock_a = test.new_lock();
    outputs
        .lock_specific_outputs(&account, &STEALTH_TARI_RESOURCE_ADDRESS, lock_a.id(), &[
            a.to_utxo_address()
        ])
        .unwrap();

    // A different lock cannot claim the already-locked output.
    let lock_b = test.new_lock();
    expect_rejected(&test, &account, lock_b.id(), &[a.to_utxo_address()]);

    // It remains locked under the first lock.
    let stored = stored_output(&test, &account, &a.commitment);
    assert!(
        matches!(stored.status, OutputStatus::LockedForSpend),
        "expected LockedForSpend, got {:?}",
        stored.status
    );
    assert_eq!(stored.lock_id, Some(lock_a.id()));
}

#[test]
fn all_or_nothing_when_one_input_is_invalid() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let valid = stealth_output(10, 1);
    let mut frozen = stealth_output(20, 2);
    frozen.is_frozen = true;
    outputs.add_output(&valid).unwrap();
    outputs.add_output(&frozen).unwrap();

    let lock = test.new_lock();
    // The valid input is listed first, then the invalid one: the whole request must fail with nothing locked.
    expect_rejected(&test, &account, lock.id(), &[
        valid.to_utxo_address(),
        frozen.to_utxo_address(),
    ]);

    assert!(matches!(
        stored_output(&test, &account, &valid.commitment).status,
        OutputStatus::Unspent
    ));
    assert_eq!(stored_output(&test, &account, &valid.commitment).lock_id, None);
    assert!(matches!(
        stored_output(&test, &account, &frozen.commitment).status,
        OutputStatus::Unspent
    ));
}

#[test]
fn sequential_overlapping_requests_do_not_double_lock_the_same_output() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let a = stealth_output(10, 1);
    let b = stealth_output(20, 2);
    outputs.add_output(&a).unwrap();
    outputs.add_output(&b).unwrap();

    // First request locks `a`.
    let lock_a = test.new_lock();
    outputs
        .lock_specific_outputs(&account, &STEALTH_TARI_RESOURCE_ADDRESS, lock_a.id(), &[
            a.to_utxo_address()
        ])
        .unwrap();

    // A second, sequential request overlapping on `a` (also wanting `b`) must fail without locking anything new.
    // (This is not a multithreaded concurrency test; SQLite serialises writers, so sequential exclusivity is the
    // guarantee exercised here.)
    let lock_b = test.new_lock();
    expect_rejected(&test, &account, lock_b.id(), &[
        a.to_utxo_address(),
        b.to_utxo_address(),
    ]);

    let stored_a = stored_output(&test, &account, &a.commitment);
    assert!(
        matches!(stored_a.status, OutputStatus::LockedForSpend),
        "expected LockedForSpend, got {:?}",
        stored_a.status
    );
    assert_eq!(stored_a.lock_id, Some(lock_a.id()));

    let stored_b = stored_output(&test, &account, &b.commitment);
    assert!(
        matches!(stored_b.status, OutputStatus::Unspent),
        "expected Unspent, got {:?}",
        stored_b.status
    );
    assert_eq!(stored_b.lock_id, None);
}

/// Builds a `LockedUnconfirmed` output bound to `lock_id`, matching storage semantics (off-chain by construction).
fn locked_unconfirmed_output(value: u64, seed: u8, lock_id: WalletLockId) -> StealthOutputModel {
    let mut output = stealth_output(value, seed);
    output.status = OutputStatus::LockedUnconfirmed;
    output.lock_id = Some(lock_id);
    output.is_on_chain = false;
    output
}

#[test]
fn accepts_locked_unconfirmed_under_same_lock() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    let lock = test.new_lock();
    // An output created earlier in this lock's transaction chain: LockedUnconfirmed, off-chain, bound to this lock.
    let earlier = locked_unconfirmed_output(10, 1, lock.id());
    outputs.add_output(&earlier).unwrap();

    // Re-locking it under the same lock must succeed: the same-lock LockedUnconfirmed allowance is reachable here.
    let selected = outputs
        .lock_specific_outputs(&account, &STEALTH_TARI_RESOURCE_ADDRESS, lock.id(), &[
            earlier.to_utxo_address()
        ])
        .unwrap();

    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].value, 10);
    assert_eq!(selected[0].commitment, earlier.commitment);

    // The output is now LockedForSpend under the same lock (re-locked).
    let stored = stored_output(&test, &account, &earlier.commitment);
    assert!(
        matches!(stored.status, OutputStatus::LockedForSpend),
        "expected LockedForSpend, got {:?}",
        stored.status
    );
    assert_eq!(stored.lock_id, Some(lock.id()));
}

#[test]
fn rejects_locked_unconfirmed_under_a_different_lock() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();

    // An output locked unconfirmed under lock_a.
    let lock_a = test.new_lock();
    let earlier = locked_unconfirmed_output(10, 1, lock_a.id());
    outputs.add_output(&earlier).unwrap();

    // A different lock cannot claim the LockedUnconfirmed output.
    let lock_b = test.new_lock();
    expect_rejected(&test, &account, lock_b.id(), &[earlier.to_utxo_address()]);

    // The output is untouched: still LockedUnconfirmed under lock_a.
    let stored = stored_output(&test, &account, &earlier.commitment);
    assert!(
        matches!(stored.status, OutputStatus::LockedUnconfirmed),
        "expected LockedUnconfirmed, got {:?}",
        stored.status
    );
    assert_eq!(stored.lock_id, Some(lock_a.id()));
}

/// The UTXO scanner streams spends only while this count is non-zero, so an output locked by a pending
/// transaction must count: its spend still has to reach the wallet.
#[test]
fn locked_outputs_count_as_unspent_or_locked() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let account = Test::test_account_address();
    let count = || {
        outputs
            .count_unspent_or_locked_outputs_for_account(&account, &STEALTH_TARI_RESOURCE_ADDRESS)
            .unwrap()
    };

    let output = stealth_output(10, 1);
    outputs.add_output(&output).unwrap();
    let mut spent = stealth_output(20, 2);
    spent.status = OutputStatus::Spent;
    outputs.add_output(&spent).unwrap();
    assert_eq!(count(), 1);

    let lock = test.new_lock();
    outputs
        .lock_specific_outputs(&account, &STEALTH_TARI_RESOURCE_ADDRESS, lock.id(), &[
            output.to_utxo_address()
        ])
        .unwrap();
    assert_eq!(count(), 1);
}

/// UTXO recovery reports an output only when recording it changes the wallet's view of it.
#[test]
fn upserting_an_output_reports_whether_it_changed() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let output = stealth_output(10, 1);

    assert!(outputs.upsert_utxo(&output).unwrap(), "a new output is inserted");
    assert!(
        !outputs.upsert_utxo(&output).unwrap(),
        "the same output again changes nothing"
    );

    let mut frozen = output.clone();
    frozen.is_frozen = true;
    assert!(outputs.upsert_utxo(&frozen).unwrap(), "freezing the output changes it");
    assert!(!outputs.upsert_utxo(&frozen).unwrap());
}

#[test]
fn setting_the_frozen_flag_reports_whether_it_changed() {
    let test = Test::new();
    let outputs = test.sdk().stealth_outputs_api();
    let output = stealth_output(10, 1);
    let address = output.to_utxo_address();

    assert_eq!(
        outputs.set_utxo_frozen(&address, true).unwrap(),
        None,
        "no output is held"
    );

    outputs.add_output(&output).unwrap();
    assert_eq!(outputs.set_utxo_frozen(&address, false).unwrap(), Some(false));
    assert_eq!(outputs.set_utxo_frozen(&address, true).unwrap(), Some(true));
    assert!(stored_output(&test, &Test::test_account_address(), &output.commitment).is_frozen);
    assert_eq!(outputs.set_utxo_frozen(&address, true).unwrap(), Some(false));
}
