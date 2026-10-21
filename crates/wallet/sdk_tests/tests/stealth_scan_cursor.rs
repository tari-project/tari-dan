//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A verifying wallet stores each shard's stealth UTXO scan cursor no further than the state version a verified
//! end-of-epoch checkpoint commits for the shard, and what an indexer reports past it as a claim. A scan resumes from
//! the claim, leaving the verified cursor where the claim began, until a verified checkpoint that ended after the claim
//! began expires it. The next scan then asks again for every update the network committed after the verified cursor,
//! whatever an indexer claimed before.

mod support;

use std::{collections::HashMap, time::Duration};

use ootle_byte_type::ToByteType;
use tari_ootle_common_types::{Epoch, StateVersion, SubstateVersion, shard::Shard};
use tari_ootle_light_client::test_fixtures::{CheckpointSpec, ValidatorSetFixture, checkpoint_entry, high};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    models::{
        ClaimedStateVersion,
        KeyBranch,
        KeyId,
        OutputStatus,
        StartOfShard,
        StealthOutputModel,
        UtxoSpent,
        UtxoUnspent,
        UtxoUpdatePayload,
        WalletEvent,
        WalletUtxoUpdate,
    },
    storage::{ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_sdk_services::{
    notify::Notify,
    utxo_scanner::{StealthUtxoScannerWorker, UtxoScannerHandle},
};
use tari_template_lib::types::{
    EncryptedData,
    UtxoId,
    constants::STEALTH_TARI_RESOURCE_ADDRESS,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
    stealth::SpendAuthorization,
};

use crate::support::{ProvingNetwork, TestWithNetwork, random_keypair};

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);

/// The shard the wallet's UTXO lives in, in the walked shard group.
const SHARD: Shard = Shard::from_u32(1);
/// The state version the latest verified checkpoint commits for [`SHARD`].
const CHECKPOINT_VERSION: u64 = 10;
/// The epoch the latest verified checkpoint ends.
const CHECKPOINT_EPOCH: Epoch = Epoch(2);

/// A verifying wallet with the test account, anchored on the validator set of epoch 2, and an indexer whose latest
/// checkpoints end epoch 2 with [`SHARD`] at [`CHECKPOINT_VERSION`].
async fn anchored_wallet() -> (ProvingNetwork, TestWithNetwork<ProvingNetwork>) {
    let sets = (1..=3).map(ValidatorSetFixture::new).collect::<Vec<_>>();
    let network = ProvingNetwork::new(&sets, 0);
    let test = TestWithNetwork::with_network_trusting(network.clone(), IndexerTrust::Verify);
    test.sdk().validator_set_api().sync().await.unwrap();

    network.replace_checkpoint(
        1,
        checkpoint_entry(CheckpointSpec {
            shard_state_versions: vec![(SHARD, CHECKPOINT_VERSION)],
            ..CheckpointSpec::handover(&sets[1], &sets[2])
        }),
    );
    network.set_latest(1);
    network.serve_latest_checkpoint(checkpoint_entry(CheckpointSpec {
        shard_group: high(),
        signers: &sets[1].high,
        next_epoch_committee: Some(sets[2].committee(high())),
        shard_state_versions: vec![(Shard::from_u32(3), 7)],
        ..CheckpointSpec::handover(&sets[1], &sets[2])
    }));
    (network, test)
}

/// A UTXO that the test account's view key recognises as its own.
fn own_utxo(test: &TestWithNetwork<ProvingNetwork>) -> UtxoUnspent {
    let sdk = test.sdk();
    let view_key = sdk
        .key_manager_api()
        .get_key(KeyId::derived(KeyBranch::ViewOnlyKey, 0))
        .unwrap();
    let (_, public_nonce) = random_keypair();
    let tag: UtxoTag = sdk.stealth_crypto_api().derive_stealth_output_tag(
        sdk.network(),
        &view_key.secret,
        &public_nonce,
        &STEALTH_TARI_RESOURCE_ADDRESS,
    );
    UtxoUnspent {
        tag,
        public_nonce: public_nonce.to_byte_type(),
    }
}

