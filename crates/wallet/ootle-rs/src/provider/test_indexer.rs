//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json,
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        IntoResponse,
        Response,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use futures::{StreamExt, stream};
use tari_consensus_types::Decision;
use tari_indexer_client::{
    event::TransactionFinalizedEvent,
    types::{
        EpochCheckpointEntry,
        GetCommitteeRequest,
        GetCommitteeResponse,
        GetEpochManagerStatsResponse,
        GetLatestEpochCheckpointRequest,
        GetLatestEpochCheckpointResponse,
        GetSubstateRequest,
        GetSubstateResponse,
        GetSubstatesRequest,
        GetSubstatesResponse,
        GetTransactionResultQuery,
        GetTransactionResultResponse,
        IndexerTransactionFinalizedResult,
        ListEpochCheckpointsRequest,
        ListEpochCheckpointsResponse,
        ProvenTransactionReceipt,
        SubstateProof,
        SubstateState,
        ValidatorSetCommittee,
    },
};
use tari_ootle_common_types::{
    Epoch,
    SubstateVersion,
    engine_types::{
        commit_result::{ExecuteResult, FinalizeResult, TransactionResult},
        encoded::Encoded,
        fees::FeeReceipt,
        substate::{EncodedSubstate, Substate, SubstateDiff, SubstateId, SubstateValue},
        transaction_receipt::{DiffSummary, FinalizeOutcome, TransactionReceipt},
    },
};
use tari_ootle_light_client::test_fixtures::{
    CheckpointSpec,
    NETWORK,
    ValidatorSetFixture,
    checkpoint_entry,
    high,
    low,
    substate_proof,
};
use tari_ootle_transaction::TransactionId;
use tari_template_lib_types::Hash32;
use tokio::net::TcpListener;

use crate::{
    provider::{IndexerProvider, IndexerTrust},
    wallet::NoWallet,
};

/// An indexer that serves a chain of signed end-of-epoch checkpoints, the committees of every epoch's validator set,
/// and the substates and transaction results it is given to serve.
#[derive(Clone)]
pub(crate) struct TestIndexer {
    state: Arc<Mutex<TestIndexerState>>,
    url: String,
}

struct TestIndexerState {
    /// Ascending by the epoch each ends.
    checkpoints: Vec<EpochCheckpointEntry>,
    /// The index in `checkpoints` of the latest one, or `None` before the network's first checkpoint.
    latest: Option<usize>,
    /// The latest checkpoint served for a shard of each of these checkpoints' shard groups, in place of the latest in
    /// `checkpoints`.
    latest_of_group: Vec<EpochCheckpointEntry>,
    /// The epoch of the first validator set.
    first_epoch: Epoch,
    /// Whether the indexer has observed the epoch the validators execute in.
    observes_consensus_epoch: bool,
    /// The epoch whose checkpoint the indexer has not stored yet, as during a backfill.
    unserved: Option<Epoch>,
    committees: HashMap<Epoch, (ValidatorSetCommittee, ValidatorSetCommittee)>,
    substates: HashMap<SubstateId, ServedSubstate>,
    /// A served substate every batch fetch answers with, requested or not.
    unrequested_on_fetch: Option<SubstateId>,
    /// The number of substates each batch fetch requested.
    fetch_sizes: Vec<usize>,
    results: HashMap<TransactionId, GetTransactionResultResponse>,
    /// The finalization events every event stream starts with.
    hints: Vec<TransactionFinalizedEvent>,
    committee_requests: usize,
    /// How many of the next transaction result requests fail.
    failing_result_requests: usize,
}

struct ServedSubstate {
    version: SubstateVersion,
    value: SubstateValue,
    proof: Option<SubstateProof>,
}

impl TestIndexer {
    /// Serves the handovers between consecutive `sets`, with the latest checkpoint the one ending `sets[latest]`'s
    /// epoch.
    pub(crate) async fn spawn(sets: &[ValidatorSetFixture], latest: usize) -> Self {
        Self::spawn_with_latest(sets, Some(latest)).await
    }

    /// Serves the committees of `sets` and no checkpoint, as on a network still in the epoch of `sets[0]`, its first.
    /// [`Self::set_latest`] serves the handovers between them.
    pub(crate) async fn spawn_before_first_checkpoint(sets: &[ValidatorSetFixture]) -> Self {
        Self::spawn_with_latest(sets, None).await
    }

