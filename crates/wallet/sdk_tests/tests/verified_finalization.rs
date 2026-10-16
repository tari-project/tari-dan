//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A wallet that verifies its indexer commits a transaction only once the indexer proves the receipt the network
//! committed for it, against a validator set the wallet walked to from its anchor.

mod support;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tari_consensus_types::Decision;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, TransactionResult},
    fees::FeeReceipt,
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
    transaction_receipt::{DiffSummary, FinalizeOutcome, TransactionReceipt},
};
use tari_indexer_client::types::ProvenTransactionReceipt;
use tari_ootle_common_types::{Epoch, SubstateVersion};
use tari_ootle_light_client::test_fixtures::{
    CheckpointSpec,
    NETWORK,
    Signers,
    ValidatorSetFixture,
    checkpoint_entry,
    substate_proof,
};
use tari_ootle_transaction::{Transaction, TransactionId, args};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    apis::{config::ConfigKey, transaction::TransactionApiError, validator_set::ValidatorSetApiError},
    models::{TransactionStatus, ValidatorSetAnchor},
    network::{TransactionFinalizedResult, TransactionQueryResult},
    storage::{ReadableWalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_wallet_sdk_services::{
    notify::Notify,
    transaction_service::{TransactionService, TransactionServiceConfig},
};
use tari_shutdown::Shutdown;
use tari_template_lib::types::Hash32;
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::support::{ProvingNetwork, Test, TestWithNetwork};

fn now() -> PrimitiveDateTime {
    let now = OffsetDateTime::now_utc();
    PrimitiveDateTime::new(now.date(), now.time())
}

/// Validator sets of epochs 1 to 3.
fn sets() -> Vec<ValidatorSetFixture> {
    (1..=3).map(ValidatorSetFixture::new).collect()
}

fn build_transaction() -> Transaction {
    Transaction::builder_localnet(Epoch(100))
        .allocate_component_address("component")
        .put_last_instruction_output_on_workspace("bucket")
        .call_method("component", "new", args!["bucket"])
        .build_and_seal(&RistrettoSecretKey::from(1))
}

/// The result of `transaction_id` committing in `set`'s epoch, with its receipt proven at a block that `set`'s
/// committee of the receipt's shard group signed.
fn committed_result(transaction_id: TransactionId, set: &ValidatorSetFixture) -> TransactionQueryResult {
    let receipt_id = SubstateId::TransactionReceipt(transaction_id.into_receipt_address());
    let fee_receipt = FeeReceipt::builder()
        .with_total_fee_payment(1000)
        .with_total_fees_paid(800)
        .build();
    let receipt = TransactionReceipt {
        outcome: FinalizeOutcome::Commit,
        diff_summary: DiffSummary::from_diff(NETWORK, &SubstateDiff::new(), set.epoch),
        fee_withdrawals: Box::new([]),
        events: Box::new([]),
        fee_receipt: fee_receipt.clone(),
        epoch: set.epoch,
        intent_commitment: Hash32::from_array([3; 32]),
    };
    let mut diff = SubstateDiff::new();
    diff.up(
        receipt_id.clone(),
        Substate::new(SubstateVersion::ZERO, receipt.clone()),
    );
    let finalize = FinalizeResult::new(
        transaction_id.into_array().into(),
        vec![],
        vec![],
        TransactionResult::Accept(diff),
        fee_receipt,
    );
    let proof = substate_proof(
        set,
        &receipt_id,
        SubstateVersion::ZERO,
        &SubstateValue::from(receipt.clone()),
    );

    TransactionQueryResult {
        transaction_id,
        result: TransactionFinalizedResult::Finalized {
            final_decision: Decision::Commit,
            execution_result: Some(Box::new(ExecuteResult {
                finalize,
                execution_time: Duration::from_secs(1),
                execute_epoch: None,
                wasm_execution_points: 0,
                native_execution_points: 0,
            })),
            execution_time: Duration::from_secs(1),
            finalized_time: now(),
            abort_details: None,
        },
        receipt: Some(ProvenTransactionReceipt {
            version: SubstateVersion::ZERO,
            receipt,
            proof: Some(proof),
        }),
    }
}

/// A verifying wallet on `network` holding `transaction`, not yet finalized.
fn wallet(network: ProvingNetwork, transaction: &Transaction) -> TestWithNetwork<ProvingNetwork> {
    let test = TestWithNetwork::without_accounts_trusting(network, IndexerTrust::Verify);
    test.store()
        .with_write_tx(|tx| tx.transactions_insert(transaction, None, &[Test::test_account_address()], false))
        .unwrap();
    test
}

#[tokio::test]
async fn a_commit_with_a_proven_receipt_is_finalized_and_anchors_on_first_use() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 1);
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    network.set_result(committed_result(transaction_id, &sets[2]));
    let test = wallet(network, &transaction);

    let finalized = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap()
        .expect("the transaction is finalized");
    assert_eq!(finalized.status, TransactionStatus::Accepted);
    assert_eq!(finalized.final_fee, Some(800));

    let anchor = test.sdk().validator_set_api().anchor().unwrap().unwrap();
    assert_eq!(anchor.anchor, ValidatorSetAnchor::FirstUse);
    assert_eq!(anchor.validator_set.epoch(), Epoch(3));
    assert_eq!(*anchor.validator_set.validator_set_hash(), sets[2].hash());
}