fn start_of_shard(max_state_version: u64, has_more: bool) -> StartOfShard {
    StartOfShard {
        shard: SHARD,
        max_state_version: StateVersion::from(max_state_version),
        has_more,
    }
}

fn cursor_of(cursors: &[(Shard, StateVersion)]) -> StateVersion {
    cursors
        .iter()
        .find(|(shard, _)| *shard == SHARD)
        .map(|(_, version)| *version)
        .expect("the request asks for the shard")
}

/// Answers like an indexer holding `utxos` in [`SHARD`], each at its state version: a request gets every UTXO past its
/// cursor, and nothing at all when there is none.
fn serve_utxos(network: &ProvingNetwork, utxos: Vec<(u64, UtxoUnspent)>) {
    network.answer_utxo_streams(move |cursors| {
        let cursor = cursor_of(cursors);
        let past_cursor = utxos
            .iter()
            .filter(|(version, _)| StateVersion::from(*version) > cursor)
            .collect::<Vec<_>>();
        let Some(max_version) = past_cursor.iter().map(|(version, _)| *version).max() else {
            return vec![];
        };
        let mut payloads = past_cursor
            .into_iter()
            .map(|(_, utxo)| UtxoUpdatePayload {
                sos: None,
                update: Some(WalletUtxoUpdate::Unspent(utxo.clone())),
                eos: None,
            })
            .collect::<Vec<_>>();
        payloads[0].sos = Some(start_of_shard(max_version, false));
        payloads
    });
}

fn scanner(test: &TestWithNetwork<ProvingNetwork>) -> UtxoScannerHandle {
    scanner_notifying(test, Notify::new(100))
}

fn scanner_notifying(test: &TestWithNetwork<ProvingNetwork>, notify: Notify<WalletEvent>) -> UtxoScannerHandle {
    let (_join, handle) = StealthUtxoScannerWorker::new(test.sdk().clone(), notify).spawn();
    handle
}