    async fn spawn_with_latest(sets: &[ValidatorSetFixture], latest: Option<usize>) -> Self {
        let checkpoints = sets
            .windows(2)
            .map(|window| checkpoint_entry(CheckpointSpec::handover(&window[0], &window[1])))
            .collect();
        let committees = sets
            .iter()
            .map(|set| (set.epoch, (set.committee(low()), set.committee(high()))))
            .collect();
        let state = Arc::new(Mutex::new(TestIndexerState {
            checkpoints,
            latest,
            latest_of_group: Vec::new(),
            first_epoch: sets[0].epoch,
            observes_consensus_epoch: true,
            unserved: None,
            committees,
            substates: HashMap::new(),
            unrequested_on_fetch: None,
            fetch_sizes: Vec::new(),
            results: HashMap::new(),
            hints: Vec::new(),
            committee_requests: 0,
            failing_result_requests: 0,
        }));
        let router = Router::new()
            .route("/epoch-checkpoints/latest", get(latest_checkpoint))
            .route("/epoch-checkpoints", get(list_checkpoints))
            .route("/committees", get(committee))
            .route("/epoch-manager/stats", get(epoch_manager_stats))
            .route("/substates/fetch", post(fetch_substates))
            .route("/substates/{id}", get(substate))
            .route("/transactions/{id}/result", get(transaction_result))
            .route("/events", get(events))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { state, url }
    }

