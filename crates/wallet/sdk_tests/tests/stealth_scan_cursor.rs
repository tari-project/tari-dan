//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A verifying wallet stores each shard's stealth UTXO scan cursor no further than the state version a verified
//! end-of-epoch checkpoint commits for the shard, so that a later scan asks for every update the network committed
//! after that checkpoint, whatever an indexer claimed before.

mod support;

use std::{collections::HashMap, time::Duration};

use ootle_byte_type::ToByteType;
use tari_ootle_common_types::{StateVersion, shard::Shard};
use tari_ootle_light_client::test_fixtures::{CheckpointSpec, ValidatorSetFixture, checkpoint_entry, high};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    models::{KeyBranch, KeyId, StartOfShard, UtxoUnspent, UtxoUpdatePayload, WalletUtxoUpdate},
    storage::{ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_sdk_services::{
    notify::Notify,
    utxo_scanner::{StealthUtxoScannerWorker, UtxoScannerHandle},
};
use tari_template_lib::types::{constants::STEALTH_TARI_RESOURCE_ADDRESS, crypto::UtxoTag};

use crate::support::{ProvingNetwork, TestWithNetwork, random_keypair};

const SCAN_TIMEOUT: Duration = Duration::from_secs(60);

/// The shard the wallet's UTXO lives in, in the walked shard group.
const SHARD: Shard = Shard::from_u32(1);
/// The state version the latest verified checkpoint commits for [`SHARD`].
const CHECKPOINT_VERSION: u64 = 10;

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
    let (_join, handle) = StealthUtxoScannerWorker::new(test.sdk().clone(), Notify::new(100)).spawn();
    handle
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

fn queued_utxos(test: &TestWithNetwork<ProvingNetwork>) -> usize {
    test.store()
        .with_read_tx(|tx| tx.utxo_process_queue_fetch_batch(100))
        .unwrap()
        .get(&STEALTH_TARI_RESOURCE_ADDRESS)
        .map(HashMap::len)
        .unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cursor_claimed_past_the_verified_state_version_is_stored_at_it() {
    let (network, test) = anchored_wallet().await;
    let scanner = scanner(&test);

    network.answer_utxo_streams(|_| {
        vec![UtxoUpdatePayload {
            sos: Some(start_of_shard(u64::MAX, false)),
            update: None,
            eos: None,
        }]
    });
    scan(&scanner).await;
    assert_eq!(
        stored_cursors(&test).get(&SHARD),
        Some(&StateVersion::from(CHECKPOINT_VERSION))
    );

    let utxo = own_utxo(&test);
    serve_utxos(&network, vec![(CHECKPOINT_VERSION + 1, utxo)]);
    scan(&scanner).await;

    let requests = network.utxo_stream_requests();
    assert_eq!(cursor_of(&requests[1]), StateVersion::from(CHECKPOINT_VERSION));
    assert_eq!(
        queued_utxos(&test),
        1,
        "the UTXO committed after the checkpoint is found"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stored_cursor_past_the_verified_state_version_is_lowered_to_it() {
    let (network, test) = anchored_wallet().await;
    test.store()
        .with_write_tx(|tx| {
            tx.shard_state_version_set_many(
                &TestWithNetwork::<ProvingNetwork>::test_account_address(),
                &STEALTH_TARI_RESOURCE_ADDRESS,
                [(SHARD, StateVersion::from(u64::MAX))],
            )
        })
        .unwrap();
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
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_verified_checkpoint_a_scan_stores_no_cursor() {
    let (network, test) = anchored_wallet().await;
    network.withhold_checkpoints();
    serve_utxos(&network, vec![(CHECKPOINT_VERSION + 1, own_utxo(&test))]);

    scan(&scanner(&test)).await;

    assert_eq!(queued_utxos(&test), 1, "the scan still finds the UTXO");
    assert_eq!(stored_cursors(&test).get(&SHARD), None);
}
