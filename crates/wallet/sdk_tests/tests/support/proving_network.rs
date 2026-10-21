//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use futures::StreamExt;
use tari_engine_types::{
    Utxo,
    substate::{Substate, SubstateId, SubstateValue},
};
use tari_indexer_client::types::{EpochCheckpointEntry, SubstateProof, ValidatorSetCommittee, WatchedSubstateItem};
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    StateVersion,
    SubstateVersion,
    optional::IsNotFoundError,
    response_status::{ResponseErrorStatus, TransactionStatusResponseError},
    shard::Shard,
};
use tari_ootle_light_client::test_fixtures::{CheckpointSpec, ValidatorSetFixture, checkpoint_entry, high, low};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId};
use tari_ootle_wallet_sdk::{
    models::UtxoUpdatePayload,
    network::{
        FetchedSubstates,
        FetchedUtxos,
        SubstateQueryResult,
        TransactionFinalizedStream,
        TransactionQueryResult,
        UtxoUpdateStream,
        WalletNetworkInterface,
    },
    storage::TagAndPublicNoncePair,
};
use tari_template_abi::TemplateDef;
use tari_template_lib::types::{ResourceAddress, TemplateAddress, UtxoId};

/// Answers a stealth UTXO stream request, given the cursor of each shard it asks from.
type UtxoStreamAnswer = Arc<dyn Fn(&[(Shard, StateVersion)]) -> Vec<UtxoUpdatePayload> + Send + Sync>;

/// An indexer that serves a chain of signed end-of-epoch checkpoints, the committees of every epoch's validator set,
/// one transaction result, and the substates it is given to serve.
#[derive(Clone)]
pub struct ProvingNetwork {
    state: Arc<Mutex<ProvingNetworkState>>,
}

struct ProvingNetworkState {
    /// Ascending by the epoch each ends.
    checkpoints: Vec<EpochCheckpointEntry>,
    /// The index in `checkpoints` of the latest one, or `None` while the network has ended no epoch.
    latest: Option<usize>,
    /// The epoch of the first validator set.
    first_epoch: Epoch,
    /// Whether the indexer fails every request about the validator sets, as when it cannot be reached.
    unreachable: bool,
    /// Whether the indexer has observed the epoch the validators execute in.
    observes_consensus_epoch: bool,
    committees: HashMap<Epoch, (ValidatorSetCommittee, ValidatorSetCommittee)>,
    result: Option<TransactionQueryResult>,
    /// Whether the indexer serves the latest checkpoint but none of the ones before it.
    withholds_walk: bool,
    /// The epoch whose checkpoint the indexer has not stored yet, as during a backfill.
    unserved: Option<Epoch>,
    /// Requests for a page of checkpoints to walk through.
    walk_requests: usize,
    substates: HashMap<SubstateId, ServedSubstate>,
    /// Requests for a committee.
    committee_requests: usize,
    /// Transactions submitted to the network.
    submissions: usize,
    /// Requests for a single substate.
    substate_queries: usize,
    /// Requests for a batch of substates.
    substate_fetches: usize,
    /// The latest checkpoint of each shard group other than the walked one.
    shard_group_checkpoints: HashMap<ShardGroup, EpochCheckpointEntry>,
    utxo_stream_answer: Option<UtxoStreamAnswer>,
    /// The shard cursors of each stealth UTXO stream request.
    utxo_stream_requests: Vec<Vec<(Shard, StateVersion)>>,
}

struct ServedSubstate {
    version: SubstateVersion,
    value: SubstateValue,
    proof: Option<SubstateProof>,
}