#[tokio::test]
async fn a_commit_stays_pending_until_its_receipt_is_proven() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 1);
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let test = wallet(network.clone(), &transaction);

    let mut unproven = committed_result(transaction_id, &sets[2]);
    unproven.receipt.as_mut().unwrap().proof = None;
    let mut without_receipt = unproven.clone();
    without_receipt.receipt = None;
    for result in [unproven, without_receipt] {
        network.set_result(result);
        let pending = test
            .sdk()
            .transaction_api()
            .check_and_store_finalized_transaction(transaction_id)
            .await
            .unwrap();
        assert!(pending.is_none());
        let stored = test.sdk().transaction_api().get(transaction_id).unwrap();
        assert_eq!(stored.status, TransactionStatus::New);
        assert!(stored.finalize.is_none());
    }
}

#[tokio::test]
async fn a_receipt_proven_by_a_committee_outside_the_trusted_sets_is_refused() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 1);
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let impostors = ValidatorSetFixture::with_signers(NETWORK, 3, Signers::new(0xe0, 4), Signers::new(0xe1, 4));
    network.set_result(committed_result(transaction_id, &impostors));
    let test = wallet(network, &transaction);

    let err = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, TransactionApiError::FinalizationUnverified { .. }),
        "{err}"
    );
    let stored = test.sdk().transaction_api().get(transaction_id).unwrap();
    assert!(stored.finalize.is_none());
}

#[tokio::test]
async fn a_result_other_than_its_proven_receipt_commits_is_refused() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 1);
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let mut result = committed_result(transaction_id, &sets[2]);
    let TransactionFinalizedResult::Finalized { execution_result, .. } = &mut result.result else {
        unreachable!()
    };
    execution_result.as_mut().unwrap().finalize.fee_receipt = FeeReceipt::builder().with_total_fees_paid(1).build();
    network.set_result(result);
    let test = wallet(network, &transaction);

    let err = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, TransactionApiError::FinalizationUnverified { .. }),
        "{err}"
    );
    let stored = test.sdk().transaction_api().get(transaction_id).unwrap();
    assert!(stored.finalize.is_none());
}

#[tokio::test]
async fn the_wallet_walks_to_the_validator_set_that_proves_a_later_receipt() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 0);
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let test = wallet(network.clone(), &transaction);
    let anchored = test.sdk().validator_set_api().sync().await.unwrap();
    assert_eq!(anchored.epoch(), Epoch(2));

    network.set_latest(1);
    network.set_result(committed_result(transaction_id, &sets[2]));
    let finalized = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap()
        .expect("the transaction is finalized");
    assert_eq!(finalized.status, TransactionStatus::Accepted);

    let validator_set_api = test.sdk().validator_set_api();
    assert_eq!(validator_set_api.latest().unwrap().unwrap().epoch(), Epoch(3));
    let anchor = validator_set_api.anchor().unwrap().unwrap();
    assert_eq!(anchor.anchor, ValidatorSetAnchor::FirstUse);
    assert_eq!(anchor.validator_set.epoch(), Epoch(2));
    let walked = test
        .store()
        .with_read_tx(|tx| tx.trusted_validator_sets_get(Epoch(3)))
        .unwrap();
    assert_eq!(*walked.validator_set_hash(), sets[2].hash());
}

#[tokio::test]
async fn a_receipt_proven_in_an_earlier_epoch_verifies_against_the_set_kept_for_it() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 0);
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let test = wallet(network.clone(), &transaction);
    test.sdk().validator_set_api().sync().await.unwrap();
    network.set_latest(1);
    test.sdk().validator_set_api().sync().await.unwrap();

    // Proven at a block of epoch 2, after the wallet walked on to epoch 3.
    network.set_result(committed_result(transaction_id, &sets[1]));
    let finalized = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap()
        .expect("the transaction is finalized");
    assert_eq!(finalized.status, TransactionStatus::Accepted);
}

#[tokio::test]
async fn a_receipt_proven_before_the_anchor_stays_pending() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 1);
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    network.set_result(committed_result(transaction_id, &sets[1]));
    let test = wallet(network, &transaction);

    let pending = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap();
    assert!(
        pending.is_none(),
        "the wallet anchored on epoch 3 and trusts no set for 2"
    );
}

