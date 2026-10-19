//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::{Arc, Weak},
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use tari_indexer_client::{
    error::IndexerRestClientError,
    rest_api_client::IndexerRestApiClient,
    types::{
        EpochCheckpointEntry,
        GetCommitteeRequest,
        GetLatestEpochCheckpointRequest,
        GetSubstateRequest,
        GetSubstatesRequest,
        ListEpochCheckpointsRequest,
        MAX_SUBSTATES_PER_FETCH,
        ProvenTransactionReceipt,
        SubstateProof,
        SubstateState,
    },
};
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    SubstateVersion,
    engine_types::{
        commit_result::FinalizeResult,
        substate::{Substate, SubstateId, SubstateValue},
    },
    optional::IsNotFoundError,
    shard::Shard,
};
use tari_ootle_light_client::{
    DEFAULT_MAX_WALK_EPOCHS,
    LightClientError,
    TrustedValidatorSet,
    TrustedValidatorSetHash,
    VerifiedCommittee,
    WalkPlan,
    check_result_matches_receipt,
    plan_walk,
};
use tari_ootle_transaction::TransactionId;
use tracing::{info, warn};

use crate::Network;

const LOG_TARGET: &str = "ootle_rs::provider::verifier";

/// The shard whose shard group the provider walks the validator set handover through. Every shard is in some shard
/// group in every epoch.
const WALK_SHARD: Shard = Shard::first();

/// The most checkpoints one request of a walk asks for, which is the most an indexer serves at once.
const CHECKPOINTS_PER_REQUEST: u32 = 100;

/// How long the latest trusted validator set judges the freshness of reads before the provider brings it up to date.
const SYNC_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// The shortest the provider allows that an epoch lasted on average when it judges whether it could have fallen behind
/// by a claimed gap of more than [`DEFAULT_MAX_WALK_EPOCHS`]: three quarters of the 20 minute epoch of ten base layer
/// blocks at the two minute target.
const MIN_EPOCH_DURATION: Duration = Duration::from_secs(15 * 60);

/// How long the walk must fail on every sync before the provider re-anchors in its place, whatever the gap.
const REANCHOR_AFTER_FAILING_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// How many epochs a read's proof may be anchored before the latest trusted validator set's epoch. The indexer serves
/// proofs anchored in the current or the previous epoch.
const MAX_PROOF_AGE_EPOCHS: u64 = 1;

/// The most committees the provider keeps verified at once.
const MAX_CACHED_COMMITTEES: usize = 256;

/// Whether a provider verifies what its indexer serves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IndexerTrust {
    /// Every substate read and every committed transaction is verified against a validator set the provider trusts.
    /// The indexer must run with `verify_substate_proofs` enabled.
    ///
    /// The provider trusts the validator set that the indexer's latest checkpoint names on first use, unless given one
    /// with [`IndexerProvider::restore_validator_set`](crate::provider::IndexerProvider::restore_validator_set), and
    /// walks the signed end-of-epoch handovers from it.
    #[default]
    Verify,
    /// The indexer's answers are taken at its word. For development against an indexer that does not verify proofs.
    Trust,
}

/// A provider's trust in the network's validator sets, saved so that a later provider walks on from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedValidatorSet {
    /// The latest validator set the provider trusts.
    pub validator_set: TrustedValidatorSet,
    /// When the provider last brought the set up to date. An indexer's claim that the provider fell more than
    /// [`DEFAULT_MAX_WALK_EPOCHS`] behind is believed only once this is long enough ago for the gap to have passed.
    pub synced_at: SystemTime,
    /// When the walk from the set started failing on every sync, if it is failing.
    pub walk_failing_since: Option<SystemTime>,
}

/// Verifies the indexer's answers against the validator sets the provider trusts.
pub(crate) struct IndexerVerifier {
    client: Weak<IndexerRestApiClient>,
    network: Network,
    trust: IndexerTrust,
    /// Held while bringing the trusted sets up to date, so that one change to them completes before the next starts.
    state: tokio::sync::Mutex<TrustState>,
}

#[derive(Default)]
struct TrustState {
    latest: Option<TrustedValidatorSet>,
    /// Every set trusted from the anchor on, so that proofs anchored in earlier epochs stay verifiable.
    sets: BTreeMap<Epoch, TrustedValidatorSetHash>,
    /// Committees verified against the set of their epoch. An entry is used only while that set is the one trusted for
    /// its epoch.
    committees: HashMap<(Epoch, ShardGroup), VerifiedCommittee>,
    synced_at: Option<SystemTime>,
    walk_failing_since: Option<SystemTime>,
}

impl TrustState {
    fn trust(&mut self, validator_set: TrustedValidatorSet) {
        self.sets.insert(validator_set.epoch(), validator_set.trusted_hash());
        self.latest = Some(validator_set);
    }

