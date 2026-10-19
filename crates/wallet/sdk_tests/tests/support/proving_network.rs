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
use tari_ootle_common_types::{Epoch, StateVersion, SubstateVersion, shard::Shard};
use tari_ootle_light_client::test_fixtures::{CheckpointSpec, ValidatorSetFixture, checkpoint_entry, high, low};
use tari_ootle_transaction::{Transaction, TransactionEnvelope, TransactionId};
use tari_ootle_wallet_sdk::{
    network::{
        FetchedSubstates,
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

use super::PanicError;

/// An indexer that serves a chain of signed end-of-epoch checkpoints, the committees of every epoch's validator set,
/// one transaction result, and the substates it is given to serve.
#[derive(Clone)]
pub struct ProvingNetwork {
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
    /// Requests for a page of checkpoints to walk through.
    walk_requests: usize,
    substates: HashMap<SubstateId, ServedSubstate>,
    /// Requests for a committee.
    committee_requests: usize,
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
                latest,
                committees,
                result: None,
                withholds_walk: false,
                walk_requests: 0,
                substates: HashMap::new(),
                committee_requests: 0,
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
        self.state.lock().unwrap().latest = latest;
    }

    pub fn replace_checkpoint(&self, index: usize, checkpoint: EpochCheckpointEntry) {
        self.state.lock().unwrap().checkpoints[index] = checkpoint;
    }

    pub fn committee_requests(&self) -> usize {
        self.state.lock().unwrap().committee_requests
    }

    pub fn walk_requests(&self) -> usize {
        self.state.lock().unwrap().walk_requests
    }

    pub fn withhold_walk(&self) {
        self.state.lock().unwrap().withholds_walk = true;
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
}

impl WalletNetworkInterface for ProvingNetwork {
    type Error = PanicError;

    async fn query_substate(
        &self,
        address: &SubstateId,
        _version: Option<SubstateVersion>,
        _local_search_only: bool,
        include_proof: bool,
    ) -> Result<SubstateQueryResult, Self::Error> {
        let state = self.state.lock().unwrap();
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
        let state = self.state.lock().unwrap();
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
        // Lets a concurrent walk run while this request is in flight.
        tokio::task::yield_now().await;
        let mut state = self.state.lock().unwrap();
        state.walk_requests += 1;
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
        let mut state = self.state.lock().unwrap();
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

    async fn wait_until_ready(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}
