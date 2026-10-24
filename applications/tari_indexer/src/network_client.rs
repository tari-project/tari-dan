//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use futures::{StreamExt, stream::FuturesUnordered};
use indexmap::IndexMap;
use log::{info, warn};
use tari_engine_types::hashing::TariHasher32;
use tari_epoch_manager::{EpochManagerError, EpochManagerReader};
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    NumPreshards,
    ShardGroup,
    SubstateAddress,
    ToSubstateAddress,
    VotePower,
    committee::Committee,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_rpc_framework::RpcStatusCode;
use tari_template_lib_types::Hash32;
use tari_validator_node_rpc::{
    ValidatorNodeRpcClientError,
    client::{TransactionResultStatus, ValidatorNodeClientFactory, ValidatorNodeRpcClient},
};

const LOG_TARGET: &str = "tari::indexer::network_client";

/// How many committee-agreed transaction results are remembered, so that a later poll of the same
/// transaction needs one member to repeat the answer instead of a fresh quorum.
const AGREED_RESULTS_CAPACITY: usize = 4096;

#[derive(Debug, Clone)]
pub struct TariNetworkClient<TEpochManager, TClientFactory> {
    epoch_manager: TEpochManager,
    client_provider: TClientFactory,
    num_preshards: NumPreshards,
    /// Finalized answers a quorum has agreed on, most recent last, bounded at
    /// [`AGREED_RESULTS_CAPACITY`].
    agreed_results: Arc<Mutex<IndexMap<TransactionId, ResultAnswer>>>,
}