    /// Trusts `validator_set` in place of every set trusted before, and marks it as never synced.
    fn anchor(&mut self, validator_set: TrustedValidatorSet) {
        self.sets.clear();
        self.committees.clear();
        self.synced_at = None;
        self.walk_failing_since = None;
        self.trust(validator_set);
    }

    fn record_synced(&mut self) {
        self.synced_at = Some(SystemTime::now());
        self.walk_failing_since = None;
    }

    fn needs_sync(&self) -> bool {
        self.latest.is_none() ||
            self.synced_at
                .is_none_or(|synced_at| synced_at.elapsed().unwrap_or_default() >= SYNC_INTERVAL)
    }

    /// How long the walk has failed on every sync, counting this failure, which starts the count when it is the first.
    fn walk_failing_for(&mut self) -> Duration {
        self.walk_failing_since
            .get_or_insert_with(SystemTime::now)
            .elapsed()
            .unwrap_or_default()
    }

    /// Whether the provider's clock allows that it fell from `trusted` to `latest_checkpoint` behind: every epoch of
    /// the gap takes at least [`MIN_EPOCH_DURATION`], and the provider last synced long enough ago for all of them.
    fn out_of_sync_long_enough(&self, trusted: &TrustedValidatorSet, latest_checkpoint: &EpochCheckpointEntry) -> bool {
        let Some(synced_at) = self.synced_at else {
            return true;
        };
        let gap = latest_checkpoint
            .epoch
            .as_u64()
            .saturating_sub(trusted.epoch().as_u64()) +
            1;
        let gap_takes_at_least = MIN_EPOCH_DURATION.saturating_mul(u32::try_from(gap).unwrap_or(u32::MAX));
        synced_at.elapsed().unwrap_or_default() >= gap_takes_at_least
    }

    fn cached_committee(&self, epoch: Epoch, shard_group: ShardGroup) -> Option<VerifiedCommittee> {
        let trusted = self.sets.get(&epoch)?;
        self.committees
            .get(&(epoch, shard_group))
            .filter(|committee| committee.validator_set_hash() == trusted.validator_set_hash())
            .cloned()
    }

    fn cache_committee(&mut self, committee: VerifiedCommittee) {
        if self.committees.len() >= MAX_CACHED_COMMITTEES {
            self.committees.clear();
        }
        self.committees
            .insert((committee.epoch(), committee.shard_group()), committee);
    }
}

impl IndexerVerifier {
    pub(crate) fn new(client: Weak<IndexerRestApiClient>, network: Network, trust: IndexerTrust) -> Self {
        Self {
            client,
            network,
            trust,
            state: Default::default(),
        }
    }

    pub(crate) fn trust(&self) -> IndexerTrust {
        self.trust
    }

    pub(crate) fn is_verifying(&self) -> bool {
        self.trust == IndexerTrust::Verify
    }

    fn client(&self) -> Result<Arc<IndexerRestApiClient>, VerifyError> {
        self.client.upgrade().ok_or(VerifyError::ClientDropped)
    }

    /// The provider's trust in the network's validator sets, or `None` before it first anchors.
    pub(crate) async fn saved_validator_set(&self) -> Option<SavedValidatorSet> {
        let state = self.state.lock().await;
        let (validator_set, synced_at) = state.latest.clone().zip(state.synced_at)?;
        Some(SavedValidatorSet {
            validator_set,
            synced_at,
            walk_failing_since: state.walk_failing_since,
        })
    }

    /// Trusts `saved` in place of every set trusted before.
    pub(crate) async fn restore_validator_set(&self, saved: SavedValidatorSet) -> Result<(), VerifyError> {
        let network = saved.validator_set.network();
        if network != self.network {
            return Err(VerifyError::ValidatorSetOfAnotherNetwork {
                network,
                expected: self.network,
            });
        }
        let mut state = self.state.lock().await;
        state.anchor(saved.validator_set);
        state.synced_at = Some(saved.synced_at);
        state.walk_failing_since = saved.walk_failing_since;
        Ok(())
    }

    /// Anchors on the validator set the latest checkpoint names unless the provider already trusts one. A transaction
    /// submitted after this commits in an epoch whose validator set the provider trusts or walks to.
    pub(crate) async fn anchor_if_untrusted(&self) -> Result<(), VerifyError> {
        if !self.is_verifying() {
            return Ok(());
        }
        let mut state = self.state.lock().await;
        if state.latest.is_none() {
            self.sync_locked(&mut state).await?;
        }
        Ok(())
    }