impl ProvingNetwork {
    /// Serves the handovers between consecutive `sets`, with the latest checkpoint the one ending `sets[latest]`'s
    /// epoch.
    pub fn new(sets: &[ValidatorSetFixture], latest: usize) -> Self {
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
                latest: Some(latest),
                first_epoch: sets[0].epoch,
                unreachable: false,
                observes_consensus_epoch: true,
                committees,
                result: None,
                withholds_walk: false,
                unserved: None,
                walk_requests: 0,
                substates: HashMap::new(),
                committee_requests: 0,
                submissions: 0,
                substate_queries: 0,
                substate_fetches: 0,
                shard_group_checkpoints: HashMap::new(),
                utxo_stream_answer: None,
                utxo_stream_requests: Vec::new(),
            })),
        }
    }

    /// Serves `set`'s committees for its epoch in place of the ones served before.
    pub fn replace_committees(&self, set: &ValidatorSetFixture) {
        self.state
            .lock()
            .unwrap()
            .committees
            .insert(set.epoch, (set.committee(low()), set.committee(high())));
    }

    pub fn set_latest(&self, latest: usize) {
        self.state.lock().unwrap().latest = Some(latest);
    }

    /// Serves no checkpoint, as in a network's first epoch.
    pub fn withhold_checkpoints(&self) {
        self.state.lock().unwrap().latest = None;
    }

    /// Fails every request about the validator sets until [`Self::reconnect`].
    pub fn disconnect(&self) {
        self.state.lock().unwrap().unreachable = true;
    }

    pub fn reconnect(&self) {
        self.state.lock().unwrap().unreachable = false;
    }

    /// Sets whether the indexer has observed the epoch the validators execute in, which it has not before it sees a
    /// block from every committee.
    pub fn set_observes_consensus_epoch(&self, observes: bool) {
        self.state.lock().unwrap().observes_consensus_epoch = observes;
    }

    pub fn submissions(&self) -> usize {
        self.state.lock().unwrap().submissions
    }

    pub fn replace_checkpoint(&self, index: usize, checkpoint: EpochCheckpointEntry) {
        self.state.lock().unwrap().checkpoints[index] = checkpoint;
    }

    pub fn committee_requests(&self) -> usize {
        self.state.lock().unwrap().committee_requests
    }

    pub fn substate_queries(&self) -> usize {
        self.state.lock().unwrap().substate_queries
    }

    pub fn substate_fetches(&self) -> usize {
        self.state.lock().unwrap().substate_fetches
    }

    pub fn walk_requests(&self) -> usize {
        self.state.lock().unwrap().walk_requests
    }

    pub fn withhold_walk(&self) {
        self.state.lock().unwrap().withholds_walk = true;
    }

    /// Leaves the checkpoint of `epoch` out of the pages of checkpoints the indexer lists.
    pub fn leave_unserved(&self, epoch: Epoch) {
        self.state.lock().unwrap().unserved = Some(epoch);
    }

    /// Serves `id` at `version` holding `value`, with `proof` to requests for proofs.
    pub fn serve_substate(
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

    pub fn set_result(&self, result: TransactionQueryResult) {
        self.state.lock().unwrap().result = Some(result);
    }

    /// Serves `checkpoint` as the latest checkpoint of its shard group, in place of the walked chain's latest one.
    pub fn serve_latest_checkpoint(&self, checkpoint: EpochCheckpointEntry) {
        self.state
            .lock()
            .unwrap()
            .shard_group_checkpoints
            .insert(checkpoint.shard_group, checkpoint);
    }

    /// Answers every stealth UTXO stream request with what `answer` returns for the request's shard cursors.
    pub fn answer_utxo_streams<F>(&self, answer: F)
    where F: Fn(&[(Shard, StateVersion)]) -> Vec<UtxoUpdatePayload> + Send + Sync + 'static {
        self.state.lock().unwrap().utxo_stream_answer = Some(Arc::new(answer));
    }

    /// The shard cursors of each stealth UTXO stream request so far.
    pub fn utxo_stream_requests(&self) -> Vec<Vec<(Shard, StateVersion)>> {
        self.state.lock().unwrap().utxo_stream_requests.clone()
    }
}

impl WalletNetworkInterface for ProvingNetwork {
    type Error = ProvingNetworkError;

    async fn query_substate(
        &self,
        address: &SubstateId,
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
        include_proof: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        let mut state = self.state.lock().unwrap();
        state.substate_queries += 1;
        let served = state
            .substates
            .get(address)
            .expect("ProvingNetwork serves no such substate");
        Ok(SubstateQueryResult {
            version: served.version,
            substate: served.value.clone(),
            proof: served.proof.clone().filter(|_| include_proof),
        })
    }

    async fn get_substates(
        &self,
        substate_ids: Vec<SubstateId>,
        include_proofs: bool,
    ) -> Result<FetchedSubstates, Self::Error> {
        let mut state = self.state.lock().unwrap();
        state.substate_fetches += 1;
        let mut fetched = FetchedSubstates::default();
        for id in substate_ids {
            let Some(served) = state.substates.get(&id) else {
                continue;
            };
            fetched
                .substates
                .insert(id.clone(), Substate::new(served.version, served.value.clone()));
            if let Some(proof) = served.proof.clone().filter(|_| include_proofs) {
                fetched.proofs.insert(id, proof);
            }
        }
        Ok(fetched)
    }