    pub(crate) fn provider(&self, indexer_trust: IndexerTrust) -> IndexerProvider<NoWallet> {
        let client = tari_indexer_client::connect_rest(&self.url).unwrap();
        IndexerProvider::new(client, NoWallet, NETWORK)
            .with_indexer_trust(indexer_trust)
            .with_transaction_timeout(Duration::from_secs(2))
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// Serves `checkpoint` after the last one served before.
    pub(crate) fn push_checkpoint(&self, checkpoint: EpochCheckpointEntry) {
        self.state.lock().unwrap().checkpoints.push(checkpoint);
    }

    /// Leaves the checkpoint of `epoch` out of the pages of checkpoints the indexer lists.
    pub(crate) fn leave_unserved(&self, epoch: Epoch) {
        self.state.lock().unwrap().unserved = Some(epoch);
    }

    /// Sets whether the indexer has observed the epoch the validators execute in, which it has not before it sees a
    /// block from every committee.
    pub(crate) fn set_observes_consensus_epoch(&self, observes: bool) {
        self.state.lock().unwrap().observes_consensus_epoch = observes;
    }

    /// Serves `checkpoint` as the latest checkpoint of each shard of its shard group, in place of the latest one in
    /// the served chain.
    pub(crate) fn serve_latest_of_group(&self, checkpoint: EpochCheckpointEntry) {
        let mut state = self.state.lock().unwrap();
        state
            .latest_of_group
            .retain(|served| served.shard_group != checkpoint.shard_group);
        state.latest_of_group.push(checkpoint);
    }

    pub(crate) fn set_latest(&self, latest: usize) {
        self.state.lock().unwrap().latest = Some(latest);
    }

    pub(crate) fn committee_requests(&self) -> usize {
        self.state.lock().unwrap().committee_requests
    }

    /// Serves `id` at `version` holding `value`, with `proof` to requests for proofs.
    pub(crate) fn serve_substate(
        &self,
        id: SubstateId,
        version: SubstateVersion,
        value: SubstateValue,
        proof: Option<SubstateProof>,
    ) {
        self.state
            .lock()
            .unwrap()
            .substates
            .insert(id, ServedSubstate { version, value, proof });
    }

    /// Answers every batch fetch with the served substate `id` as well, whether requested or not.
    pub(crate) fn serve_unrequested_on_fetch(&self, id: SubstateId) {
        self.state.lock().unwrap().unrequested_on_fetch = Some(id);
    }

    /// The number of substates each batch fetch so far requested.
    pub(crate) fn fetch_sizes(&self) -> Vec<usize> {
        self.state.lock().unwrap().fetch_sizes.clone()
    }

    pub(crate) fn set_result(&self, transaction_id: TransactionId, result: GetTransactionResultResponse) {
        self.state.lock().unwrap().results.insert(transaction_id, result);
    }

    /// Fails the next `n` transaction result requests.
    pub(crate) fn fail_result_requests(&self, n: usize) {
        self.state.lock().unwrap().failing_result_requests = n;
    }

    /// Announces that `transaction_id` committed on every event stream.
    pub(crate) fn hint_commit(&self, transaction_id: TransactionId) {
        self.state.lock().unwrap().hints.push(TransactionFinalizedEvent {
            transaction_id,
            outcome: FinalizeOutcome::Commit,
        });
    }
}

type Shared = State<Arc<Mutex<TestIndexerState>>>;

async fn latest_checkpoint(State(state): Shared, Query(req): Query<GetLatestEpochCheckpointRequest>) -> Response {
    let state = state.lock().unwrap();
    if let Some(shard) = req.shard &&
        let Some(checkpoint) = state
            .latest_of_group
            .iter()
            .find(|checkpoint| checkpoint.shard_group.contains(&shard))
    {
        return Json(GetLatestEpochCheckpointResponse {
            checkpoint: checkpoint.clone(),
        })
        .into_response();
    }
    let Some(latest) = state.latest else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Json(GetLatestEpochCheckpointResponse {
        checkpoint: state.checkpoints[latest].clone(),
    })
    .into_response()
}

/// The epoch after the latest checkpoint's as both the current and the consensus epoch, or no consensus epoch while it
/// is unobserved.
async fn epoch_manager_stats(State(state): Shared) -> Json<GetEpochManagerStatsResponse> {
    let state = state.lock().unwrap();
    let epoch = state
        .latest
        .map_or(state.first_epoch, |latest| state.checkpoints[latest].epoch + Epoch(1));
    Json(GetEpochManagerStatsResponse {
        current_epoch: epoch,
        consensus_epoch: state.observes_consensus_epoch.then_some(epoch),
        current_block_height: 0,
        current_block_hash: Hash32::default(),
    })
}

async fn list_checkpoints(
    State(state): Shared,
    Query(req): Query<ListEpochCheckpointsRequest>,
) -> Json<ListEpochCheckpointsResponse> {
    let state = state.lock().unwrap();
    let from_epoch = req.from_epoch.unwrap_or(Epoch(0));
    Json(ListEpochCheckpointsResponse {
        checkpoints: state
            .latest
            .map_or(&[][..], |latest| &state.checkpoints[..=latest])
            .iter()
            .filter(|checkpoint| checkpoint.epoch >= from_epoch && Some(checkpoint.epoch) != state.unserved)
            .take(req.limit.unwrap_or(20) as usize)
            .cloned()
            .collect(),
    })
}

async fn committee(State(state): Shared, Query(req): Query<GetCommitteeRequest>) -> Json<GetCommitteeResponse> {
    let mut state = state.lock().unwrap();
    state.committee_requests += 1;
    let (low, high) = state
        .committees
        .get(&req.epoch)
        .expect("no validator set for the epoch");
    let committee = if low.shard_group.contains(&req.shard) {
        low
    } else {
        high
    };
    Json(GetCommitteeResponse {
        committee: committee.clone(),
    })
}

async fn substate(State(state): Shared, Path(id): Path<String>, Query(req): Query<GetSubstateRequest>) -> Response {
    let id = id.parse::<SubstateId>().unwrap();
    let state = state.lock().unwrap();
    let Some(served) = state.substates.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Json(GetSubstateResponse {
        version: served.version,
        state: SubstateState::Up {
            substate: Encoded::encode(&served.value).unwrap(),
        },
        verified: true,
        proof: served.proof.clone().filter(|_| req.include_proof),
    })
    .into_response()
}

async fn fetch_substates(State(state): Shared, Json(req): Json<GetSubstatesRequest>) -> Json<GetSubstatesResponse> {
    let mut state = state.lock().unwrap();
    state.fetch_sizes.push(req.requests.len());
    let mut substates = HashMap::new();
    let mut proofs = Vec::new();
    for id in req.requests.iter().chain(state.unrequested_on_fetch.iter()) {
        let Some(served) = state.substates.get(id) else {
            continue;
        };
        substates.insert(
            id.clone(),
            EncodedSubstate::new(served.version, Encoded::encode(&served.value).unwrap()),
        );
        if let Some(proof) = served.proof.clone().filter(|_| req.include_proofs) {
            proofs.push((id.clone(), proof));
        }
    }
    Json(GetSubstatesResponse::new(substates, proofs))
}

async fn transaction_result(
    State(state): Shared,
    Path(id): Path<String>,
    Query(query): Query<GetTransactionResultQuery>,
) -> Response {
    let mut state = state.lock().unwrap();
    if state.failing_result_requests > 0 {
        state.failing_result_requests -= 1;
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Some(result) = state
        .results
        .iter()
        .find(|(transaction_id, _)| transaction_id.to_string() == id)
        .map(|(_, result)| result)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut result = result.clone();
    if !query.include_proof {
        result.receipt = None;
    }
    Json(result).into_response()
}

async fn events(State(state): Shared) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let hints = state
        .lock()
        .unwrap()
        .hints
        .iter()
        .map(|hint| {
            Ok(Event::default()
                .event("TransactionFinalized")
                .data(serde_json::to_string(hint).unwrap()))
        })
        .collect::<Vec<_>>();
    Sse::new(stream::iter(hints).chain(stream::pending()))
}

/// The result of `transaction_id` committing in `set`'s epoch, with its receipt proven at a block that `set`'s
/// committee of the receipt's shard group signed.
pub(crate) fn committed_result(
    transaction_id: TransactionId,
    set: &ValidatorSetFixture,
) -> GetTransactionResultResponse {
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
    let execute_result = ExecuteResult {
        finalize,
        execution_time: Duration::from_secs(1),
        execute_epoch: None,
        wasm_execution_points: 0,
        native_execution_points: 0,
    };
    GetTransactionResultResponse {
        result: IndexerTransactionFinalizedResult::Finalized {
            final_decision: Decision::Commit,
            execution_result: Some(Encoded::encode(&execute_result).unwrap()),
            execution_time: Duration::from_secs(1),
            finalized_time: time::macros::datetime!(2026-10-17 12:00),
            abort_details: None,
        },
        receipt: Some(ProvenTransactionReceipt {
            version: SubstateVersion::ZERO,
            value: Encoded::encode(&SubstateValue::from(receipt)).unwrap(),
            proof: Some(proof),
        }),
    }
}