    /// Brings the trusted validator set up to the one the latest checkpoint names. On first use the provider anchors
    /// on that set. Later it walks the handovers to it, however far behind it is.
    ///
    /// When the walk fails because the indexer's checkpoints do not connect to the trusted set, the provider
    /// re-anchors on the set the latest checkpoint names in place of the walk, but only once its own clock vouches for
    /// the need:
    /// - the walk has failed on every sync for [`REANCHOR_AFTER_FAILING_FOR`], or
    /// - the gap is more than [`DEFAULT_MAX_WALK_EPOCHS`], and the provider has been out of sync long enough to fall
    ///   that far behind.
    pub(crate) async fn sync(&self) -> Result<TrustedValidatorSet, VerifyError> {
        let mut state = self.state.lock().await;
        self.sync_locked(&mut state).await
    }

    async fn sync_locked(&self, state: &mut TrustState) -> Result<TrustedValidatorSet, VerifyError> {
        let client = self.client()?;
        let latest_checkpoint = client
            .get_latest_epoch_checkpoint(GetLatestEpochCheckpointRequest {
                shard: Some(WALK_SHARD),
                include_committees: true,
            })
            .await?
            .checkpoint;
        let Some(trusted) = state.latest.clone() else {
            info!(target: LOG_TARGET, "Anchoring on the validator set that the checkpoint of {} names", latest_checkpoint.epoch);
            return self.anchor_on(state, &latest_checkpoint);
        };
        let plan = plan_walk(&trusted, latest_checkpoint.epoch, DEFAULT_MAX_WALK_EPOCHS);
        let synced = match plan {
            WalkPlan::UpToDate => trusted,
            WalkPlan::Walk { .. } | WalkPlan::Reanchor => {
                match self.walk(&client, state, latest_checkpoint.epoch).await {
                    Ok(walked) => walked,
                    Err(err) if err.is_unwalkable_history() => {
                        let failing_for = state.walk_failing_for();
                        let fell_behind_offline = matches!(plan, WalkPlan::Reanchor) &&
                            state.out_of_sync_long_enough(&trusted, &latest_checkpoint);
                        if failing_for < REANCHOR_AFTER_FAILING_FOR && !fell_behind_offline {
                            return Err(err);
                        }
                        warn!(
                            target: LOG_TARGET,
                            "The walk from the trusted validator set of {} to the checkpoint of {} failed ({err}), and has failed for {}s. Re-anchoring on the set it names.",
                            trusted.epoch(),
                            latest_checkpoint.epoch,
                            failing_for.as_secs()
                        );
                        return self.anchor_on(state, &latest_checkpoint);
                    },
                    Err(err) => return Err(err),
                }
            },
        };
        state.record_synced();
        Ok(synced)
    }

    fn anchor_on(
        &self,
        state: &mut TrustState,
        checkpoint: &EpochCheckpointEntry,
    ) -> Result<TrustedValidatorSet, VerifyError> {
        let trusted = TrustedValidatorSet::anchor_on_first_use(self.network, WALK_SHARD, checkpoint)?;
        state.anchor(trusted.clone());
        state.record_synced();
        Ok(trusted)
    }

    /// Walks the handovers from the latest trusted set to the validator set of the epoch after `to_epoch`. Each set
    /// walked to is trusted as soon as it verifies, so a walk that fails keeps the sets verified before the failure.
    async fn walk(
        &self,
        client: &IndexerRestApiClient,
        state: &mut TrustState,
        to_epoch: Epoch,
    ) -> Result<TrustedValidatorSet, VerifyError> {
        let mut trusted = state
            .latest
            .clone()
            .expect("the walk starts from a trusted validator set");
        while trusted.epoch() <= to_epoch {
            let checkpoints = client
                .list_epoch_checkpoints(ListEpochCheckpointsRequest {
                    from_epoch: Some(trusted.epoch()),
                    limit: Some(CHECKPOINTS_PER_REQUEST),
                    shard: Some(WALK_SHARD),
                    include_committees: true,
                })
                .await?
                .checkpoints;
            let from_epoch = trusted.epoch();
            for checkpoint in checkpoints.iter().take_while(|checkpoint| checkpoint.epoch <= to_epoch) {
                trusted = trusted.advance(checkpoint)?;
                state.trust(trusted.clone());
            }
            if trusted.epoch() == from_epoch {
                return Err(VerifyError::CheckpointNotServed { epoch: from_epoch });
            }
        }
        Ok(trusted)
    }