impl<TAddr, TEpochManager, TClientFactory> TariNetworkClient<TEpochManager, TClientFactory>
where
    TAddr: NodeAddressable + 'static,
    TEpochManager: EpochManagerReader<Addr = TAddr> + 'static,
    TClientFactory: ValidatorNodeClientFactory<TAddr> + 'static,
{
    pub fn new(epoch_manager: TEpochManager, client_provider: TClientFactory, num_preshards: NumPreshards) -> Self {
        Self {
            epoch_manager,
            client_provider,
            num_preshards,
            agreed_results: Arc::new(Mutex::new(IndexMap::with_capacity(AGREED_RESULTS_CAPACITY))),
        }
    }

    /// The current epoch, once the epoch manager has completed its initial scan. The scan is waited
    /// on because until it lands the epoch manager reports zero, and a caller deriving anything
    /// durable from a zero epoch — a retention key, a validity window — writes a value the node
    /// will disagree with seconds later.
    pub async fn current_epoch(&self) -> Result<Epoch, NetworkClientError> {
        self.epoch_manager.wait_for_initial_scanning_to_complete().await?;
        Ok(self.epoch_manager.current_epoch().await?)
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<TransactionId, NetworkClientError> {
        // Ensure initial scanning has completed to ensure an accurate epoch
        self.epoch_manager.wait_for_initial_scanning_to_complete().await?;

        let tx_id = transaction.calculate_id();

        info!(
            target: LOG_TARGET,
            "Submitting transaction {} to the network", tx_id
        );

        let involved = transaction.involved_substate_addresses_iter().collect::<HashSet<_>>();

        let results = self
            .try_with_committee(involved, |mut client| {
                let transaction = transaction.clone();
                async move { client.submit_transaction(transaction).await }
            })
            .await?;

        let success_count = results.values().filter(|r| r.is_ok()).count();

        info!(
            target: LOG_TARGET,
            "Submitted transaction {} succeeded for {}/{} shard groups",
            tx_id,
            success_count,
            results.len()
        );
        if success_count != results.len() {
            warn!(
                target: LOG_TARGET,
                "Transaction {} was not submitted to some shard groups. {}",
                tx_id,
                results
                    .iter()
                    .filter_map(|(shard_group, result)| {
                        if let Err(err) = result {
                            Some(format!("Failed for {}: {}", shard_group, err))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        Ok(tx_id)
    }

    /// The status of a transaction as agreed by its receipt committee, or `None` if the committee
    /// agrees it does not know the transaction.
    ///
    /// An answer settles the query once members holding more than the committee's tolerated faulty
    /// vote power give it. When the members that answer never reach that agreement the transaction
    /// is reported as pending, which every caller already treats as "ask again later". The
    /// exceptions are listed on [`TransactionResultTally::conclude`].
    pub async fn get_finalized_transaction_result(
        &self,
        transaction_id: TransactionId,
    ) -> Result<Option<TransactionResultStatus>, NetworkClientError> {
        let committee = self
            .committee_for_substate(transaction_id.to_substate_address())
            .await?;
        let agreed = self.agreed_result(&transaction_id);

        // Agreement needs at least f + 1 answers, so that many are asked at once. A finalized answer
        // this indexer has already seen agreed cannot change, so one member repeating it is enough.
        // A member that fails or disagrees frees its slot for the next.
        let width = if agreed.is_some() {
            1
        } else {
            committee.len().saturating_sub(1) / 3 + 1
        };
        let mut tally = TransactionResultTally::new(committee.max_failures(), agreed);
        let mut requests = committee.shuffled().map(|member| {
            let mut client = self.client_provider.create_client(&member.address);
            let vote_power = member.vote_power;
            async move {
                let response = client.get_finalized_transaction_result(transaction_id).await.optional();
                (member, vote_power, response)
            }
        });
        let mut in_flight = FuturesUnordered::new();
        in_flight.extend(requests.by_ref().take(width));

        while let Some((member, vote_power, response)) = in_flight.next().await {
            let response = response.and_then(|status| Ok((ResultAnswer::of(status.as_ref())?, status)));
            if let Err(err) = &response {
                warn!(
                    target: LOG_TARGET,
                    "Validator '{member}' gave no usable result for transaction {transaction_id}: {err}"
                );
            }
            if let Some((answer, status)) = tally.observe(vote_power, response) {
                if answer.is_finalized() {
                    self.record_agreed_result(transaction_id, answer);
                }
                return Ok(status);
            }
            in_flight.extend(requests.next());
        }

        tally.conclude(transaction_id, committee.len())
    }

    fn agreed_result(&self, transaction_id: &TransactionId) -> Option<ResultAnswer> {
        self.agreed_results
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(transaction_id)
            .cloned()
    }

    fn record_agreed_result(&self, transaction_id: TransactionId, answer: ResultAnswer) {
        let mut agreed = self.agreed_results.lock().unwrap_or_else(|e| e.into_inner());
        agreed.insert(transaction_id, answer);
        if agreed.len() > AGREED_RESULTS_CAPACITY {
            agreed.shift_remove_index(0);
        }
    }

    async fn committee_for_substate(
        &self,
        substate_address: SubstateAddress,
    ) -> Result<Arc<Committee<TAddr>>, NetworkClientError> {
        let epoch = self.epoch_manager.current_epoch().await?;
        let num_committees = self.epoch_manager.get_num_committees(epoch).await?;
        let shard_group = substate_address.to_shard_group(self.num_preshards, num_committees);
        self.committee_for_shard_group(epoch, shard_group).await
    }

    /// A shard group with no assigned validators is a statement about the network - nothing answers
    /// for that part of the shard space at this epoch - rather than a failure of this indexer, so it
    /// must reach the caller as an unavailable committee and not an internal error.
    async fn committee_for_shard_group(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Arc<Committee<TAddr>>, NetworkClientError> {
        self.epoch_manager
            .get_committee_by_shard_group(epoch, shard_group)
            .await
            .optional()?
            .filter(|committee| !committee.is_empty())
            .ok_or(NetworkClientError::NoCommitteeForShardGroup { epoch, shard_group })
    }

    /// Fetches the committee members for the given shard and calls the given callback with each member until
    /// the callback returns a `Ok` with results for each shard group. If an Ok is returned, the hashmap is guaranteed
    /// to be the same size as the number of unique shard groups queried.
    pub async fn try_with_committee<'a, F, T, TFut, ISubstateAddr>(
        &self,
        substate_addresses: ISubstateAddr,
        mut callback: F,
    ) -> Result<IndexMap<ShardGroup, Result<T, ValidatorNodeRpcClientError>>, NetworkClientError>
    where
        F: FnMut(TClientFactory::Client) -> TFut,
        TFut: Future<Output = Result<T, ValidatorNodeRpcClientError>> + 'a,
        TClientFactory::Client: 'a,
        T: 'static,
        ISubstateAddr: IntoIterator<Item = SubstateAddress>,
    {
        let epoch = self.epoch_manager.current_epoch().await?;
        let num_committees = self.epoch_manager.get_num_committees(epoch).await?;

        info!(
            target: LOG_TARGET,
            "Fetching committee members at epoch {} ({} total committees)",
            epoch,
            num_committees,
        );

        let mut all_members = HashMap::new();
        for substate_address in substate_addresses {
            let shard_group = substate_address.to_shard_group(self.num_preshards, num_committees);

            if all_members.contains_key(&shard_group) {
                continue; // Already processed this shard group
            }

            let committee = self.committee_for_shard_group(epoch, shard_group).await?;
            all_members.insert(shard_group, committee);
        }

        let committee_size = all_members.len();
        if committee_size == 0 {
            return Err(NetworkClientError::NoCommitteeMembers);
        }

        let mut num_succeeded = 0;
        let mut results = IndexMap::with_capacity(committee_size);
        let mut last_error_sg = None;
        for (shard_group, committee) in all_members {
            for member in committee.shuffled() {
                let client = self.client_provider.create_client(&member.address);
                match callback(client).await {
                    Ok(ret) => {
                        num_succeeded += 1;
                        results.insert(shard_group, Ok(ret));
                        break; // Move onto the next shard group
                    },
                    Err(err) => {
                        warn!(
                            target: LOG_TARGET,
                            "Request failed for validator '{}': {}", member, err
                        );
                        last_error_sg = Some(shard_group);
                        results.insert(shard_group, Err(err));
                    },
                }
            }
        }

        if num_succeeded == 0 {
            let mut last_error = None;

            if let Some(sg) = last_error_sg {
                let last = results.swap_remove(&sg).expect("shard group must exist");
                match last {
                    Ok(_) => {},
                    Err(e) => last_error = Some(e),
                }
            }
            return Err(NetworkClientError::AllValidatorsFailed {
                committee_size,
                last_error,
            });
        }

        Ok(results)
    }
}

/// What committee members must agree on for a transaction-result answer to be believed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ResultAnswer {
    NotFound,
    Pending,
    /// A finalized answer, identified by a hash of everything it reports except the answering
    /// member's own execution and finalization timings. Every honest member derives the rest from
    /// the same deterministic execution.
    Finalized {
        is_commit: bool,
        hash: Hash32,
    },
}

impl ResultAnswer {
    fn of(status: Option<&TransactionResultStatus>) -> Result<Self, ValidatorNodeRpcClientError> {
        let finalized = match status {
            None => return Ok(Self::NotFound),
            Some(TransactionResultStatus::Pending) => return Ok(Self::Pending),
            Some(TransactionResultStatus::Finalized(finalized)) => finalized,
        };

        // A member derives its decision from its execution result, so an answer where the two
        // disagree cannot have come from an honest member.
        let execute_result = finalized
            .execute_result
            .as_ref()
            .map(|r| r.decode())
            .transpose()
            .map_err(|e| {
                ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                    "Node returned an undecodable execution result: {e}"
                ))
            })?;
        let is_commit = finalized.final_decision.is_commit();
        match execute_result {
            Some(r) if r.finalize.result.is_any_accept() != is_commit => {
                return Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                    "Node returned a {} decision contradicting its execution result",
                    if is_commit { "commit" } else { "abort" }
                )));
            },
            None if is_commit => {
                return Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                    "Node returned a commit decision without an execution result"
                )));
            },
            _ => {},
        }

        let mut hasher = TariHasher32::new_with_label("IndexerFinalizedResult");
        hasher.update(&finalized.final_decision);
        hasher.update(&finalized.abort_details);
        if let Some(r) = &execute_result {
            let finalize =
                tari_bor::encode(&r.finalize).map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!(e)))?;
            hasher.update(&finalize);
            hasher.update(&r.execute_epoch.map(|epoch| epoch.as_u64()));
            hasher.update(&r.wasm_execution_points);
            hasher.update(&r.native_execution_points);
        }
        Ok(Self::Finalized {
            is_commit,
            hash: hasher.result(),
        })
    }

    fn is_finalized(&self) -> bool {
        matches!(self, Self::Finalized { .. })
    }

    fn is_abort(&self) -> bool {
        matches!(self, Self::Finalized { is_commit: false, .. })
    }
}

