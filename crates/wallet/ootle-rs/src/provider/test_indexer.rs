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
        fees::FeeReceipt,
        substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
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
    /// The index in `checkpoints` of the latest one.
    latest: usize,
    committees: HashMap<Epoch, (ValidatorSetCommittee, ValidatorSetCommittee)>,
    substates: HashMap<SubstateId, ServedSubstate>,
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
            committees,
            substates: HashMap::new(),
            results: HashMap::new(),
            hints: Vec::new(),
            committee_requests: 0,
            failing_result_requests: 0,
        }));
        let router = Router::new()
            .route("/epoch-checkpoints/latest", get(latest_checkpoint))
            .route("/epoch-checkpoints", get(list_checkpoints))
            .route("/committees", get(committee))
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

    pub(crate) fn set_latest(&self, latest: usize) {
        self.state.lock().unwrap().latest = latest;
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

async fn latest_checkpoint(State(state): Shared) -> Json<GetLatestEpochCheckpointResponse> {
    let state = state.lock().unwrap();
    Json(GetLatestEpochCheckpointResponse {
        checkpoint: state.checkpoints[state.latest].clone(),
    })
}

async fn list_checkpoints(
    State(state): Shared,
    Query(req): Query<ListEpochCheckpointsRequest>,
) -> Json<ListEpochCheckpointsResponse> {
    let state = state.lock().unwrap();
    let from_epoch = req.from_epoch.unwrap_or(Epoch(0));
    Json(ListEpochCheckpointsResponse {
        checkpoints: state.checkpoints[..=state.latest]
            .iter()
            .filter(|checkpoint| checkpoint.epoch >= from_epoch)
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
            substate: Box::new(served.value.clone()),
        },
        verified: true,
        proof: served.proof.clone().filter(|_| req.include_proof),
    })
    .into_response()
}

async fn fetch_substates(State(state): Shared, Json(req): Json<GetSubstatesRequest>) -> Json<GetSubstatesResponse> {
    let state = state.lock().unwrap();
    let mut substates = HashMap::new();
    let mut proofs = HashMap::new();
    for id in &req.requests {
        let Some(served) = state.substates.get(id) else {
            continue;
        };
        substates.insert(id.clone(), Substate::new(served.version, served.value.clone()));
        if let Some(proof) = served.proof.clone().filter(|_| req.include_proofs) {
            proofs.insert(id.clone(), proof);
        }
    }
    Json(GetSubstatesResponse { substates, proofs })
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
    GetTransactionResultResponse {
        result: IndexerTransactionFinalizedResult::Finalized {
            final_decision: Decision::Commit,
            execution_result: Some(Box::new(ExecuteResult {
                finalize,
                execution_time: Duration::from_secs(1),
                execute_epoch: None,
                wasm_execution_points: 0,
                native_execution_points: 0,
            })),
            execution_time: Duration::from_secs(1),
            finalized_time: time::macros::datetime!(2026-10-17 12:00),
            abort_details: None,
        },
        receipt: Some(ProvenTransactionReceipt {
            version: SubstateVersion::ZERO,
            receipt,
            proof: Some(proof),
        }),
    }
}