    /// The committee of `shard_group` in `epoch`, verified against the validator set the provider trusts for `epoch`.
    /// Walks forward first if `epoch` is past the latest trusted set.
    ///
    /// Returns `None` when the provider trusts no set for `epoch`: one before its anchor, or one that no checkpoint
    /// names yet.
    async fn verify_committee(
        &self,
        state: &mut TrustState,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Option<VerifiedCommittee>, VerifyError> {
        if state.latest.as_ref().is_none_or(|latest| latest.epoch() < epoch) {
            self.sync_locked(state).await?;
        }
        if let Some(latest) = state.latest.as_ref().filter(|latest| latest.epoch() == epoch) &&
            latest.walk_committee().shard_group() == shard_group
        {
            return Ok(Some(latest.walk_committee().clone()));
        }
        let Some(validator_set) = state.sets.get(&epoch).copied() else {
            return Ok(None);
        };
        if let Some(committee) = state.cached_committee(epoch, shard_group) {
            return Ok(Some(committee));
        }
        let committee = self
            .client()?
            .get_committee(GetCommitteeRequest {
                epoch,
                shard: shard_group.start(),
            })
            .await?
            .committee;
        let committee = validator_set.verify_committee(&committee)?;
        state.cache_committee(committee.clone());
        Ok(Some(committee))
    }

    /// Verifies that `substate_id` at `version` holds `value` at a block recent enough to show its current state: one
    /// of the latest trusted validator set's epoch, or of the epoch before.
    ///
    /// The latest trusted set advances only as far as the checkpoints the indexer serves, so an indexer that withholds
    /// newer checkpoints can still serve state from before them.
    pub(crate) async fn verify_substate(
        &self,
        substate_id: &SubstateId,
        version: SubstateVersion,
        value: &SubstateValue,
        proof: Option<&SubstateProof>,
    ) -> Result<(), VerifyError> {
        let proof = proof.ok_or_else(|| VerifyError::SubstateUnproven {
            substate_id: substate_id.clone(),
        })?;
        let anchor = proof.anchor;
        let committee = {
            let mut state = self.state.lock().await;
            if state.needs_sync() {
                self.sync_locked(&mut state).await?;
            }
            check_fresh(&state, substate_id, anchor.epoch)?;
            let committee = self
                .verify_committee(&mut state, anchor.epoch, anchor.shard_group)
                .await?
                .ok_or_else(|| VerifyError::SubstateProvenOutsideTrustedSets {
                    substate_id: substate_id.clone(),
                    epoch: anchor.epoch,
                })?;
            check_fresh(&state, substate_id, anchor.epoch)?;
            committee
        };
        committee
            .verify_substate(substate_id, version, Some(value), proof)
            .map_err(|source| VerifyError::SubstateUnverified {
                substate_id: substate_id.clone(),
                source,
            })?;
        Ok(())
    }

    /// Verifies that the network committed `result` as the result of `transaction_id`: the indexer's proof of the
    /// committed receipt verifies against a validator set the provider trusts, and the result is the one that receipt
    /// commits. Returns `false` while there is no proof to verify yet, and fails for a proof anchored before the
    /// provider's anchor, which no later sync makes verifiable.
    pub(crate) async fn verify_committed_result(
        &self,
        transaction_id: TransactionId,
        result: &FinalizeResult,
        proven: Option<&ProvenTransactionReceipt>,
    ) -> Result<bool, VerifyError> {
        let Some((proven, proof)) = proven.and_then(|proven| proven.proof.as_ref().map(|proof| (proven, proof))) else {
            info!(target: LOG_TARGET, "Transaction {transaction_id} committed, but the indexer served no proof of its receipt yet");
            return Ok(false);
        };
        let committee = {
            let mut state = self.state.lock().await;
            let committee = self
                .verify_committee(&mut state, proof.anchor.epoch, proof.anchor.shard_group)
                .await?;
            if committee.is_none() &&
                let Some(&anchor_epoch) = state.sets.keys().next() &&
                proof.anchor.epoch < anchor_epoch
            {
                return Err(VerifyError::ReceiptProvenBeforeAnchor {
                    transaction_id,
                    epoch: proof.anchor.epoch,
                    anchor_epoch,
                });
            }
            committee
        };
        let Some(committee) = committee else {
            info!(
                target: LOG_TARGET,
                "The receipt of transaction {transaction_id} is proven in {}, for which the provider trusts no validator set yet",
                proof.anchor.epoch
            );
            return Ok(false);
        };
        let unverified = |source| VerifyError::FinalizationUnverified { transaction_id, source };
        committee.verify_receipt(transaction_id, proven).map_err(unverified)?;
        check_result_matches_receipt(committee.network(), transaction_id, &proven.receipt, result)
            .map_err(unverified)?;
        Ok(true)
    }

    /// `substate_id`'s live substate, verified when verifying. A not-found error if the indexer has none.
    pub(crate) async fn fetch_substate(&self, substate_id: &SubstateId) -> Result<Substate, VerifyError> {
        let resp = self
            .client()?
            .get_substate(substate_id, GetSubstateRequest {
                include_proof: self.is_verifying(),
                ..Default::default()
            })
            .await?;
        let version = resp.version;
        let SubstateState::Up { substate: value } = resp.state else {
            return Err(VerifyError::IndexerClientError(
                IndexerRestClientError::InvalidResponse {
                    message: format!(
                        "Indexer answered {substate_id} v{version} as down to a request for its live version"
                    ),
                },
            ));
        };
        if self.is_verifying() {
            self.verify_substate(substate_id, version, &value, resp.proof.as_ref())
                .await?;
        }
        Ok(Substate::new(version, *value))
    }

    /// The live substates of `substate_ids` that the indexer has, every one verified when verifying.
    pub(crate) async fn fetch_substates(
        &self,
        substate_ids: &[SubstateId],
    ) -> Result<HashMap<SubstateId, Substate>, VerifyError> {
        let invalid_count = || {
            VerifyError::IndexerClientError(IndexerRestClientError::RequestInvariant {
                details: format!(
                    "a substate fetch takes between 1 and {MAX_SUBSTATES_PER_FETCH} substates, not {}",
                    substate_ids.len()
                ),
            })
        };
        if substate_ids.len() > MAX_SUBSTATES_PER_FETCH {
            return Err(invalid_count());
        }
        let requests = substate_ids.to_vec().try_into().map_err(|_| invalid_count())?;
        let resp = self
            .client()?
            .fetch_substates(GetSubstatesRequest {
                requests,
                cached_only: false,
                include_proofs: self.is_verifying(),
            })
            .await?;
        if self.is_verifying() {
            for (substate_id, substate) in &resp.substates {
                self.verify_substate(
                    substate_id,
                    substate.version(),
                    substate.substate_value(),
                    resp.proofs.get(substate_id),
                )
                .await?;
            }
        }
        Ok(resp.substates)
    }
}

impl fmt::Debug for IndexerVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IndexerVerifier")
            .field("network", &self.network)
            .field("trust", &self.trust)
            .finish_non_exhaustive()
    }
}