/// Folds committee members' answers to a transaction-result query into an agreed answer.
struct TransactionResultTally {
    /// Faulty vote power the committee tolerates. An answer is believed once members holding more
    /// than this give it, since at least one of them is then honest.
    max_failures: VotePower,
    /// A finalized answer a quorum agreed on in an earlier query, settled by any one member that
    /// repeats it.
    agreed: Option<ResultAnswer>,
    /// Vote power behind each distinct answer, with the first response that gave it.
    answers: HashMap<ResultAnswer, (VotePower, Option<TransactionResultStatus>)>,
    last_error: Option<ValidatorNodeRpcClientError>,
}

type MemberAnswer = (ResultAnswer, Option<TransactionResultStatus>);

impl TransactionResultTally {
    fn new(max_failures: VotePower, agreed: Option<ResultAnswer>) -> Self {
        Self {
            max_failures,
            agreed,
            answers: HashMap::new(),
            last_error: None,
        }
    }

    /// Folds in one member's response, returning the answer if this response settles the query.
    fn observe(
        &mut self,
        vote_power: VotePower,
        response: Result<MemberAnswer, ValidatorNodeRpcClientError>,
    ) -> Option<MemberAnswer> {
        let (answer, status) = match response {
            Ok(answered) => answered,
            Err(err) => {
                self.last_error = Some(err);
                return None;
            },
        };
        if self.agreed.as_ref() == Some(&answer) {
            return Some((answer, status));
        }

        let (power, _) = self
            .answers
            .entry(answer.clone())
            .or_insert_with(|| (VotePower::zero(), status));
        *power = power.saturating_add(vote_power);
        if *power > self.max_failures {
            return self.answers.remove(&answer).map(|(_, status)| (answer, status));
        }
        None
    }