/// An unspent stealth output of the test account.
fn held_output(seed: u8) -> StealthOutputModel {
    StealthOutputModel {
        owner_account: TestWithNetwork::<ProvingNetwork>::test_account_address(),
        resource_address: STEALTH_TARI_RESOURCE_ADDRESS,
        commitment: PedersenCommitmentBytes::from_array([seed; PedersenCommitmentBytes::length()]),
        value: 10,
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

async fn scan(scanner: &UtxoScannerHandle) {
    tokio::time::timeout(
        SCAN_TIMEOUT,
        scanner.scan(
            TestWithNetwork::<ProvingNetwork>::test_account_address(),
            STEALTH_TARI_RESOURCE_ADDRESS,
        ),
    )
    .await
    .expect("the scan did not finish")
    .expect("the scan failed");
}

fn stored_cursors(test: &TestWithNetwork<ProvingNetwork>) -> HashMap<Shard, StateVersion> {
    test.store()
        .with_read_tx(|tx| {
            tx.shard_state_version_get(
                &TestWithNetwork::<ProvingNetwork>::test_account_address(),
                &STEALTH_TARI_RESOURCE_ADDRESS,
            )
        })
        .unwrap()
}

fn stored_claims(test: &TestWithNetwork<ProvingNetwork>) -> HashMap<Shard, ClaimedStateVersion> {
    test.store()
        .with_read_tx(|tx| {
            tx.shard_state_version_claims_get(
                &TestWithNetwork::<ProvingNetwork>::test_account_address(),
                &STEALTH_TARI_RESOURCE_ADDRESS,
            )
        })
        .unwrap()
}

fn store_cursor(test: &TestWithNetwork<ProvingNetwork>, state_version: u64) {
    test.store()
        .with_write_tx(|tx| {
            tx.shard_state_version_set_many(
                &TestWithNetwork::<ProvingNetwork>::test_account_address(),
                &STEALTH_TARI_RESOURCE_ADDRESS,
                [(SHARD, StateVersion::from(state_version))],
            )
        })
        .unwrap();
}

fn store_claim(test: &TestWithNetwork<ProvingNetwork>, state_version: u64, verified_epoch: Option<Epoch>) {
    test.store()
        .with_write_tx(|tx| {
            tx.shard_state_version_claims_set_many(
                &TestWithNetwork::<ProvingNetwork>::test_account_address(),
                &STEALTH_TARI_RESOURCE_ADDRESS,
                [(SHARD, claim(state_version, verified_epoch))],
            )
        })
        .unwrap();
}

/// A claim that began at `state_version` and has not advanced.
fn claim(state_version: u64, verified_epoch: Option<Epoch>) -> ClaimedStateVersion {
    advanced_claim(state_version, state_version, verified_epoch)
}

fn advanced_claim(first_state_version: u64, state_version: u64, verified_epoch: Option<Epoch>) -> ClaimedStateVersion {
    ClaimedStateVersion {
        state_version: StateVersion::from(state_version),
        first_state_version: StateVersion::from(first_state_version),
        verified_epoch,
    }
}

fn queued_utxos(test: &TestWithNetwork<ProvingNetwork>) -> usize {
    test.store()
        .with_read_tx(|tx| tx.utxo_process_queue_fetch_batch(None, 100))
        .unwrap()
        .entries
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .map(HashMap::len)
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cursor_reported_past_the_verified_state_version_is_stored_at_it_and_claimed() {
    let (network, test) = anchored_wallet().await;
    let scanner = scanner(&test);
    serve_utxos(&network, vec![(CHECKPOINT_VERSION + 5, own_utxo(&test))]);

    scan(&scanner).await;
    assert_eq!(
        stored_cursors(&test).get(&SHARD),
        Some(&StateVersion::from(CHECKPOINT_VERSION))
    );
    assert_eq!(
        stored_claims(&test).get(&SHARD),
        Some(&claim(CHECKPOINT_VERSION + 5, Some(CHECKPOINT_EPOCH)))
    );

    scan(&scanner).await;

    let requests = network.utxo_stream_requests();
    assert_eq!(
        cursor_of(&requests[1]),
        StateVersion::from(CHECKPOINT_VERSION + 5),
        "the next scan resumes from the claim"
    );
    assert_eq!(queued_utxos(&test), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scan_resumed_from_a_claim_advances_the_claim_but_not_the_verified_cursor() {
    let (network, test) = anchored_wallet().await;
    let began_in = Epoch(CHECKPOINT_EPOCH.as_u64() - 1);
    store_cursor(&test, CHECKPOINT_VERSION - 5);
    store_claim(&test, CHECKPOINT_VERSION + 5, Some(began_in));
    serve_utxos(&network, vec![(CHECKPOINT_VERSION + 10, own_utxo(&test))]);

    scan(&scanner(&test)).await;

    assert_eq!(
        cursor_of(&network.utxo_stream_requests()[0]),
        StateVersion::from(CHECKPOINT_VERSION + 5)
    );
    assert_eq!(queued_utxos(&test), 1);
    assert_eq!(
        stored_claims(&test).get(&SHARD),
        Some(&advanced_claim(
            CHECKPOINT_VERSION + 5,
            CHECKPOINT_VERSION + 10,
            Some(began_in)
        )),
        "the claim advances and keeps where it began"
    );
    assert_eq!(
        stored_cursors(&test).get(&SHARD),
        Some(&StateVersion::from(CHECKPOINT_VERSION - 5)),
        "the verified cursor stays where the claim began"
    );
}

/// A lying indexer claims a cursor just ahead of the network, which the network then overtakes. An update committed
/// below the claim is never delivered while scans resume from it, so once the claim expires the shard is scanned again
/// from the verified cursor, where the claim began.
#[tokio::test(flavor = "multi_thread")]
async fn an_expired_claim_that_the_network_overtook_is_rescanned_from_the_verified_cursor() {
    let (network, test) = anchored_wallet().await;
    store_cursor(&test, CHECKPOINT_VERSION - 5);
    store_claim(
        &test,
        CHECKPOINT_VERSION - 2,
        Some(Epoch(CHECKPOINT_EPOCH.as_u64() - 2)),
    );
    serve_utxos(&network, vec![(CHECKPOINT_VERSION - 3, own_utxo(&test))]);

    scan(&scanner(&test)).await;

    assert_eq!(
        cursor_of(&network.utxo_stream_requests()[0]),
        StateVersion::from(CHECKPOINT_VERSION - 5)
    );
    assert_eq!(queued_utxos(&test), 1, "the UTXO the claim skipped is found");
    assert_eq!(stored_claims(&test).get(&SHARD), None);
    assert_eq!(
        stored_cursors(&test).get(&SHARD),
        Some(&StateVersion::from(CHECKPOINT_VERSION - 3))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_claim_past_the_verified_checkpoint_is_rescanned_from_the_verified_cursor() {
    let (network, test) = anchored_wallet().await;
    store_cursor(&test, CHECKPOINT_VERSION);
    store_claim(&test, u64::MAX, Some(Epoch(CHECKPOINT_EPOCH.as_u64() - 2)));
    serve_utxos(&network, vec![(CHECKPOINT_VERSION + 1, own_utxo(&test))]);

    scan(&scanner(&test)).await;

    assert_eq!(
        cursor_of(&network.utxo_stream_requests()[0]),
        StateVersion::from(CHECKPOINT_VERSION),
        "the scan resumes from the verified cursor"
    );
    assert_eq!(
        queued_utxos(&test),
        1,
        "the UTXO committed after the checkpoint is found"
    );
    assert_eq!(
        stored_claims(&test).get(&SHARD),
        Some(&claim(CHECKPOINT_VERSION + 1, Some(CHECKPOINT_EPOCH))),
        "the honest indexer's claim replaces the expired one"
    );
}

/// A checkpoint only one epoch after the one the wallet verified when the claim began may have ended before the claim
/// began, so it does not expire it.
#[tokio::test(flavor = "multi_thread")]
async fn a_claim_too_recent_for_the_verified_checkpoint_to_expire_is_resumed_from() {
    let (network, test) = anchored_wallet().await;
    let claimed = CHECKPOINT_VERSION + 100;
    store_claim(&test, claimed, Some(Epoch(CHECKPOINT_EPOCH.as_u64() - 1)));
    serve_utxos(&network, vec![]);

    scan(&scanner(&test)).await;

    assert_eq!(
        cursor_of(&network.utxo_stream_requests()[0]),
        StateVersion::from(claimed)
    );
    assert_eq!(
        stored_claims(&test).get(&SHARD),
        Some(&claim(claimed, Some(Epoch(CHECKPOINT_EPOCH.as_u64() - 1))))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claim_begun_without_a_verified_checkpoint_expires_at_the_first() {
    let (network, test) = anchored_wallet().await;
    store_cursor(&test, CHECKPOINT_VERSION);
    store_claim(&test, u64::MAX, None);
    serve_utxos(&network, vec![(CHECKPOINT_VERSION + 1, own_utxo(&test))]);

    scan(&scanner(&test)).await;

    assert_eq!(
        cursor_of(&network.utxo_stream_requests()[0]),
        StateVersion::from(CHECKPOINT_VERSION)
    );
    assert_eq!(queued_utxos(&test), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_cursor_past_the_verified_state_version_is_lowered_to_it() {
    let (network, test) = anchored_wallet().await;
    store_cursor(&test, u64::MAX);
    serve_utxos(&network, vec![(CHECKPOINT_VERSION + 1, own_utxo(&test))]);

    scan(&scanner(&test)).await;

    assert_eq!(
        cursor_of(&network.utxo_stream_requests()[0]),
        StateVersion::from(CHECKPOINT_VERSION)
    );
    assert_eq!(queued_utxos(&test), 1);
    assert_eq!(
        stored_cursors(&test).get(&SHARD),
        Some(&StateVersion::from(CHECKPOINT_VERSION))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cursor_at_or_below_the_verified_state_version_is_stored_as_claimed() {
    let (network, test) = anchored_wallet().await;
    serve_utxos(&network, vec![(CHECKPOINT_VERSION - 4, own_utxo(&test))]);

    scan(&scanner(&test)).await;

    assert_eq!(queued_utxos(&test), 1);
    assert_eq!(
        stored_cursors(&test).get(&SHARD),
        Some(&StateVersion::from(CHECKPOINT_VERSION - 4))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scan_pages_past_the_verified_state_version_without_storing_past_it() {
    let (network, test) = anchored_wallet().await;
    let first = own_utxo(&test);
    let second = own_utxo(&test);
    network.answer_utxo_streams(move |cursors| {
        let cursor = cursor_of(cursors).as_u64();
        let (max_version, has_more, utxo) = if cursor < CHECKPOINT_VERSION + 1 {
            (CHECKPOINT_VERSION + 1, true, &first)
        } else if cursor < CHECKPOINT_VERSION + 2 {
            (CHECKPOINT_VERSION + 2, false, &second)
        } else {
            return vec![];
        };
        vec![UtxoUpdatePayload {
            sos: Some(start_of_shard(max_version, has_more)),
            update: Some(WalletUtxoUpdate::Unspent(utxo.clone())),
            eos: None,
        }]
    });

    scan(&scanner(&test)).await;

    let requested = network
        .utxo_stream_requests()
        .iter()
        .map(|cursors| cursor_of(cursors).as_u64())
        .collect::<Vec<_>>();
    assert_eq!(requested, [0, CHECKPOINT_VERSION + 1]);
    assert_eq!(queued_utxos(&test), 2);
    assert_eq!(
        stored_cursors(&test).get(&SHARD),
        Some(&StateVersion::from(CHECKPOINT_VERSION))
    );
    assert_eq!(
        stored_claims(&test).get(&SHARD),
        Some(&advanced_claim(
            CHECKPOINT_VERSION + 1,
            CHECKPOINT_VERSION + 2,
            Some(CHECKPOINT_EPOCH)
        )),
        "the claim begins on the first page past the verified state version"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_verified_checkpoint_a_scan_stores_only_a_claim() {
    let (network, test) = anchored_wallet().await;
    network.withhold_checkpoints();
    serve_utxos(&network, vec![(CHECKPOINT_VERSION + 1, own_utxo(&test))]);

    scan(&scanner(&test)).await;

    assert_eq!(queued_utxos(&test), 1, "the scan still finds the UTXO");
    assert_eq!(stored_cursors(&test).get(&SHARD), None);
    assert_eq!(
        stored_claims(&test).get(&SHARD),
        Some(&claim(CHECKPOINT_VERSION + 1, None))
    );
}

/// An indexer may deliver a spend again, as when a scan resumes from the verified cursor below it, so the wallet must
/// report each spend once however often it arrives.
#[tokio::test(flavor = "multi_thread")]
async fn a_spend_delivered_again_is_reported_once() {
    let (network, test) = anchored_wallet().await;
    let output = held_output(7);
    test.sdk().stealth_outputs_api().add_output(&output).unwrap();
    let spent = UtxoSpent {
        id: UtxoId::from(output.commitment),
        version: SubstateVersion::new(1),
    };
    network.answer_utxo_streams(move |_| {
        vec![UtxoUpdatePayload {
            sos: Some(start_of_shard(CHECKPOINT_VERSION + 1, false)),
            update: Some(WalletUtxoUpdate::Spent(spent.clone())),
            eos: None,
        }]
    });
    let notify = Notify::new(100);
    let mut events = notify.subscribe();
    let scanner = scanner_notifying(&test, notify);

    scan(&scanner).await;
    scan(&scanner).await;

    assert_eq!(network.utxo_stream_requests().len(), 2);
    let mut spent_events = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, WalletEvent::UtxoSpent(_)) {
            spent_events += 1;
        }
    }
    assert_eq!(spent_events, 1);
}