/// Fails unless a proof anchored in `anchor_epoch` is recent enough against the latest trusted set to show current
/// state.
fn check_fresh(state: &TrustState, substate_id: &SubstateId, anchor_epoch: Epoch) -> Result<(), VerifyError> {
    let Some(latest) = state.latest.as_ref() else {
        return Ok(());
    };
    if anchor_epoch.as_u64() + MAX_PROOF_AGE_EPOCHS < latest.epoch().as_u64() {
        return Err(VerifyError::SubstateProofStale {
            substate_id: substate_id.clone(),
            anchor_epoch,
            latest_trusted_epoch: latest.epoch(),
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("Indexer client error: {0}")]
    IndexerClientError(#[from] IndexerRestClientError),
    #[error("Indexer client has been dropped")]
    ClientDropped,
    #[error("The indexer's validator set data does not verify: {0}")]
    ValidatorSetUnverified(#[from] LightClientError),
    #[error("The indexer served no checkpoint ending {epoch} to walk the validator set handover through")]
    CheckpointNotServed { epoch: Epoch },
    #[error("The validator set to trust is of the {network} network, but this provider is for {expected}")]
    ValidatorSetOfAnotherNetwork { network: Network, expected: Network },
    #[error(
        "The indexer served {substate_id} without a proof. Use an indexer with verify_substate_proofs enabled, or \
         retry once the indexer can prove it."
    )]
    SubstateUnproven { substate_id: SubstateId },
    #[error("The indexer proved {substate_id} in {epoch}, for which this provider trusts no validator set")]
    SubstateProvenOutsideTrustedSets { substate_id: SubstateId, epoch: Epoch },
    #[error("The indexer's proof of {substate_id} does not verify: {source}")]
    SubstateUnverified {
        substate_id: SubstateId,
        source: LightClientError,
    },
    #[error(
        "The indexer proved {substate_id} at a block of {anchor_epoch}, too long before {latest_trusted_epoch} to \
         show its current state"
    )]
    SubstateProofStale {
        substate_id: SubstateId,
        anchor_epoch: Epoch,
        latest_trusted_epoch: Epoch,
    },
    #[error(
        "The indexer proved that transaction {transaction_id} committed at a block of {epoch}, before {anchor_epoch} \
         whose validator set this provider anchored on"
    )]
    ReceiptProvenBeforeAnchor {
        transaction_id: TransactionId,
        epoch: Epoch,
        anchor_epoch: Epoch,
    },
    #[error("The indexer's proof that transaction {transaction_id} committed does not verify: {source}")]
    FinalizationUnverified {
        transaction_id: TransactionId,
        source: LightClientError,
    },
}

impl VerifyError {
    /// Whether the error shows that the indexer's checkpoints do not walk from the trusted set, as opposed to a failure
    /// to reach the indexer.
    fn is_unwalkable_history(&self) -> bool {
        matches!(self, Self::ValidatorSetUnverified(_) | Self::CheckpointNotServed { .. })
    }
}