    /// The answer once every member has responded without settling the query:
    /// - When members holding more than the faulty vote power report an abort but disagree on its details, the abort is
    ///   returned without them: the outcome is agreed, the details are not.
    /// - Otherwise any finalized or pending answer reports the transaction as pending.
    /// - Otherwise, when some member does not know the transaction, it is reported as unknown.
    /// - Otherwise every member failed.
    fn conclude(
        mut self,
        transaction_id: TransactionId,
        committee_size: usize,
    ) -> Result<Option<TransactionResultStatus>, NetworkClientError> {
        let abort_power = self
            .answers
            .iter()
            .filter(|(answer, _)| answer.is_abort())
            .fold(VotePower::zero(), |acc, (_, (power, _))| acc.saturating_add(*power));
        if abort_power > self.max_failures {
            let best_attested = self
                .answers
                .iter()
                .filter(|(answer, _)| answer.is_abort())
                .max_by_key(|(_, (power, _))| *power)
                .map(|(answer, _)| answer.clone());
            if let Some((_, Some(TransactionResultStatus::Finalized(mut finalized)))) =
                best_attested.and_then(|answer| self.answers.remove(&answer))
            {
                warn!(
                    target: LOG_TARGET,
                    "Committee agreed transaction {transaction_id} aborted but not on the details. Reporting the abort without them."
                );
                finalized.abort_details = None;
                finalized.execute_result = None;
                return Ok(Some(TransactionResultStatus::Finalized(finalized)));
            }
        }

        if self.answers.keys().any(ResultAnswer::is_finalized) {
            warn!(
                target: LOG_TARGET,
                "Committee did not agree on a finalized result for transaction {transaction_id}. Reporting it as pending."
            );
            return Ok(Some(TransactionResultStatus::Pending));
        }
        if self.answers.contains_key(&ResultAnswer::Pending) {
            return Ok(Some(TransactionResultStatus::Pending));
        }
        if self.answers.contains_key(&ResultAnswer::NotFound) {
            return Ok(None);
        }
        Err(NetworkClientError::AllValidatorsFailed {
            committee_size,
            last_error: self.last_error,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkClientError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Rpc call failed for all ({committee_size}) validators: {}", .last_error.display())]
    AllValidatorsFailed {
        committee_size: usize,
        last_error: Option<ValidatorNodeRpcClientError>,
    },
    #[error("No committee at present. Try again later")]
    NoCommitteeMembers,
    #[error("No validators are assigned to {shard_group} at {epoch}")]
    NoCommitteeForShardGroup { epoch: Epoch, shard_group: ShardGroup },
}

impl NetworkClientError {
    /// Returns the rejection details when every involved validator failed and the last failure was a
    /// BAD_REQUEST, i.e. the transaction was explicitly rejected as invalid (e.g. by mempool
    /// validation) rather than failing for transient reasons.
    pub fn validation_rejection_details(&self) -> Option<&str> {
        let NetworkClientError::AllValidatorsFailed {
            last_error: Some(rpc_err),
            ..
        } = self
        else {
            return None;
        };
        let status = rpc_err.status()?;
        if status.as_status_code() == RpcStatusCode::BadRequest {
            Some(status.details())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tari_engine_types::{
        commit_result::{ExecuteResult, RejectReason, TransactionResult},
        encoded::Encoded,
        substate::SubstateId,
    };
    use tari_epoch_manager::EpochManagerEvent;
    use tari_ootle_common_types::{
        SubstateRequirementRef,
        ValidatorSetHash,
        committee::{CommitteeInfo, CommitteeMember},
    };
    use tari_ootle_storage::{global::models::ValidatorNode, time::PrimitiveDateTime};
    use tari_sidechain::FixedHash;
    use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
    use tari_validator_node_rpc::client::{FinalizedResult, SubstateBatch, SubstateProofData, SubstateResult};
    use tokio::sync::broadcast;

    use super::*;

    type Addr = String;

    #[derive(Clone)]
    enum Answer {
        Pending,
        Finalized(Box<FinalizedResult>),
        Unreachable,
    }

    /// Answers for each committee member, by address, and how many requests they have served.
    #[derive(Clone)]
    struct FakeCommittee(Arc<HashMap<Addr, Answer>>, Arc<AtomicUsize>);

    struct FakeMember {
        committee: FakeCommittee,
        address: Addr,
    }

    impl ValidatorNodeClientFactory<Addr> for FakeCommittee {
        type Client = FakeMember;

        fn create_client(&self, address: &Addr) -> Self::Client {
            FakeMember {
                committee: self.clone(),
                address: address.clone(),
            }
        }
    }

    impl ValidatorNodeRpcClient<Addr> for FakeMember {
        async fn submit_transaction(&mut self, _: Transaction) -> Result<TransactionId, ValidatorNodeRpcClientError> {
            unimplemented!()
        }

        async fn get_finalized_transaction_result(
            &mut self,
            _: TransactionId,
        ) -> Result<TransactionResultStatus, ValidatorNodeRpcClientError> {
            self.committee.1.fetch_add(1, Ordering::SeqCst);
            match &self.committee.0[&self.address] {
                Answer::Pending => Ok(TransactionResultStatus::Pending),
                Answer::Finalized(result) => Ok(TransactionResultStatus::Finalized(result.clone())),
                Answer::Unreachable => Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow::anyhow!(
                    "unreachable"
                ))),
            }
        }

        async fn get_substate(
            &mut self,
            _: SubstateRequirementRef<'_>,
        ) -> Result<SubstateResult, ValidatorNodeRpcClientError> {
            unimplemented!()
        }

        async fn get_substate_with_proof(
            &mut self,
            _: SubstateRequirementRef<'_>,
        ) -> Result<(SubstateResult, Option<SubstateProofData>), ValidatorNodeRpcClientError> {
            unimplemented!()
        }

        async fn get_substates_batch(
            &mut self,
            _: &[&SubstateId],
            _: bool,
        ) -> Result<SubstateBatch, ValidatorNodeRpcClientError> {
            unimplemented!()
        }
    }

    struct FakeEpochManager(Arc<Committee<Addr>>);

    impl EpochManagerReader for FakeEpochManager {
        type Addr = Addr;

        fn subscribe(&self) -> broadcast::Receiver<EpochManagerEvent> {
            unimplemented!()
        }

        async fn wait_for_initial_scanning_to_complete(&self) -> Result<(), EpochManagerError> {
            Ok(())
        }

        async fn get_all_validator_nodes(&self, _: Epoch) -> Result<Vec<ValidatorNode<Addr>>, EpochManagerError> {
            unimplemented!()
        }

        async fn get_committee_info_by_validator_address(
            &self,
            _: Epoch,
            _: &Addr,
        ) -> Result<CommitteeInfo, EpochManagerError> {
            unimplemented!()
        }

        async fn get_committee_for_substate(
            &self,
            _: Epoch,
            _: SubstateAddress,
        ) -> Result<Arc<Committee<Addr>>, EpochManagerError> {
            Ok(self.0.clone())
        }

        async fn get_validator_node_by_public_key(
            &self,
            _: Epoch,
            _: RistrettoPublicKeyBytes,
        ) -> Result<ValidatorNode<Addr>, EpochManagerError> {
            unimplemented!()
        }

        async fn get_our_validator_node(&self, _: Epoch) -> Result<ValidatorNode<Addr>, EpochManagerError> {
            unimplemented!()
        }

        async fn get_local_committee_info(&self, _: Epoch) -> Result<CommitteeInfo, EpochManagerError> {
            unimplemented!()
        }

        async fn get_committee_info(&self, _: Epoch, _: ShardGroup) -> Result<CommitteeInfo, EpochManagerError> {
            unimplemented!()
        }

        async fn get_committee_info_for_substate(
            &self,
            _: Epoch,
            _: SubstateAddress,
        ) -> Result<CommitteeInfo, EpochManagerError> {
            unimplemented!()
        }

        async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
            Ok(Epoch(1))
        }

        async fn get_current_epoch_hash(&self) -> Result<FixedHash, EpochManagerError> {
            unimplemented!()
        }

        async fn get_epoch_hash(&self, _: Epoch) -> Result<FixedHash, EpochManagerError> {
            unimplemented!()
        }

        async fn get_validator_set_hash(&self, _: Epoch) -> Result<ValidatorSetHash, EpochManagerError> {
            unimplemented!()
        }

        async fn get_validator_set_committee_proof(
            &self,
            _: Epoch,
            _: ShardGroup,
        ) -> Result<tari_state_tree::ValidatorSetCommitteeProof, EpochManagerError> {
            unimplemented!()
        }

        async fn get_num_committees(&self, _: Epoch) -> Result<u32, EpochManagerError> {
            Ok(1)
        }

        async fn get_committee_by_shard_group(
            &self,
            _: Epoch,
            _: ShardGroup,
        ) -> Result<Arc<Committee<Addr>>, EpochManagerError> {
            Ok(self.0.clone())
        }

        async fn get_committees_overlapping_shard_group(
            &self,
            _: Epoch,
            _: ShardGroup,
        ) -> Result<HashMap<ShardGroup, Committee<Addr>>, EpochManagerError> {
            unimplemented!()
        }

        async fn get_random_committee_member(
            &self,
            _: Epoch,
            _: Option<ShardGroup>,
            _: HashSet<Addr>,
        ) -> Result<ValidatorNode<Addr>, EpochManagerError> {
            unimplemented!()
        }

        async fn lock_epoch(&self, _: Epoch) -> Result<(), EpochManagerError> {
            unimplemented!()
        }

        async fn get_observed_epoch_hash(&self, _: Epoch) -> Result<Option<FixedHash>, EpochManagerError> {
            unimplemented!()
        }

        async fn get_birthday_epoch(&self) -> Result<Option<Epoch>, EpochManagerError> {
            unimplemented!()
        }
    }

    fn transaction_id() -> TransactionId {
        TransactionId::from([7u8; 32])
    }

    fn finalized(result: TransactionResult, total_fees_required: u64) -> Answer {
        let mut execute_result = ExecuteResult::new_rejected(
            transaction_id().as_hash(),
            RejectReason::ForeignPledgeInputConflict,
            Some(Epoch(1)),
        );
        execute_result.finalize.result = result;
        execute_result.finalize.total_fees_required = total_fees_required;
        Answer::Finalized(Box::new(FinalizedResult {
            final_decision: (&execute_result.finalize.result).into(),
            execute_result: Some(Encoded::encode(&execute_result).unwrap()),
            execution_time: Default::default(),
            finalized_time: PrimitiveDateTime::MIN,
            abort_details: None,
        }))
    }

    /// A commit, told apart from other commits of the same transaction by `tag`.
    fn committed(tag: u64) -> Answer {
        finalized(TransactionResult::Accept(Default::default()), tag)
    }

    fn aborted() -> Answer {
        finalized(TransactionResult::Reject(RejectReason::ForeignPledgeInputConflict), 0)
    }

    fn aborted_with(details: &str) -> Answer {
        let Answer::Finalized(mut result) = aborted() else {
            unreachable!()
        };
        result.abort_details = Some(details.to_string());
        Answer::Finalized(result)
    }

    fn abort_details(status: &TransactionResultStatus) -> Option<&str> {
        let TransactionResultStatus::Finalized(result) = status else {
            panic!("expected a finalized result, got {status:?}");
        };
        assert!(result.final_decision.is_abort());
        result.abort_details.as_deref()
    }

    async fn query(answers: Vec<Answer>) -> Result<Option<TransactionResultStatus>, NetworkClientError> {
        network(answers)
            .0
            .get_finalized_transaction_result(transaction_id())
            .await
    }

    /// A committee of equally weighted members, member `n` answering with `answers[n]`, and a count
    /// of the requests it serves.
    fn network(answers: Vec<Answer>) -> (TariNetworkClient<FakeEpochManager, FakeCommittee>, Arc<AtomicUsize>) {
        let members = (0..answers.len())
            .map(|n| CommitteeMember {
                address: format!("vn{n}"),
                public_key: RistrettoPublicKeyBytes::default(),
                vote_power: VotePower::of(1),
            })
            .collect();
        let answers = answers
            .into_iter()
            .enumerate()
            .map(|(n, answer)| (format!("vn{n}"), answer))
            .collect();
        let requests = Arc::new(AtomicUsize::new(0));
        let client = TariNetworkClient::new(
            FakeEpochManager(Arc::new(Committee::new(members))),
            FakeCommittee(Arc::new(answers), requests.clone()),
            NumPreshards::P1,
        );
        (client, requests)
    }

    fn is_pending(status: &TransactionResultStatus) -> bool {
        matches!(status, TransactionResultStatus::Pending)
    }

    /// The tag of a committed answer.
    fn commit_tag(status: &TransactionResultStatus) -> Option<u64> {
        match status {
            TransactionResultStatus::Finalized(result) if result.final_decision.is_commit() => Some(
                result
                    .execute_result
                    .as_ref()?
                    .decode()
                    .unwrap()
                    .finalize
                    .total_fees_required,
            ),
            _ => None,
        }
    }

    // Members are asked in random order, so the tests that mix answers repeat to put each answer
    // in every position.
    const ROUNDS: usize = 20;

    #[tokio::test]
    async fn a_lone_finalized_answer_is_not_believed() {
        for answer in [committed(1), aborted()] {
            let status = query(vec![
                answer,
                Answer::Unreachable,
                Answer::Unreachable,
                Answer::Unreachable,
            ])
            .await
            .unwrap()
            .unwrap();
            assert!(is_pending(&status));
        }
    }

    #[tokio::test]
    async fn a_lone_finalized_answer_among_pending_members_is_pending() {
        for _ in 0..ROUNDS {
            for answer in [committed(1), aborted()] {
                let status = query(vec![answer, Answer::Pending, Answer::Pending, Answer::Pending])
                    .await
                    .unwrap()
                    .unwrap();
                assert!(is_pending(&status));
            }
        }
    }

    #[tokio::test]
    async fn a_dissenting_finalized_answer_is_outvoted() {
        for _ in 0..ROUNDS {
            for dissent in [committed(1), aborted(), Answer::Pending] {
                let status = query(vec![dissent, committed(2), committed(2), committed(2)])
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(commit_tag(&status), Some(2));
            }
        }
    }

    #[tokio::test]
    async fn more_than_f_agreeing_members_settle_the_result() {
        for _ in 0..ROUNDS {
            let status = query(vec![aborted(), aborted(), Answer::Unreachable, Answer::Pending])
                .await
                .unwrap()
                .unwrap();
            let TransactionResultStatus::Finalized(result) = status else {
                panic!("expected the agreed abort, got {status:?}");
            };
            assert!(result.final_decision.is_abort());
        }
    }

    #[tokio::test]
    async fn a_single_member_committee_is_believed() {
        let status = query(vec![committed(3)]).await.unwrap().unwrap();
        assert_eq!(commit_tag(&status), Some(3));
    }

    #[tokio::test]
    async fn a_decision_contradicting_its_own_result_is_discarded() {
        let Answer::Finalized(mut forged) = committed(1) else {
            unreachable!()
        };
        forged.final_decision = (&TransactionResult::Reject(RejectReason::ForeignPledgeInputConflict)).into();
        let forged = Answer::Finalized(forged);
        for _ in 0..ROUNDS {
            let status = query(vec![forged.clone(), forged.clone(), aborted(), Answer::Pending])
                .await
                .unwrap()
                .unwrap();
            assert!(is_pending(&status));
        }
    }

    #[tokio::test]
    async fn every_member_failing_is_an_error() {
        let result = query(vec![Answer::Unreachable, Answer::Unreachable]).await;
        assert!(matches!(result, Err(NetworkClientError::AllValidatorsFailed { .. })));
    }

    #[tokio::test]
    async fn abort_details_given_by_one_member_are_not_returned() {
        for _ in 0..ROUNDS {
            let status = query(vec![
                aborted_with("one member's account"),
                aborted_with("another member's account"),
                Answer::Unreachable,
                Answer::Unreachable,
            ])
            .await
            .unwrap()
            .unwrap();
            assert_eq!(abort_details(&status), None);
            let TransactionResultStatus::Finalized(result) = status else {
                unreachable!()
            };
            assert!(result.execute_result.is_none());
        }
    }

    #[tokio::test]
    async fn agreed_abort_details_are_returned() {
        for _ in 0..ROUNDS {
            let status = query(vec![
                aborted_with("agreed"),
                aborted_with("agreed"),
                aborted_with("one member's account"),
                Answer::Pending,
            ])
            .await
            .unwrap()
            .unwrap();
            assert_eq!(abort_details(&status), Some("agreed"));
        }
    }

    #[tokio::test]
    async fn an_agreed_result_is_confirmed_by_one_member_on_later_polls() {
        // Seven members: f = 2, so the first poll needs three to agree.
        let (client, requests) = network(vec![committed(2); 7]);
        let status = client
            .get_finalized_transaction_result(transaction_id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(commit_tag(&status), Some(2));
        assert_eq!(requests.swap(0, Ordering::SeqCst), 3);

        let status = client
            .get_finalized_transaction_result(transaction_id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(commit_tag(&status), Some(2));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }
}