/// A network whose history breaks after epoch 2: its checkpoint ending epoch 2 names a validator set whose committee is
/// of epoch `far`, so no walk from epoch 2 connects to the latest checkpoint, which ends `far`.
fn broken_history(far: u64) -> (Vec<ValidatorSetFixture>, ProvingNetwork) {
    let sets = vec![
        ValidatorSetFixture::new(1),
        ValidatorSetFixture::new(2),
        ValidatorSetFixture::new(far),
        ValidatorSetFixture::new(far + 1),
    ];
    let network = ProvingNetwork::new(&sets, 0);
    (sets, network)
}

/// Sets the wallet's last validator set sync to `ago` before now.
fn last_synced(test: &TestWithNetwork<ProvingNetwork>, ago: Duration) {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap() - ago;
    test.store()
        .with_write_tx(|tx| tx.config_set(ConfigKey::ValidatorSetSyncedAt.as_key_str(), &at.as_secs(), false))
        .unwrap();
}

#[tokio::test]
async fn a_wallet_synced_recently_never_reanchors_on_a_claimed_gap() {
    let far = 600;
    let (sets, network) = broken_history(far);
    let transaction = build_transaction();
    let transaction_id = transaction.calculate_id();
    let test = wallet(network.clone(), &transaction);
    test.sdk().validator_set_api().sync().await.unwrap();

    // The indexer claims the network is `far` epochs on, and proves the receipt with the set it names there.
    network.set_latest(2);
    network.set_result(committed_result(transaction_id, &sets[3]));
    let err = test
        .sdk()
        .transaction_api()
        .check_and_store_finalized_transaction(transaction_id)
        .await
        .unwrap_err();
    assert!(matches!(err, TransactionApiError::ValidatorSet(_)), "{err}");

    let stored = test.sdk().transaction_api().get(transaction_id).unwrap();
    assert!(stored.finalize.is_none());
    let anchor = test.sdk().validator_set_api().anchor().unwrap().unwrap();
    assert_eq!(anchor.anchor, ValidatorSetAnchor::FirstUse);
    assert_eq!(anchor.validator_set.epoch(), Epoch(2));
}

#[tokio::test]
async fn a_wallet_out_of_sync_long_enough_reanchors_when_the_walk_fails() {
    let far = 600;
    let (_, network) = broken_history(far);
    let transaction = build_transaction();
    let test = wallet(network.clone(), &transaction);
    let validator_set_api = test.sdk().validator_set_api();
    validator_set_api.sync().await.unwrap();

    network.set_latest(2);
    last_synced(&test, Duration::from_secs(365 * 24 * 60 * 60));
    let trusted = validator_set_api.sync().await.unwrap();
    assert_eq!(trusted.epoch(), Epoch(far + 1));
    assert_eq!(trusted.anchor_epoch(), Epoch(far + 1));
    let anchor = validator_set_api.anchor().unwrap().unwrap();
    assert_eq!(anchor.anchor, ValidatorSetAnchor::Reanchor);
    assert_eq!(anchor.validator_set.epoch(), Epoch(far + 1));
    let forgotten = test.store().with_read_tx(|tx| tx.trusted_validator_sets_get(Epoch(2)));
    assert!(forgotten.is_err(), "the old chain is forgotten");
}

#[tokio::test]
async fn a_wallet_far_behind_walks_the_whole_way() {
    let sets = (1..=510).map(ValidatorSetFixture::new).collect::<Vec<_>>();
    let network = ProvingNetwork::new(&sets, 0);
    let transaction = build_transaction();
    let test = wallet(network.clone(), &transaction);
    let validator_set_api = test.sdk().validator_set_api();
    validator_set_api.sync().await.unwrap();

    network.set_latest(sets.len() - 2);
    last_synced(&test, Duration::from_secs(365 * 24 * 60 * 60));
    let trusted = validator_set_api.sync().await.unwrap();
    assert_eq!(trusted.epoch(), Epoch(510));
    let anchor = validator_set_api.anchor().unwrap().unwrap();
    assert_eq!(anchor.anchor, ValidatorSetAnchor::FirstUse);
    assert_eq!(anchor.validator_set.epoch(), Epoch(2));
}

#[tokio::test]
async fn a_walk_the_indexer_withholds_fails_and_keeps_the_trusted_set() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 0);
    let transaction = build_transaction();
    let test = wallet(network.clone(), &transaction);
    let validator_set_api = test.sdk().validator_set_api();
    validator_set_api.sync().await.unwrap();

    network.set_latest(1);
    network.withhold_walk();
    let err = validator_set_api.sync().await.unwrap_err();
    assert!(
        matches!(err, ValidatorSetApiError::CheckpointNotServed { epoch } if epoch == Epoch(2)),
        "{err}"
    );
    assert_eq!(validator_set_api.latest().unwrap().unwrap().epoch(), Epoch(2));
}