impl IsNotFoundError for VerifyError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::IndexerClientError(err) if err.is_not_found_error())
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::{VersionedSubstateId, engine_types::confidential::ClaimedOutputTombstone};
    use tari_ootle_light_client::test_fixtures::{
        CheckpointSpec,
        NETWORK,
        NUM_PRESHARDS,
        Signers,
        ValidatorSetFixture,
        checkpoint_entry,
        high,
        shard_group_of,
        substate_proof,
    };
    use tari_template_lib_types::ClaimedOutputTombstoneAddress;

    use super::*;
    use crate::{
        provider::{IndexerProvider, ProviderError, test_indexer::TestIndexer},
        wallet::NoWallet,
    };

    const VERSION: SubstateVersion = SubstateVersion::new(3);

    fn substate(seed: u8) -> (SubstateId, SubstateValue) {
        (
            SubstateId::ClaimedOutputTombstone(ClaimedOutputTombstoneAddress::from_bytes(&[seed; 32]).unwrap()),
            SubstateValue::ClaimedOutputTombstone(ClaimedOutputTombstone { value: u64::from(seed) }),
        )
    }

    fn tombstone_value(value: &SubstateValue) -> u64 {
        match value {
            SubstateValue::ClaimedOutputTombstone(tombstone) => tombstone.value,
            _ => panic!("not a tombstone"),
        }
    }

    /// The error a read of `id` fails with.
    async fn read_error(provider: &IndexerProvider<NoWallet>, id: &SubstateId) -> VerifyError {
        match provider.fetch_substate(id.clone()).await {
            Ok(_) => panic!("the read of {id} succeeded"),
            Err(ProviderError::Unverified(err)) => err,
            Err(err) => panic!("the read of {id} failed with {err}"),
        }
    }

    /// Validator sets of epochs 1 to 5.
    fn sets() -> Vec<ValidatorSetFixture> {
        (1..=5).map(ValidatorSetFixture::new).collect()
    }

    /// A provider that anchored on the set of epoch 2 and walked to the set of epoch 5.
    async fn walked_provider(
        indexer_trust: IndexerTrust,
    ) -> (Vec<ValidatorSetFixture>, TestIndexer, IndexerProvider<NoWallet>) {
        let sets = sets();
        let indexer = TestIndexer::spawn(&sets, 0).await;
        let provider = indexer.provider(indexer_trust);
        assert_eq!(provider.sync_validator_set().await.unwrap().epoch(), Epoch(2));
        indexer.set_latest(3);
        assert_eq!(provider.sync_validator_set().await.unwrap().epoch(), Epoch(5));
        (sets, indexer, provider)
    }

    /// `n` substates of the high shard group. The provider walks the low one, so a committee of the high one is
    /// fetched.
    fn high_substates(n: usize) -> Vec<(SubstateId, SubstateValue)> {
        let substates = (1..=u8::MAX)
            .map(substate)
            .filter(|(id, _)| {
                shard_group_of(VersionedSubstateId::new(id.clone(), VERSION).to_shard(NUM_PRESHARDS)) == high()
            })
            .take(n)
            .collect::<Vec<_>>();
        assert_eq!(substates.len(), n);
        substates
    }

    #[tokio::test]
    async fn a_substate_proven_in_the_latest_trusted_epoch_is_read() {
        let (sets, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let (id, value) = substate(1);
        let proof = substate_proof(&sets[4], &id, VERSION, &value);
        indexer.serve_substate(id.clone(), VERSION, value.clone(), Some(proof));

        let read = provider.fetch_substate(id.clone()).await.unwrap();
        assert_eq!(read.version(), VERSION);
        assert_eq!(tombstone_value(read.substate_value()), tombstone_value(&value));

        let fetched = provider
            .verifier()
            .fetch_substates(std::slice::from_ref(&id))
            .await
            .unwrap();
        assert_eq!(tombstone_value(fetched[&id].substate_value()), tombstone_value(&value));
    }

    #[tokio::test]
    async fn the_first_read_anchors_on_the_set_the_latest_checkpoint_names() {
        let sets = sets();
        let indexer = TestIndexer::spawn(&sets, 3).await;
        let provider = indexer.provider(IndexerTrust::Verify);
        let (id, value) = substate(1);
        let proof = substate_proof(&sets[4], &id, VERSION, &value);
        indexer.serve_substate(id.clone(), VERSION, value, Some(proof));

        provider.fetch_substate(id).await.unwrap();
        let trusted = provider.saved_validator_set().await.unwrap().validator_set;
        assert_eq!(trusted.epoch(), Epoch(5));
        assert_eq!(trusted.anchor_epoch(), Epoch(5));
    }

    #[tokio::test]
    async fn a_substate_proven_an_epoch_before_the_latest_trusted_set_is_read() {
        let (sets, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let (id, value) = substate(1);
        let proof = substate_proof(&sets[3], &id, VERSION, &value);
        indexer.serve_substate(id.clone(), VERSION, value, Some(proof));

        provider.fetch_substate(id).await.unwrap();
    }

    #[tokio::test]
    async fn a_substate_proven_too_long_before_the_latest_trusted_set_is_refused() {
        let (sets, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let (id, value) = substate(1);
        let proof = substate_proof(&sets[2], &id, VERSION, &value);
        indexer.serve_substate(id.clone(), VERSION, value, Some(proof));

        let err = read_error(&provider, &id).await;
        assert!(
            matches!(err, VerifyError::SubstateProofStale {
                anchor_epoch: Epoch(3),
                latest_trusted_epoch: Epoch(5),
                ..
            }),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_substate_served_without_a_proof_is_refused() {
        let (_, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let (id, value) = substate(1);
        indexer.serve_substate(id.clone(), VERSION, value, None);

        let err = read_error(&provider, &id).await;
        assert!(matches!(err, VerifyError::SubstateUnproven { .. }), "{err}");
    }

    #[tokio::test]
    async fn a_substate_whose_proof_shows_another_value_is_refused() {
        let (sets, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let (id, value) = substate(1);
        let (_, proven_value) = substate(2);
        let proof = substate_proof(&sets[4], &id, VERSION, &proven_value);
        indexer.serve_substate(id.clone(), VERSION, value, Some(proof));

        let err = read_error(&provider, &id).await;
        assert!(matches!(err, VerifyError::SubstateUnverified { .. }), "{err}");
    }

    #[tokio::test]
    async fn a_substate_proven_before_the_anchor_is_refused() {
        let sets = sets();
        // The provider anchors on the set of epoch 4, which the latest checkpoint names.
        let indexer = TestIndexer::spawn(&sets, 2).await;
        let provider = indexer.provider(IndexerTrust::Verify);
        let (id, value) = substate(1);
        let proof = substate_proof(&sets[2], &id, VERSION, &value);
        indexer.serve_substate(id.clone(), VERSION, value, Some(proof));

        let err = read_error(&provider, &id).await;
        assert!(
            matches!(err, VerifyError::SubstateProvenOutsideTrustedSets {
                epoch: Epoch(3),
                ..
            }),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_fetch_with_any_unproven_substate_is_refused() {
        let (sets, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let (proven_id, proven_value) = substate(1);
        let proof = substate_proof(&sets[4], &proven_id, VERSION, &proven_value);
        indexer.serve_substate(proven_id.clone(), VERSION, proven_value, Some(proof));
        let (unproven_id, unproven_value) = substate(2);
        indexer.serve_substate(unproven_id.clone(), VERSION, unproven_value, None);

        let err = provider
            .verifier()
            .fetch_substates(&[proven_id, unproven_id.clone()])
            .await
            .unwrap_err();
        assert!(
            matches!(&err, VerifyError::SubstateUnproven { substate_id } if *substate_id == unproven_id),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_trusting_provider_reads_without_proofs() {
        let sets = sets();
        let indexer = TestIndexer::spawn(&sets, 3).await;
        let provider = indexer.provider(IndexerTrust::Trust);
        let (id, value) = substate(1);
        indexer.serve_substate(id.clone(), VERSION, value.clone(), None);

        let read = provider.fetch_substate(id).await.unwrap();
        assert_eq!(tombstone_value(read.substate_value()), tombstone_value(&value));
        assert!(provider.saved_validator_set().await.is_none());
    }

    #[tokio::test]
    async fn a_missing_substate_is_not_found() {
        let (_, _, provider) = walked_provider(IndexerTrust::Verify).await;
        let (id, _) = substate(1);

        assert!(provider.get_substate(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_committee_is_fetched_once_for_the_reads_it_proves() {
        let (sets, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let substates = high_substates(2);
        for (id, value) in &substates {
            let proof = substate_proof(&sets[4], id, VERSION, value);
            indexer.serve_substate(id.clone(), VERSION, value.clone(), Some(proof));
        }

        let before = indexer.committee_requests();
        for (id, _) in &substates {
            provider.fetch_substate(id.clone()).await.unwrap();
        }
        assert_eq!(indexer.committee_requests() - before, 1);
    }

    /// A checkpoint ending epoch 5 that the set of epoch 5 did not sign, which names the set `other` for epoch 6.
    fn forged_handover(other: &ValidatorSetFixture) -> EpochCheckpointEntry {
        let forged = ValidatorSetFixture::with_signers(NETWORK, 5, Signers::new(0xb0, 4), Signers::new(0xb1, 4));
        checkpoint_entry(CheckpointSpec::handover(&forged, other))
    }

    #[tokio::test]
    async fn a_saved_validator_set_is_walked_on_from() {
        let (_, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let saved = provider.saved_validator_set().await.unwrap();
        let other = ValidatorSetFixture::with_signers(NETWORK, 6, Signers::new(0xa0, 4), Signers::new(0xa1, 4));
        indexer.push_checkpoint(forged_handover(&other));
        indexer.set_latest(4);

        let restarted = indexer.provider(IndexerTrust::Verify);
        restarted.restore_validator_set(saved.clone()).await.unwrap();
        let err = restarted.sync_validator_set().await.unwrap_err();
        assert!(
            matches!(err, ProviderError::Unverified(VerifyError::ValidatorSetUnverified(_))),
            "{err}"
        );
        assert_eq!(
            restarted.saved_validator_set().await.unwrap().validator_set,
            saved.validator_set
        );

        let anchored_anew = indexer.provider(IndexerTrust::Verify);
        let trusted = anchored_anew.sync_validator_set().await.unwrap();
        assert_eq!(trusted.validator_set_hash(), &other.hash());
    }

    #[tokio::test]
    async fn a_validator_set_of_another_network_is_not_trusted() {
        let (_, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let saved = provider.saved_validator_set().await.unwrap();
        let client = tari_indexer_client::connect_rest(indexer.url()).unwrap();
        let other = IndexerProvider::new(client, NoWallet, Network::MainNet);

        let err = other.restore_validator_set(saved).await.unwrap_err();
        assert!(
            matches!(
                err,
                ProviderError::Unverified(VerifyError::ValidatorSetOfAnotherNetwork { .. })
            ),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_walk_that_keeps_failing_for_a_day_reanchors() {
        let (_, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let other = ValidatorSetFixture::with_signers(NETWORK, 6, Signers::new(0xa0, 4), Signers::new(0xa1, 4));
        indexer.push_checkpoint(forged_handover(&other));
        indexer.set_latest(4);

        provider.sync_validator_set().await.unwrap_err();
        assert_eq!(
            provider.saved_validator_set().await.unwrap().validator_set.epoch(),
            Epoch(5)
        );

        provider.verifier().state.lock().await.walk_failing_since =
            Some(SystemTime::now() - REANCHOR_AFTER_FAILING_FOR);
        let trusted = provider.sync_validator_set().await.unwrap();
        assert_eq!(trusted.epoch(), Epoch(6));
        assert_eq!(trusted.anchor_epoch(), Epoch(6));
        assert_eq!(trusted.validator_set_hash(), &other.hash());
    }

    /// A provider restored from the set of epoch 5 as last synced at `synced_at`, whose indexer claims a latest
    /// checkpoint far enough ahead to re-anchor on and serves nothing to walk to it. Returns the set that checkpoint
    /// names.
    async fn restored_with_claimed_gap(synced_at: SystemTime) -> (ValidatorSetFixture, IndexerProvider<NoWallet>) {
        let (_, indexer, provider) = walked_provider(IndexerTrust::Verify).await;
        let mut saved = provider.saved_validator_set().await.unwrap();
        saved.synced_at = synced_at;
        let claimed_epoch = 5 + DEFAULT_MAX_WALK_EPOCHS;
        let claimed =
            ValidatorSetFixture::with_signers(NETWORK, claimed_epoch, Signers::new(0xc0, 4), Signers::new(0xc1, 4));
        let named =
            ValidatorSetFixture::with_signers(NETWORK, claimed_epoch + 1, Signers::new(0xd0, 4), Signers::new(0xd1, 4));
        indexer.push_checkpoint(checkpoint_entry(CheckpointSpec::handover(&claimed, &named)));
        indexer.set_latest(4);

        let restarted = indexer.provider(IndexerTrust::Verify);
        restarted.restore_validator_set(saved).await.unwrap();
        (named, restarted)
    }

    #[tokio::test]
    async fn a_restored_validator_set_synced_recently_is_not_reanchored_on_a_claimed_gap() {
        let (_, restarted) = restored_with_claimed_gap(SystemTime::now()).await;

        restarted.sync_validator_set().await.unwrap_err();
        assert_eq!(
            restarted.saved_validator_set().await.unwrap().validator_set.epoch(),
            Epoch(5)
        );
    }

    #[tokio::test]
    async fn a_restored_validator_set_out_of_sync_for_the_claimed_gap_reanchors() {
        let gap_takes = MIN_EPOCH_DURATION.saturating_mul(u32::try_from(DEFAULT_MAX_WALK_EPOCHS + 1).unwrap());
        let (named, restarted) = restored_with_claimed_gap(SystemTime::now() - gap_takes).await;

        let trusted = restarted.sync_validator_set().await.unwrap();
        assert_eq!(trusted.validator_set_hash(), &named.hash());
        assert_eq!(trusted.anchor_epoch(), trusted.epoch());
    }
}
