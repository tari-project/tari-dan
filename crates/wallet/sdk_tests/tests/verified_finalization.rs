//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! A wallet that verifies its indexer commits a transaction only once the indexer proves the receipt the network
//! committed for it, against a validator set the wallet walked to from its anchor.

mod support;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::StreamExt;
use tari_consensus_types::Decision;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_engine_types::{
    Utxo,
    commit_result::{ExecuteResult, FinalizeResult, TransactionResult},
    fees::FeeReceipt,
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
    transaction_receipt::{DiffSummary, FinalizeOutcome, TransactionReceipt},
};
use tari_indexer_client::types::{
    EpochCheckpointEntry,
    ProvenTransactionReceipt,
    ValidatorSetCommittee,
    WatchedSubstateItem,
};
use tari_ootle_common_types::{Epoch, StateVersion, SubstateVersion, shard::Shard};
use tari_ootle_light_client::test_fixtures::{
    CheckpointSpec,
    NETWORK,
    Signers,
    ValidatorSetFixture,
    checkpoint_entry,
    high,
    low,
    substate_proof,
};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId, args};
use tari_ootle_wallet_sdk::{
    IndexerTrust,
    apis::{transaction::TransactionApiError, validator_set::ValidatorSetApiError},
    models::{TransactionStatus, ValidatorSetAnchor},
    network::{
        SubstateQueryResult,
        TransactionFinalizedResult,
        TransactionFinalizedStream,
        TransactionQueryResult,
        UtxoUpdateStream,
        WalletNetworkInterface,
    },
    storage::{ReadableWalletStore, TagAndPublicNoncePair, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};
use tari_template_abi::TemplateDef;
use tari_template_lib::types::{Hash32, ResourceAddress, TemplateAddress, UtxoId};
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::support::{PanicError, Test, TestWithNetwork};

/// An indexer that serves a chain of signed end-of-epoch checkpoints, the committees of every epoch's validator set,
/// and one transaction result.
#[derive(Clone)]
struct ProvingNetwork {
    state: Arc<Mutex<ProvingNetworkState>>,
}

struct ProvingNetworkState {
    /// Ascending by the epoch each ends.
    checkpoints: Vec<EpochCheckpointEntry>,
    /// The index in `checkpoints` of the latest one.
    latest: usize,
    committees: HashMap<Epoch, (ValidatorSetCommittee, ValidatorSetCommittee)>,
    result: Option<TransactionQueryResult>,
    /// Whether the indexer serves the latest checkpoint but none of the ones before it.
    withholds_walk: bool,
}

impl ProvingNetwork {
    /// Serves the handovers between consecutive `sets`, with the latest checkpoint the one ending `sets[latest]`'s
    /// epoch.
    fn new(sets: &[ValidatorSetFixture], latest: usize) -> Self {
        let checkpoints = sets
            .windows(2)
            .map(|window| checkpoint_entry(CheckpointSpec::handover(&window[0], &window[1])))
            .collect();
        let committees = sets
            .iter()
            .map(|set| (set.epoch, (set.committee(low()), set.committee(high()))))
            .collect();
        Self {
            state: Arc::new(Mutex::new(ProvingNetworkState {
                checkpoints,
                latest,
                committees,
                result: None,
                withholds_walk: false,
            })),
        }
    }

    fn set_latest(&self, latest: usize) {
        self.state.lock().unwrap().latest = latest;
    }

    fn withhold_walk(&self) {
        self.state.lock().unwrap().withholds_walk = true;
    }

    fn set_result(&self, result: TransactionQueryResult) {
        self.state.lock().unwrap().result = Some(result);
    }
}

impl WalletNetworkInterface for ProvingNetwork {
    type Error = PanicError;

    async fn query_substate(
        &self,
        _address: &SubstateId,
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn get_substates(&self, _: Vec<SubstateId>) -> Result<HashMap<SubstateId, Substate>, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn submit_transaction(&self, _transaction: Transaction) -> Result<TransactionId, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn submit_transaction_envelope(
        &self,
        _transaction: TransactionEnvelope,
    ) -> Result<TransactionId, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn submit_dry_run_transaction(
        &self,
        _transaction: Transaction,
    ) -> Result<TransactionQueryResult, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn query_transaction_result(
        &self,
        _transaction_id: TransactionId,
        include_proof: bool,
    ) -> Result<TransactionQueryResult, Self::Error> {
        assert!(include_proof, "a verifying wallet asks for the proof");
        Ok(self
            .state
            .lock()
            .unwrap()
            .result
            .clone()
            .expect("no transaction result"))
    }

    async fn subscribe_transaction_finalized(&self) -> Result<TransactionFinalizedStream<Self::Error>, Self::Error> {
        Ok(futures::stream::pending().boxed())
    }

    async fn fetch_template_definition(&self, _template_address: TemplateAddress) -> Result<TemplateDef, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn stream_stealth_utxo_updates(
        &self,
        _from_epoch: Epoch,
        _resource_address: ResourceAddress,
        _shard_state_versions: Vec<(Shard, StateVersion)>,
        _unspent_only: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn list_watched_substates(
        &self,
        _template_address: Option<TemplateAddress>,
        _limit: Option<u64>,
        _offset: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn get_unspent_utxos(
        &self,
        _resource_address: ResourceAddress,
        _tag_and_nonce_pairs: Vec<TagAndPublicNoncePair>,
    ) -> Result<Vec<(UtxoId, Utxo)>, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn get_latest_epoch_checkpoint(&self, _shard: Shard) -> Result<EpochCheckpointEntry, Self::Error> {
        let state = self.state.lock().unwrap();
        Ok(state.checkpoints[state.latest].clone())
    }

    async fn list_epoch_checkpoints(
        &self,
        from_epoch: Epoch,
        limit: u32,
        _shard: Shard,
    ) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        let state = self.state.lock().unwrap();
        if state.withholds_walk {
            return Ok(vec![]);
        }
        Ok(state.checkpoints[..=state.latest]
            .iter()
            .filter(|checkpoint| checkpoint.epoch >= from_epoch)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn get_committee(&self, epoch: Epoch, shard: Shard) -> Result<ValidatorSetCommittee, Self::Error> {
        let state = self.state.lock().unwrap();
        let (low, high) = state.committees.get(&epoch).expect("no validator set for the epoch");
        Ok(if low.shard_group.contains(&shard) {
            low.clone()
        } else {
            high.clone()
        })
    }

    async fn get_current_epoch(&self) -> Result<Epoch, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

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

#[tokio::test]
async fn a_wallet_too_far_behind_to_walk_reanchors_and_forgets_the_old_chain() {
    let far = 600;
    let sets = vec![
        ValidatorSetFixture::new(1),
        ValidatorSetFixture::new(2),
        ValidatorSetFixture::new(far),
        ValidatorSetFixture::new(far + 1),
    ];
    let network = ProvingNetwork::new(&sets, 0);
    let transaction = build_transaction();
    let test = wallet(network.clone(), &transaction);
    let validator_set_api = test.sdk().validator_set_api();
    validator_set_api.sync().await.unwrap();

    network.set_latest(2);
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