#[tokio::test]
async fn concurrent_syncs_walk_once() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 0);
    let transaction = build_transaction();
    let test = wallet(network.clone(), &transaction);
    test.sdk().validator_set_api().sync().await.unwrap();

    network.set_latest(1);
    let (first_api, second_api) = (test.sdk().validator_set_api(), test.sdk().validator_set_api());
    let (first, second) = tokio::join!(first_api.sync(), second_api.sync());
    assert_eq!(first.unwrap().epoch(), Epoch(3));
    assert_eq!(second.unwrap().epoch(), Epoch(3));
    assert_eq!(network.walk_requests(), 1);
}

#[tokio::test]
async fn the_wallet_anchors_before_its_first_submission() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 1);
    let transaction = build_transaction();
    let test = wallet(network, &transaction);
    assert!(test.sdk().validator_set_api().anchor().unwrap().is_none());

    assert!(
        test.sdk()
            .transaction_api()
            .submit_transaction(transaction.calculate_id())
            .await
            .unwrap()
    );

    let anchor = test.sdk().validator_set_api().anchor().unwrap().unwrap();
    assert_eq!(anchor.anchor, ValidatorSetAnchor::FirstUse);
    assert_eq!(anchor.validator_set.epoch(), Epoch(3));
}

#[tokio::test]
async fn a_walk_that_keeps_failing_for_a_day_reanchors() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 0);
    let transaction = build_transaction();
    let test = wallet(network.clone(), &transaction);
    let validator_set_api = test.sdk().validator_set_api();
    validator_set_api.sync().await.unwrap();

    // The next checkpoint is signed by a committee outside the trusted set, so no walk connects.
    let impostors = Signers::new(0xee, 4);
    network.replace_checkpoint(
        1,
        checkpoint_entry(CheckpointSpec {
            signers: &impostors,
            ..CheckpointSpec::handover(&sets[1], &sets[2])
        }),
    );
    network.set_latest(1);
    let err = validator_set_api.sync().await.unwrap_err();
    assert!(matches!(err, ValidatorSetApiError::Unverified(_)), "{err}");
    assert_eq!(
        validator_set_api.anchor().unwrap().unwrap().anchor,
        ValidatorSetAnchor::FirstUse,
        "a walk that just started failing is no reason to re-anchor"
    );

    let a_day_ago = SystemTime::now().duration_since(UNIX_EPOCH).unwrap() - Duration::from_secs(25 * 60 * 60);
    test.store()
        .with_write_tx(|tx| {
            tx.config_set(
                ConfigKey::ValidatorSetWalkFailingSince.as_key_str(),
                &Some(a_day_ago.as_secs()),
                false,
            )
        })
        .unwrap();
    let trusted = validator_set_api.sync().await.unwrap();
    assert_eq!(trusted.epoch(), Epoch(3));
    let anchor = validator_set_api.anchor().unwrap().unwrap();
    assert_eq!(anchor.anchor, ValidatorSetAnchor::Reanchor);
    assert_eq!(anchor.validator_set.epoch(), Epoch(3));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_transaction_service_keeps_the_trusted_validator_set_up_to_date() {
    let sets = sets();
    let network = ProvingNetwork::new(&sets, 1);
    // No transaction to submit, so nothing but the periodic sync anchors.
    let test = TestWithNetwork::without_accounts_trusting(network, IndexerTrust::Verify);
    let config = TransactionServiceConfig {
        validator_set_sync_interval: Duration::from_millis(20),
        ..TransactionServiceConfig::default()
    };
    let shutdown = Shutdown::new();
    let (service, _handle) =
        TransactionService::with_config(config, Notify::new(16), test.sdk().clone(), shutdown.to_signal());
    tokio::spawn(service.run());

    let anchored = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(anchor) = test.sdk().validator_set_api().anchor().unwrap() {
                break anchor;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the service never synced the trusted validator set");
    assert_eq!(anchored.validator_set.epoch(), Epoch(3));
}

#[tokio::test]
async fn a_wallet_out_of_sync_for_less_than_the_gap_takes_does_not_reanchor() {
    let far = 600;
    let (_, network) = broken_history(far);
    let transaction = build_transaction();
    let test = wallet(network.clone(), &transaction);
    let validator_set_api = test.sdk().validator_set_api();
    validator_set_api.sync().await.unwrap();

    // 600 epochs take at least 150 hours at 15 minutes an epoch.
    network.set_latest(2);
    last_synced(&test, Duration::from_secs(72 * 60 * 60));
    let err = validator_set_api.sync().await.unwrap_err();
    assert!(matches!(err, ValidatorSetApiError::Unverified(_)), "{err}");
    let anchor = validator_set_api.anchor().unwrap().unwrap();
    assert_eq!(anchor.anchor, ValidatorSetAnchor::FirstUse);
}