    async fn submit_transaction(&self, transaction: Transaction) -> Result<TransactionId, Self::Error> {
        self.state.lock().unwrap().submissions += 1;
        Ok(transaction.calculate_id())
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
        shard_state_versions: Vec<(Shard, StateVersion)>,
        _unspent_only: bool,
    ) -> Result<UtxoUpdateStream<Self::Error>, Self::Error> {
        let mut state = self.state.lock().unwrap();
        let answer = state
            .utxo_stream_answer
            .clone()
            .expect("ProvingNetwork answers no UTXO stream");
        let payloads = answer(&shard_state_versions);
        state.utxo_stream_requests.push(shard_state_versions);
        Ok(futures::stream::iter(payloads.into_iter().map(Ok)).boxed())
    }

    async fn list_watched_substates(
        &self,
        _template_address: Option<TemplateAddress>,
        _limit: Option<u64>,
        _offset: Option<u64>,
    ) -> Result<Vec<WatchedSubstateItem>, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn fetch_utxos(
        &self,
        _resource_address: ResourceAddress,
        _tag_and_nonce_pairs: Vec<TagAndPublicNoncePair>,
    ) -> Result<FetchedUtxos, Self::Error> {
        panic!("ProvingNetwork called")
    }

    async fn get_latest_epoch_checkpoint(&self, shard: Shard) -> Result<EpochCheckpointEntry, Self::Error> {
        let state = self.state.lock().unwrap();
        if state.unreachable {
            return Err(ProvingNetworkError::Unreachable);
        }
        let latest = state.latest.ok_or(ProvingNetworkError::NoCheckpoint)?;
        if let Some(checkpoint) = state
            .shard_group_checkpoints
            .values()
            .find(|checkpoint| checkpoint.shard_group.contains(&shard))
        {
            return Ok(checkpoint.clone());
        }
        Ok(state.checkpoints[latest].clone())
    }

    async fn list_epoch_checkpoints(
        &self,
        from_epoch: Epoch,
        limit: u32,
        _shard: Shard,
    ) -> Result<Vec<EpochCheckpointEntry>, Self::Error> {
        // Lets a concurrent walk run while this request is in flight.
        tokio::task::yield_now().await;
        let mut state = self.state.lock().unwrap();
        if state.unreachable {
            return Err(ProvingNetworkError::Unreachable);
        }
        state.walk_requests += 1;
        let Some(latest) = state.latest.filter(|_| !state.withholds_walk) else {
            return Ok(vec![]);
        };
        Ok(state.checkpoints[..=latest]
            .iter()
            .filter(|checkpoint| checkpoint.epoch >= from_epoch && Some(checkpoint.epoch) != state.unserved)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn get_committee(&self, epoch: Epoch, shard: Shard) -> Result<ValidatorSetCommittee, Self::Error> {
        let mut state = self.state.lock().unwrap();
        if state.unreachable {
            return Err(ProvingNetworkError::Unreachable);
        }
        state.committee_requests += 1;
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

    /// The epoch after the latest checkpoint's, or the first validator set's before any.
    async fn get_consensus_epoch(&self) -> Result<Option<Epoch>, Self::Error> {
        let state = self.state.lock().unwrap();
        if state.unreachable {
            return Err(ProvingNetworkError::Unreachable);
        }
        if !state.observes_consensus_epoch {
            return Ok(None);
        }
        Ok(Some(state.latest.map_or(state.first_epoch, |latest| {
            state.checkpoints[latest].epoch + Epoch(1)
        })))
    }

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProvingNetworkError {
    #[error("No epoch checkpoint found")]
    NoCheckpoint,
    #[error("The indexer cannot be reached")]
    Unreachable,
}

impl IsNotFoundError for ProvingNetworkError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::NoCheckpoint)
    }
}

impl TransactionStatusResponseError for ProvingNetworkError {
    fn get_status(&self) -> ResponseErrorStatus {
        match self {
            Self::NoCheckpoint => ResponseErrorStatus::NotFound {
                message: self.to_string(),
            },
            Self::Unreachable => ResponseErrorStatus::InternalError {
                message: self.to_string(),
            },
        }
    }

    fn get_error_message(&self) -> String {
        self.to_string()
    }
}
