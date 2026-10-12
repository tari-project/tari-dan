//  Copyright 2023, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use futures::{FutureExt, StreamExt, TryStreamExt, stream};
use log::*;
use ootle_network::Network;
use tari_common_types::types::FixedHash;
use tari_engine_types::{
    ProtocolVersion,
    substate::{Substate, SubstateId, SubstateValue},
};
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    NumPreshards,
    ShardGroup,
    SubstateAddress,
    SubstateRequirementRef,
    SubstateVersion,
    ToSubstateAddress,
    VotePower,
    committee::Committee,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_storage::{
    DownProofAnchor,
    SubstateProofVerifyError,
    TrustedStateRoot,
    consensus_models::{CommittedBlockProof, VerifiedBlockTip},
    decode_substate_down_proof,
    exclusion_is_shard_bound,
    verify_substate_down_proof_against_roots,
    verify_substate_value_proof_against_root,
};
use tari_validator_node_rpc::client::{
    SubstateBatch,
    SubstateProofData,
    SubstateResult,
    ValidatorNodeClientFactory,
    ValidatorNodeRpcClient,
};

use crate::{
    committee_read::{CommitteeReadTally, MemberResponse, READ_RACE_WIDTH, race_committee},
    error::IndexerError,
    substate_cache::{SubstateCache, SubstateCacheEntry, SubstateCacheEntryRef, caches_nonexistence},
};

const LOG_TARGET: &str = "tari::indexer::scanner";

/// Coarse staleness backstop for everything the cache holds. Correctness rests on invalidation from
/// the transition stream, so this bounds only the cases that stream cannot correct.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(900);

/// Staleness backstop for a cached `DoesNotExist`, held shorter than [`DEFAULT_CACHE_TTL`].
///
/// A creation retracts the entry through the transition stream, so this bounds only the case where
/// that stream does not deliver. It is kept short because it is the one entry whose staleness a
/// caller feels as an absence: a substate it is waiting for stays missing until this expires.
pub const DEFAULT_NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(60);

/// A store of committee-validated shard-group state merkle roots that the read path consults to
/// avoid re-validating a served commit proof's QC chain when its root is already trusted.
///
/// The trust decision is keyed on the 32-byte `state_merkle_root` scoped by `(epoch, shard_group)`:
/// a node cannot produce a substate value proof that verifies against a root a quorum already
/// signed, so reusing such a root is exactly as sound as re-validating the commit proof.
#[async_trait]
pub trait TrustedRootStore: std::fmt::Debug + Send + Sync + 'static {
    /// True if `root` is a recorded, committee-validated state merkle root for `(epoch, shard_group)`.
    async fn is_trusted(&self, epoch: Epoch, shard_group: ShardGroup, root: FixedHash) -> Result<bool, IndexerError>;

    /// Records a newly committee-validated tip so subsequent reads at this root hit the fast path.
    async fn record(&self, tip: VerifiedBlockTip) -> Result<(), IndexerError>;
}

/// How many validated commit-proof tips a manager remembers. Reads cite the latest few blocks of each shard group, so
/// a small memo covers them; when it fills it starts over.
const VALIDATED_TIP_MEMO_SIZE: usize = 1024;

/// The most substates a validator will answer for in one batch request.
const SUBSTATE_BATCH_SIZE: usize = 50;

/// How many batch requests are in flight at once across the shard groups a lookup touches.
const BATCH_FETCH_CONCURRENCY: usize = 4;

/// How many committee members a batch chunk is tried against before it is given up on. Unlike
/// [`READ_RACE_WIDTH`], which bounds concurrent in-flight reads, these are sequential attempts: a
/// batch is large enough that asking several members at once for the same chunk would waste more
/// bandwidth than the latency it saves.
const BATCH_READ_ATTEMPTS: usize = 5;

/// What a batch's proofs establish about the results in it.
#[derive(Debug, Clone, Copy)]
enum BatchTrust {
    /// Proof verification is off, so there was nothing to establish.
    NotRequired,
    /// Every result was proven against the batch's anchor.
    Proven,
    /// The responder had no committed block to anchor against, so it proved nothing.
    Unanchored,
}

/// Outcome of a substate lookup together with whether the value was committee-verified.
#[derive(Debug, Clone)]
pub struct SubstateLookupResult {
    pub result: SubstateResult,
    /// True when the value was proven against a committee-signed state root. False when proof
    /// verification is disabled, the result is `DoesNotExist` (not provable), or no committee member
    /// could supply a proof yet (e.g. nothing has been committed since an epoch change).
    pub verified: bool,
    /// The proof `result` was verified with. Only a committee fetch and
    /// [`CachedSubstateManager::get_substate_with_proof`] supply one: other cache reads leave it `None`.
    pub proof: Option<SubstateProofData>,
}

/// A live substate and the proof it was verified with, if it was.
#[derive(Debug, Clone)]
pub struct ProvenSubstate {
    pub substate: Substate,
    pub proof: Option<SubstateProofData>,
}

/// What [`CachedSubstateManager::get_input_substates`] found.
#[derive(Debug, Clone)]
pub enum InputSubstatesLookup {
    /// Every requirement is up.
    AllUp(HashMap<SubstateId, SubstateLookupResult>),
    /// The first requirement found to be down. The lookup stops there, so the others are unknown.
    Down {
        substate_id: SubstateId,
        version: SubstateVersion,
    },
    /// The first requirement found not to exist. The lookup stops there, so the others are unknown.
    DoesNotExist { substate_id: SubstateId },
}

impl InputSubstatesLookup {
    /// The stopping outcome for `substate_id`, or `None` when `result` is up.
    fn not_up(substate_id: &SubstateId, result: &SubstateResult) -> Option<Self> {
        match result {
            SubstateResult::Up { .. } => None,
            SubstateResult::Down { version } => Some(Self::Down {
                substate_id: substate_id.clone(),
                version: *version,
            }),
            SubstateResult::DoesNotExist => Some(Self::DoesNotExist {
                substate_id: substate_id.clone(),
            }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CachedSubstateManager<TEpochManager, TVnClient, TSubstateCache> {
    network: Network,
    committee_provider: TEpochManager,
    validator_node_client_factory: TVnClient,
    substate_cache: TSubstateCache,
    /// Coarse staleness backstop on every cache entry. Correctness rests on the cache's own
    /// invalidation, so this exists to bound how long a value may be served if the transitions that
    /// would have retracted it never arrive.
    cache_ttl: Duration,
    /// Staleness backstop for a cached `DoesNotExist`. See [`DEFAULT_NEGATIVE_CACHE_TTL`].
    negative_cache_ttl: Duration,
    /// When set, substates fetched from a validator must come with a proof that verifies against the
    /// shard group committee, or they are rejected (fail-closed). The negative `DoesNotExist` case
    /// is not provable and is left to the existing f+1 agreement.
    verify_substate_proofs: bool,
    /// When set, lets a read skip re-validating a served commit proof whose root is already trusted,
    /// and is warmed with newly-validated roots. See [`TrustedRootStore`].
    trusted_root_store: Option<Arc<dyn TrustedRootStore>>,
    /// Whether this manager writes what it fetches into the substate cache. A manager that does not verify proofs
    /// must not write into a cache that a verifying manager shares, or its unverified results would be served as
    /// that manager's.
    cache_writes: bool,
    /// Tips of commit proofs this manager validated, by the block they commit. A block id is the hash of the header
    /// the committee signed, so a later commit proof of the same block yields the same tip. Bounded by
    /// [`VALIDATED_TIP_MEMO_SIZE`].
    validated_tips: Arc<std::sync::Mutex<HashMap<FixedHash, VerifiedBlockTip>>>,
    /// The network's preshard count, from its consensus constants: it maps a substate to its shard, both to route a
    /// read and to check that a proof is anchored to a root whose shard group holds the substate.
    num_preshards: NumPreshards,
    #[cfg(feature = "metrics")]
    metrics: Option<crate::metrics::Metrics>,
}

impl<TEpochManager, TVnClient, TAddr, TSubstateCache> CachedSubstateManager<TEpochManager, TVnClient, TSubstateCache>
where
    TAddr: NodeAddressable,
    TEpochManager: EpochManagerReader<Addr = TAddr>,
    TVnClient: ValidatorNodeClientFactory<TAddr>,
    TSubstateCache: SubstateCache,
{
    pub fn new(
        network: Network,
        num_preshards: NumPreshards,
        committee_provider: TEpochManager,
        validator_node_client_factory: TVnClient,
        substate_cache: TSubstateCache,
    ) -> Self {
        Self {
            network,
            committee_provider,
            validator_node_client_factory,
            substate_cache,
            cache_ttl: DEFAULT_CACHE_TTL,
            negative_cache_ttl: DEFAULT_NEGATIVE_CACHE_TTL,
            verify_substate_proofs: false,
            trusted_root_store: None,
            cache_writes: true,
            validated_tips: Arc::default(),
            num_preshards,
            #[cfg(feature = "metrics")]
            metrics: None,
        }
    }

    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    pub fn with_negative_cache_ttl(mut self, ttl: Duration) -> Self {
        self.negative_cache_ttl = ttl;
        self
    }

    pub fn with_substate_proof_verification(mut self, enabled: bool) -> Self {
        self.verify_substate_proofs = enabled;
        self
    }

    /// Whether what this manager fetches is written into its substate cache (on by default). It still reads the
    /// cache either way.
    pub fn with_cache_writes(mut self, enabled: bool) -> Self {
        self.cache_writes = enabled;
        self
    }

    /// Sets the trusted-root store used to skip commit-proof re-validation on a store hit (and warmed
    /// on a miss). Only meaningful together with [`Self::with_substate_proof_verification`].
    pub fn with_trusted_root_store(mut self, store: Arc<dyn TrustedRootStore>) -> Self {
        self.trusted_root_store = Some(store);
        self
    }

    /// Whether substates served by this manager are verified against the shard group committee.
    pub fn verifies_substates(&self) -> bool {
        self.verify_substate_proofs
    }

    #[cfg(feature = "metrics")]
    pub fn with_metrics(mut self, registry: &mut prometheus_client::registry::Registry) -> Self {
        self.metrics = Some(crate::metrics::Metrics::register(registry));
        self
    }

    /// Attempts to find the latest substate for the given address. If the lowest possible version is known, it can be
    /// provided to reduce effort/time required to scan.
    pub async fn get_substate(
        &self,
        substate_id: &SubstateId,
        specific_version: Option<SubstateVersion>,
    ) -> Result<SubstateLookupResult, IndexerError> {
        debug!(target: LOG_TARGET, "get_substate: {}v{}", substate_id, specific_version.display());
        if let Some(lookup_result) = self.read_fresh_cache_entry(substate_id, specific_version).await? {
            return Ok(lookup_result);
        }
        #[cfg(feature = "metrics")]
        self.metrics.as_ref().inspect(|m| m.inc_cache_misses());

        self.fetch_and_cache_substate(substate_id, specific_version).await
    }

    /// Like [`Self::get_substate`], but a live result comes with the proof it was verified with. A
    /// cached head is served with the proof held for it; one with no proof held is fetched from the
    /// committee again, and cached with its proof.
    ///
    /// The proof is `None` when proof verification is off, for a result that is not live, and when no
    /// committee member could prove the result.
    pub async fn get_substate_with_proof(
        &self,
        substate_id: &SubstateId,
        specific_version: Option<SubstateVersion>,
    ) -> Result<SubstateLookupResult, IndexerError> {
        debug!(target: LOG_TARGET, "get_substate_with_proof: {}v{}", substate_id, specific_version.display());
        if let Some(mut lookup_result) = self.read_fresh_cache_entry(substate_id, specific_version).await? {
            let SubstateResult::Up { substate } = &lookup_result.result else {
                return Ok(lookup_result);
            };
            lookup_result.proof = self.substate_cache.read_proof(substate_id, substate.version()).await?;
            if lookup_result.proof.is_some() {
                return Ok(lookup_result);
            }
        }
        #[cfg(feature = "metrics")]
        self.metrics.as_ref().inspect(|m| m.inc_cache_misses());

        self.fetch_and_cache_substate(substate_id, specific_version).await
    }

    async fn fetch_and_cache_substate(
        &self,
        substate_id: &SubstateId,
        specific_version: Option<SubstateVersion>,
    ) -> Result<SubstateLookupResult, IndexerError> {
        // Captured before the fetch so that a transition arriving while it is in flight can veto the
        // write it produces.
        let watermark = self.substate_cache.watermark(substate_id).await?;

        let lookup_result = self
            .fetch_substate_from_committee(substate_id, specific_version)
            .await?;

        if let Some(watermark) = watermark {
            // The cache holds each substate's head version. A live version is always the head, and a
            // lookup that named no version answers with the head; a named version that came back down
            // says only that the head is higher.
            let is_head = match &lookup_result.result {
                SubstateResult::Up { .. } => true,
                SubstateResult::Down { .. } => specific_version.is_none(),
                // Having no live version is as much a statement about the head as naming one, but
                // only for the substates whose nonexistence the stream is able to retract.
                SubstateResult::DoesNotExist => specific_version.is_none() && caches_nonexistence(substate_id),
            };
            // Unverified results are not cached while verification is on, so the next read retries
            // for a proven copy instead of pinning the unverified value. Nonexistence is exempt: it
            // has no proof to wait for.
            let admissible = lookup_result.verified ||
                !self.verify_substate_proofs ||
                matches!(lookup_result.result, SubstateResult::DoesNotExist);
            if self.cache_writes && is_head && admissible {
                let version = lookup_result.result.version();
                debug!(target: LOG_TARGET, "Updating cached substate {} with version {}", substate_id, version.display());
                let entry = SubstateCacheEntryRef {
                    version,
                    substate_result: &lookup_result.result,
                    cached_at: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs(),
                    verified: lookup_result.verified,
                    proof: lookup_result.proof.as_ref(),
                };
                self.substate_cache.write(substate_id, entry, watermark).await?;
            }
        }

        Ok(lookup_result)
    }

    /// The cached answer for `substate_id` at `specific_version`, or `None` when the cache holds no
    /// entry that may be served: none at all, one that has aged out, or an unverified one while
    /// verification is on.
    async fn read_fresh_cache_entry(
        &self,
        substate_id: &SubstateId,
        specific_version: Option<SubstateVersion>,
    ) -> Result<Option<SubstateLookupResult>, IndexerError> {
        let cache_res = self
            .substate_cache
            .read(substate_id)
            .await?
            .and_then(|entry| entry.answer_at(specific_version));
        if let Some(entry) = cache_res {
            // Absence has nothing to prove against the state tree, so a cached nonexistence is never
            // verified and gating it on a proof would mean never serving one. Its evidence is the
            // f+1 agreement that produced it. Every other entry that is unverified (e.g. written by
            // the batch path or before verification was enabled) is refetched while verification is
            // on, so it can be replaced with a proven copy.
            let is_nonexistence = matches!(entry.substate_result, SubstateResult::DoesNotExist);
            if is_nonexistence || entry.verified || !self.verify_substate_proofs {
                let ttl = if is_nonexistence {
                    self.negative_cache_ttl
                } else {
                    self.cache_ttl
                };
                let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs();
                let age = now.saturating_sub(entry.cached_at);
                if age <= ttl.as_secs() {
                    debug!(target: LOG_TARGET, "Substate cache hit for {} with version {}", substate_id, entry.version.display());
                    #[cfg(feature = "metrics")]
                    self.metrics.as_ref().inspect(|m| m.inc_cache_hits());
                    return Ok(Some(SubstateLookupResult {
                        result: entry.substate_result,
                        verified: entry.verified,
                        proof: None,
                    }));
                }

                debug!(
                    target: LOG_TARGET,
                    "Cached substate {} at v{} has aged out ({}s). Fetching from committee.",
                    substate_id,
                    entry.version.display(),
                    age,
                );
            }
        }
        Ok(None)
    }

    pub async fn get_cached_substates<'a, I: Iterator<Item = &'a SubstateId> + ExactSizeIterator>(
        &self,
        substate_ids: I,
    ) -> Result<HashMap<&'a SubstateId, Option<SubstateCacheEntry>>, IndexerError> {
        let mut results = HashMap::with_capacity(substate_ids.len());
        for substate_id in substate_ids {
            let cache_res = self.substate_cache.read(substate_id).await?;
            results.insert(substate_id, cache_res);
        }
        Ok(results)
    }

    async fn build_vn_committee_map<'a>(
        &self,
        substate_ids: &[&'a SubstateId],
        epoch: Epoch,
        num_committees: u32,
    ) -> Result<HashMap<ShardGroup, (Arc<Committee<TAddr>>, Vec<&'a SubstateId>)>, IndexerError> {
        let mut map = HashMap::<_, (_, Vec<&'a SubstateId>)>::with_capacity(substate_ids.len());
        for &substate_id in substate_ids {
            let shard_group = SubstateAddress::from_substate_id(substate_id, SubstateVersion::ZERO)
                .to_shard_group(self.num_preshards, num_committees);
            if let Some((_, substates_mut)) = map.get_mut(&shard_group) {
                substates_mut.push(substate_id);
                continue;
            }
            let committee = self
                .committee_provider
                .get_committee_by_shard_group(epoch, shard_group)
                .await
                .optional()?
                .filter(|committee| !committee.is_empty())
                .ok_or_else(|| IndexerError::NoCommitteeMembers {
                    details: format!("No validators are assigned to {shard_group} at {epoch}"),
                })?;
            map.insert(shard_group, (committee, vec![substate_id]));
        }
        Ok(map)
    }

    /// Fetches the live version of each substate from the committee. A substate the committee holds down or
    /// absent is left out of the result. Each result carries the proof it was verified with, when one was.
    ///
    /// A batch is answered by one member, which may leave an id out or report it down without being able to prove
    /// either. Every requested id the batch does not settle is therefore confirmed through
    /// [`Self::get_substate`]'s committee agreement, and a confirmation that fails fails the call: leaving the id out
    /// would tell the caller it is not live.
    pub async fn fetch_and_cache_substates(
        &self,
        substate_ids: &[SubstateId],
    ) -> Result<HashMap<SubstateId, ProvenSubstate>, IndexerError> {
        let substate_ids = substate_ids
            .iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let mut heads = self.fetch_and_cache_heads(&substate_ids).await?;
        let mut lookups = Vec::with_capacity(substate_ids.len());
        let mut unsettled = Vec::new();
        for id in substate_ids {
            match heads.remove(id) {
                Some(head)
                    if head.verified ||
                        !self.verify_substate_proofs ||
                        matches!(head.result, SubstateResult::Up { .. }) =>
                {
                    lookups.push((id.clone(), head));
                },
                _ => unsettled.push(id.clone()),
            }
        }
        let confirmed = stream::iter(unsettled)
            .map(|id| async move {
                let lookup = self.get_substate(&id, None).await?;
                Ok::<_, IndexerError>((id, lookup))
            })
            .buffer_unordered(BATCH_FETCH_CONCURRENCY)
            .try_collect::<Vec<_>>()
            .await?;
        lookups.extend(confirmed);

        // A batch answers with the head version; a caller asking for substates by id wants the live ones, and a
        // down head is not one.
        Ok(lookups
            .into_iter()
            .filter_map(|(id, lookup)| {
                let proof = lookup.proof;
                lookup
                    .result
                    .into_up()
                    .map(|substate| (id, ProvenSubstate { substate, proof }))
            })
            .collect())
    }

    /// Looks up the substates a transaction declares as inputs, stopping at the first one that is not
    /// up.
    ///
    /// Consensus aborts a transaction whose declared input is not up, so once one is found there is
    /// nothing left to learn from the rest. Fresh cache entries are consulted first, then the misses
    /// are fetched in batches. A batch omission says only that one member did not answer for the id,
    /// so an omitted input is confirmed through [`Self::get_substate`]'s committee agreement before
    /// the lookup stops at it.
    pub async fn get_input_substates(
        &self,
        requirements: &[SubstateRequirementRef<'_>],
    ) -> Result<InputSubstatesLookup, IndexerError> {
        let mut found = HashMap::with_capacity(requirements.len());
        let mut misses = Vec::new();
        for req in requirements {
            match self.read_fresh_cache_entry(req.substate_id(), req.version()).await? {
                Some(lookup) => {
                    if let Some(not_up) = InputSubstatesLookup::not_up(req.substate_id(), &lookup.result) {
                        return Ok(not_up);
                    }
                    found.insert(req.substate_id().clone(), lookup);
                },
                None => misses.push(*req),
            }
        }
        if misses.is_empty() {
            return Ok(InputSubstatesLookup::AllUp(found));
        }

        let miss_ids = misses.iter().map(|req| req.substate_id()).collect::<Vec<_>>();
        let heads = self.fetch_and_cache_heads(&miss_ids).await?;

        for req in misses {
            // An unproven head that is not up is confirmed with the committee before the lookup
            // stops at it, as an omitted one is.
            let from_batch = heads
                .get(req.substate_id())
                .filter(|head| {
                    head.verified || !self.verify_substate_proofs || matches!(head.result, SubstateResult::Up { .. })
                })
                .and_then(|head| {
                    let entry = SubstateCacheEntry {
                        version: head.result.version(),
                        substate_result: head.result.clone(),
                        cached_at: 0,
                        verified: head.verified,
                    };
                    entry.answer_at(req.version()).map(|entry| SubstateLookupResult {
                        result: entry.substate_result,
                        verified: entry.verified,
                        proof: None,
                    })
                });
            let lookup = match from_batch {
                Some(lookup) => lookup,
                None => self.get_substate(req.substate_id(), req.version()).await?,
            };
            if let Some(not_up) = InputSubstatesLookup::not_up(req.substate_id(), &lookup.result) {
                return Ok(not_up);
            }
            found.insert(req.substate_id().clone(), lookup);
        }
        Ok(InputSubstatesLookup::AllUp(found))
    }

    /// Fetches the head of each substate in batches from its shard group, caching what may be cached.
    /// Ids that no member answered for are absent from the result.
    async fn fetch_and_cache_heads(
        &self,
        substate_ids: &[&SubstateId],
    ) -> Result<HashMap<SubstateId, SubstateLookupResult>, IndexerError> {
        let epoch = self.committee_provider.current_epoch().await?;
        let num_committees = self.committee_provider.get_num_committees(epoch).await?;
        let committee_map = self.build_vn_committee_map(substate_ids, epoch, num_committees).await?;

        // Captured before any fetch so that a transition arriving while one is in flight can veto the
        // write it produces.
        let mut watermarks = HashMap::with_capacity(substate_ids.len());
        for &substate_id in substate_ids {
            if let Some(watermark) = self.substate_cache.watermark(substate_id).await? {
                watermarks.insert(substate_id, watermark);
            }
        }

        let mut fetches = Vec::new();
        for (shard_group, (committee, substate_ids)) in &committee_map {
            debug!(target: LOG_TARGET, "Fetching {} substates from shard group {}", substate_ids.len(), shard_group);
            for chunk in substate_ids.chunks(SUBSTATE_BATCH_SIZE) {
                // Boxed so that each future's `Send` is settled here, where every lifetime is concrete
                // (rust-lang/rust#102211).
                fetches.push(self.race_substate_batch(committee, chunk, *shard_group).boxed());
            }
        }
        let batches = stream::iter(fetches)
            .buffer_unordered(BATCH_FETCH_CONCURRENCY)
            .try_collect::<Vec<_>>()
            .await?;

        let mut results = HashMap::with_capacity(substate_ids.len());
        for (batch, batch_verified) in batches {
            let commit_proof = batch.commit_proof;
            if !batch.missing.is_empty() {
                debug!(
                    target: LOG_TARGET,
                    "{} requested substate(s) are unknown to the member that answered",
                    batch.missing.len(),
                );
            }

            for substate in batch.substates {
                // A batch answers with heads, and the proof of a down version cannot show that no
                // later version is up, so only an up head is proven by its batch.
                let verified = batch_verified && matches!(substate.result, SubstateResult::Up { .. });
                // A proven batch carries an anchor and a value proof for every result in it.
                let proof = commit_proof.clone().zip(substate.value_proof).filter(|_| verified).map(
                    |(commit_proof, substate_value_proof)| SubstateProofData {
                        substate_value_proof,
                        commit_proof,
                        proof_epoch: substate.proof_epoch,
                        // A batch's down proofs are not checked, so none rides along with its verified proof.
                        substate_down_proof: None,
                        destroyed_at_state_version: None,
                    },
                );
                if let Some(watermark) = watermarks.get(&substate.substate_id).copied() {
                    let entry = SubstateCacheEntryRef {
                        version: substate.result.version(),
                        substate_result: &substate.result,
                        cached_at: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs(),
                        verified,
                        proof: proof.as_ref(),
                    };
                    // An unverified entry is not cached while verification is on, so the next read
                    // retries for a proven copy instead of pinning an unproven value.
                    if self.cache_writes && (verified || !self.verify_substate_proofs) {
                        self.substate_cache
                            .write(&substate.substate_id, entry, watermark)
                            .await?;
                    }
                }

                results.insert(substate.substate_id, SubstateLookupResult {
                    result: substate.result,
                    verified,
                    proof,
                });
            }
        }
        Ok(results)
    }

    /// Fetches one chunk of substate ids, preferring a member that can prove it.
    ///
    /// A member that answers without an anchor has nothing committed to prove against (it is behind,
    /// or in the window right after an epoch change). Rather than settle for that answer, its batch is
    /// held and the rest of the committee is asked, the same way [`CommitteeReadTally`] holds an
    /// unproven single-substate result while it keeps racing. The held batch is served only once every
    /// member has failed to prove the chunk, and never cached.
    ///
    /// Returns the batch and whether it was proven.
    async fn race_substate_batch(
        &self,
        committee: &Committee<TAddr>,
        chunk: &[&SubstateId],
        shard_group: ShardGroup,
    ) -> Result<(SubstateBatch, bool), IndexerError> {
        let mut unproven = None;
        for member in committee.shuffled().take(BATCH_READ_ATTEMPTS) {
            let mut client = self.validator_node_client_factory.create_client(&member.address);
            let batch = match client.get_substates_batch(chunk, self.verify_substate_proofs).await {
                Ok(batch) => batch,
                Err(e) => {
                    warn!(target: LOG_TARGET, "⚠️Failed to get substate batch from {}: {}", member.address, e);
                    continue;
                },
            };

            // One anchor covers the whole batch, so the commit proof is validated against the
            // committee at most once per batch and each result costs only its Merkle path.
            match self.verify_substate_batch(&batch).await {
                Ok(BatchTrust::NotRequired) => return Ok((batch, false)),
                Ok(BatchTrust::Proven) => return Ok((batch, true)),
                Ok(BatchTrust::Unanchored) => {
                    debug!(
                        target: LOG_TARGET,
                        "{} has nothing committed to prove this batch against; asking another member",
                        member.address
                    );
                    unproven.get_or_insert(batch);
                },
                // Fail closed: an unverifiable batch disqualifies this member.
                Err(e) => {
                    warn!(target: LOG_TARGET, "⚠️Rejected substate batch from {}: {}", member.address, e);
                },
            }
        }

        match unproven {
            Some(batch) => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️No member of {shard_group} could prove {} substate(s); serving them unproven",
                    chunk.len()
                );
                Ok((batch, false))
            },
            None => Err(IndexerError::ValidatorNodeClientError(format!(
                "No member of {shard_group} answered for {} substate(s)",
                chunk.len()
            ))),
        }
    }

    /// Verifies every value proof in `batch` against the batch's single anchor.
    ///
    /// A batch that carries an anchor must prove every result it returns; one that carries no anchor
    /// proves nothing, which is not misbehaviour and is reported as [`BatchTrust::Unanchored`] for the
    /// caller to weigh against what other members offer.
    async fn verify_substate_batch(&self, batch: &SubstateBatch) -> Result<BatchTrust, IndexerError> {
        if !self.verify_substate_proofs {
            return Ok(BatchTrust::NotRequired);
        }
        let Some(commit_proof) = &batch.commit_proof else {
            return Ok(BatchTrust::Unanchored);
        };

        let trusted_root = self.trusted_root_from_commit_proof(commit_proof).await?;
        // A batch answers with heads, which a down proof cannot settle (it says nothing about later versions), so
        // the down proofs a batch carries are not checked.
        for substate in &batch.substates {
            let Some(value_proof) = &substate.value_proof else {
                return Err(IndexerError::SubstateProofVerificationFailed {
                    details: format!(
                        "{} was returned without a proof in an anchored batch",
                        substate.substate_id
                    ),
                });
            };
            let (version, value) = match &substate.result {
                SubstateResult::Up { substate } => (substate.version(), Some(substate.substate_value())),
                SubstateResult::Down { version } => (*version, None),
                // A batch returns only substates it holds a record of; the ids it holds none for come
                // back as `missing`, which no proof can settle. Anything else here would be cached as
                // verified without a proof having been checked.
                SubstateResult::DoesNotExist => {
                    return Err(IndexerError::SubstateProofVerificationFailed {
                        details: format!("{} was returned as DoesNotExist in a batch", substate.substate_id),
                    });
                },
            };
            verify_substate_value_proof_against_root(
                value_proof,
                &substate.substate_id,
                version,
                value,
                self.network,
                self.num_preshards,
                Epoch(substate.proof_epoch),
                &trusted_root,
            )
            .map_err(|e| IndexerError::SubstateProofVerificationFailed { details: e.to_string() })?;
        }

        Ok(BatchTrust::Proven)
    }

    async fn fetch_substate_from_committee(
        &self,
        substate_id: &SubstateId,
        specific_version: Option<SubstateVersion>,
    ) -> Result<SubstateLookupResult, IndexerError> {
        let requirement = SubstateRequirementRef::new(substate_id, specific_version);
        let lookup_result = self.get_specific_substate_from_committee(requirement).await?;
        debug!(target: LOG_TARGET, "Substate result for {} with version {}: {:?}", substate_id, specific_version.display(), lookup_result);
        Ok(lookup_result)
    }

    /// Returns a specific version. If this is not found an error is returned.
    async fn get_specific_substate_from_committee(
        &self,
        substate_req: SubstateRequirementRef<'_>,
    ) -> Result<SubstateLookupResult, IndexerError> {
        debug!(target: LOG_TARGET, "get_specific_substate_from_committee: {substate_req}");
        let epoch = self.committee_provider.current_epoch().await?;
        // A shard group with no assigned validators reads back as a missing committee rather than an
        // empty one, and both mean the same thing here: nothing answers for this substate at this
        // epoch, which is a temporary state of the network and not an internal failure.
        let Some(committee) = self
            .committee_provider
            .get_committee_for_substate(epoch, substate_req.or_zero_version().to_substate_address())
            .await
            .optional()?
            .filter(|committee| !committee.is_empty())
        else {
            return Err(IndexerError::NoCommitteeMembers {
                details: format!("No committee found for substate {} at epoch {}", substate_req, epoch),
            });
        };

        let tally = CommitteeReadTally::new(committee.len(), self.verify_substate_proofs, substate_req.version());
        race_committee(
            committee
                .shuffled()
                .map(|member| self.request_substate_from_vn(&member.address, substate_req)),
            READ_RACE_WIDTH,
            tally,
            substate_req,
        )
        // Boxed so that the future's `Send` is settled here, where every lifetime is concrete. Left
        // opaque, rustc has to re-prove it from the caller's generic view and gives up with
        // "implementation of `Send` is not general enough" (rust-lang/rust#102211).
        .boxed()
        .await
    }

    /// One committee member's answer to a read, logged.
    async fn request_substate_from_vn(
        &self,
        vn_addr: &TAddr,
        substate_req: SubstateRequirementRef<'_>,
    ) -> MemberResponse {
        debug!(target: LOG_TARGET, "Getting substate {} from vn {}", substate_req, vn_addr);
        let response = self.get_substate_from_vn(vn_addr, substate_req).await;
        match &response {
            Ok((substate_result, proof)) => {
                debug!(target: LOG_TARGET, "Got substate result for {} from vn {} (verified = {}): {:?}", substate_req, vn_addr, proof.is_some(), substate_result);
            },
            Err(e) => {
                warn!(target: LOG_TARGET, "Could not get substate {} from vn {}: {}", substate_req, vn_addr, e);
            },
        }
        response
    }

    /// Gets a substate directly from querying a VN, with the proof it came with if that proof verified
    /// against the committee.
    async fn get_substate_from_vn(
        &self,
        vn_addr: &TAddr,
        substate_requirement: SubstateRequirementRef<'_>,
    ) -> Result<(SubstateResult, Option<SubstateProofData>), IndexerError> {
        // build a client with the VN
        let mut client = self.validator_node_client_factory.create_client(vn_addr);

        if !self.verify_substate_proofs {
            return client
                .get_substate(substate_requirement)
                .await
                .map(|result| (result, None))
                .map_err(|e| IndexerError::ValidatorNodeClientError(e.to_string()));
        }

        let (result, proof) = client
            .get_substate_with_proof(substate_requirement)
            .await
            .map_err(|e| IndexerError::ValidatorNodeClientError(e.to_string()))?;

        // The validator has nothing committed to anchor a proof against yet (e.g. immediately after
        // an epoch change). Return the result unverified and let the caller decide.
        let Some(proof) = proof else {
            return Ok((result, None));
        };

        // Verify up/down results against the committee. An invalid proof disqualifies this
        // validator's response (fail-closed) so the caller tries another member. `DoesNotExist` is
        // not provable and is left to the existing f+1 agreement.
        match &result {
            SubstateResult::Up { substate } => {
                self.verify_substate_proof(
                    substate_requirement.substate_id(),
                    substate.version(),
                    Some(substate.substate_value()),
                    &proof,
                )
                .await?;
            },
            SubstateResult::Down { version } => {
                // Only a down proof shows the version was ever up; without one, the Down is left to f+1
                // agreement. A down proof says nothing about later versions, so it answers only a read for the
                // version it names, and is not checked for any other read.
                let Some(down_proof) = proof.substate_down_proof.as_deref() else {
                    return Ok((result, None));
                };
                if substate_requirement.version() != Some(*version) {
                    return Ok((result, None));
                }
                let proven = self
                    .verify_down_proof(
                        substate_requirement.substate_id(),
                        *version,
                        down_proof,
                        &proof.commit_proof,
                    )
                    .await?;
                if !proven {
                    return Ok((result, None));
                }
            },
            SubstateResult::DoesNotExist => return Ok((result, None)),
        }

        Ok((result, Some(proof)))
    }

    /// Verifies the down proof of `(substate_id, version)` whose exclusion half is anchored at the root of
    /// `down_commit_proof`.
    ///
    /// The proof orders its two roots by the heights of their blocks, which only a commit proof's signatures
    /// authenticate: the trusted-root store vouches for a root, not for the height a header claims for it. Both commit
    /// proofs are therefore validated against their committees, never taken from the store.
    ///
    /// Returns `Ok(false)` when the Down is unproven but nothing shows the member dishonest: a root cannot be
    /// established for want of its committee, or the proof is of a kind that does not prove a Down (see
    /// [`Self::check_down_proof`]). An invalid proof is an error, which disqualifies the member that served it.
    async fn verify_down_proof(
        &self,
        substate_id: &SubstateId,
        version: SubstateVersion,
        down_proof: &[u8],
        down_commit_proof: &[u8],
    ) -> Result<bool, IndexerError> {
        // An exclusion against a root committed before V2 cannot prove a Down, so such a proof is unproven whatever
        // its commit proofs hold, and validating them would be wasted work. A member that names an earlier epoch than
        // its anchor's only makes its own Down unproven; naming a later one fails validation below.
        let down_epoch = decode_commit_proof(down_commit_proof)?.epoch();
        if !exclusion_is_shard_bound(ProtocolVersion::at(self.network, down_epoch)) {
            return Ok(false);
        }
        let Some(down_root) = self.establish_down_proof_root(down_commit_proof, substate_id).await? else {
            return Ok(false);
        };
        self.verify_down_proof_against(substate_id, version, down_proof, &down_root)
            .await
    }

    /// [`Self::verify_down_proof`] against an already validated `down_root`.
    async fn verify_down_proof_against(
        &self,
        substate_id: &SubstateId,
        version: SubstateVersion,
        down_proof: &[u8],
        down_root: &DownProofAnchor,
    ) -> Result<bool, IndexerError> {
        let decoded = decode_substate_down_proof(down_proof)
            .map_err(|e| IndexerError::SubstateProofVerificationFailed { details: e.to_string() })?;
        let Some(up_root) = self
            .establish_down_proof_root(&decoded.up_commit_proof, substate_id)
            .await?
        else {
            return Ok(false);
        };
        self.check_down_proof(substate_id, version, down_proof, &up_root, down_root)
    }

    /// Checks a down proof against its two established roots.
    ///
    /// Two kinds of proof verify yet leave the Down unproven rather than refuted, since an honest node serves them:
    /// - a global substate proved with roots of two shard groups: each group commits the global shard on its own chain,
    ///   and a node that spans a reshard serves exactly such a proof;
    /// - an exclusion root committed before V2, whose leaves do not name their shard, so that any shard's root proves
    ///   the substate absent.
    fn check_down_proof(
        &self,
        substate_id: &SubstateId,
        version: SubstateVersion,
        down_proof: &[u8],
        up_root: &DownProofAnchor,
        down_root: &DownProofAnchor,
    ) -> Result<bool, IndexerError> {
        match verify_substate_down_proof_against_roots(
            down_proof,
            substate_id,
            version,
            self.network,
            self.num_preshards,
            up_root,
            down_root,
        ) {
            Ok(()) => Ok(true),
            Err(
                e @ (SubstateProofVerifyError::DownProofGlobalAcrossShardGroups { .. } |
                SubstateProofVerifyError::DownProofExclusionNotShardBound { .. }),
            ) => {
                debug!(target: LOG_TARGET, "Down of {substate_id}v{version} is unproven: {e}");
                Ok(false)
            },
            Err(e) => Err(IndexerError::SubstateProofVerificationFailed { details: e.to_string() }),
        }
    }

    /// Validates a root a down proof is anchored to. `None` when it cannot be established for want of its committee
    /// (e.g. an epoch the indexer does not know yet, or no longer knows), which leaves the Down unproven; a commit
    /// proof that its committee does not sign is an error, which disqualifies the member that served it.
    async fn establish_down_proof_root(
        &self,
        commit_proof: &[u8],
        substate_id: &SubstateId,
    ) -> Result<Option<DownProofAnchor>, IndexerError> {
        match self.validated_tip_from_commit_proof(commit_proof).await {
            Ok(tip) => Ok(Some(tip.into())),
            Err(e @ IndexerError::SubstateProofVerificationFailed { .. }) => Err(e),
            Err(e) => {
                debug!(
                    target: LOG_TARGET,
                    "Cannot establish a root of the down proof for {substate_id}: {e}. Leaving the Down unproven."
                );
                Ok(None)
            },
        }
    }

    async fn verify_substate_proof(
        &self,
        substate_id: &SubstateId,
        version: SubstateVersion,
        value: Option<&SubstateValue>,
        proof: &SubstateProofData,
    ) -> Result<(), IndexerError> {
        let trusted_root = self.trusted_root_from_commit_proof(&proof.commit_proof).await?;
        verify_substate_value_proof_against_root(
            &proof.substate_value_proof,
            substate_id,
            version,
            value,
            self.network,
            self.num_preshards,
            Epoch(proof.proof_epoch),
            &trusted_root,
        )
        .map_err(|e| IndexerError::SubstateProofVerificationFailed { details: e.to_string() })?;
        Ok(())
    }

    /// Establishes the shard-group state merkle root that value proofs anchored to `commit_proof`
    /// must verify against, with the epoch and shard group of the block that committed it.
    ///
    /// The returned root is trusted because a quorum of the shard group signed the block header
    /// committing it, independently of any value proof that goes on to cite it. That is what makes it
    /// safe to establish once and reuse for a whole batch of value proofs, and to record for later
    /// reads.
    async fn trusted_root_from_commit_proof(&self, commit_proof: &[u8]) -> Result<TrustedStateRoot, IndexerError> {
        let commit_proof_bytes = commit_proof;
        let commit_proof = decode_commit_proof(commit_proof_bytes)?;
        let epoch = commit_proof.epoch();
        let shard_group = commit_proof
            .shard_group()
            .map_err(|e| IndexerError::SubstateProofVerificationFailed { details: e.to_string() })?;
        let root = commit_proof.state_merkle_root();

        // Fast path: if this exact (epoch, shard_group, root) was already committee-validated and
        // recorded in the trusted-root store, skip re-validating the commit proof's QC chain (and the
        // committee lookup). A node cannot forge a value proof that verifies against a root a quorum
        // already signed, so this is as sound as the full path.
        if let Some(store) = &self.trusted_root_store &&
            store.is_trusted(epoch, shard_group, root).await?
        {
            debug!(
                target: LOG_TARGET,
                "trusted-root HIT at epoch {epoch} {shard_group}: skipped commit-proof validation"
            );
            return Ok(TrustedStateRoot {
                epoch,
                shard_group,
                root,
            });
        }

        Ok(self.validated_tip_from_commit_proof(commit_proof_bytes).await?.into())
    }

    /// Validates `commit_proof` against its shard group committee, so that every field of the returned tip, its height
    /// included, is one the committee signed. Records the tip in the trusted-root store.
    async fn validated_tip_from_commit_proof(&self, commit_proof: &[u8]) -> Result<VerifiedBlockTip, IndexerError> {
        let commit_proof = decode_commit_proof(commit_proof)?;
        let block_id = commit_proof.block_id();
        if let Some(tip) = self.remembered_tip(&block_id) {
            return Ok(tip);
        }
        let epoch = commit_proof.epoch();
        let shard_group = commit_proof
            .shard_group()
            .map_err(|e| IndexerError::SubstateProofVerificationFailed { details: e.to_string() })?;
        let committee = self
            .committee_provider
            .get_committee_by_shard_group(epoch, shard_group)
            .await?;

        let verified_tip = commit_proof
            .validate(committee.quorum_threshold(), |pk| {
                Ok(committee.get_power_by_public_key(pk).unwrap_or_else(VotePower::zero))
            })
            .map_err(|e| IndexerError::SubstateProofVerificationFailed { details: e.to_string() })?;
        debug!(
            target: LOG_TARGET,
            "trusted-root MISS at epoch {epoch} {shard_group}: validated commit proof"
        );

        self.remember_tip(block_id, verified_tip);

        // Warm the store so subsequent reads at this tip hit the fast path. A write failure must not
        // fail an otherwise-verified read.
        if let Some(store) = &self.trusted_root_store {
            let trusted = store
                .is_trusted(epoch, shard_group, verified_tip.state_merkle_root)
                .await
                .unwrap_or(false);
            if !trusted && let Err(e) = store.record(verified_tip).await {
                warn!(target: LOG_TARGET, "Failed to record verified root at epoch {epoch} {shard_group}: {e}");
            }
        }

        Ok(verified_tip)
    }

    fn remembered_tip(&self, block_id: &FixedHash) -> Option<VerifiedBlockTip> {
        self.validated_tips
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(block_id)
            .copied()
    }

    fn remember_tip(&self, block_id: FixedHash, tip: VerifiedBlockTip) {
        let mut tips = self
            .validated_tips
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if tips.len() >= VALIDATED_TIP_MEMO_SIZE {
            tips.clear();
        }
        tips.insert(block_id, tip);
    }
}

fn decode_commit_proof(commit_proof: &[u8]) -> Result<CommittedBlockProof, IndexerError> {
    CommittedBlockProof::from_bytes(commit_proof).map_err(|e| IndexerError::SubstateProofVerificationFailed {
        details: format!("undecodable commit proof: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        str::FromStr,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use tari_engine_types::{non_fungible::NonFungibleContainer, substate::SubstateValue};
    use tari_epoch_manager::{EpochManagerError, EpochManagerEvent};
    use tari_ootle_common_types::committee::{CommitteeInfo, CommitteeMember};
    use tari_ootle_storage::global::models::ValidatorNode;
    use tari_ootle_transaction::{Transaction, TransactionId};
    use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
    use tari_validator_node_rpc::{ValidatorNodeRpcClientError, client::TransactionResultStatus};
    use tokio::sync::broadcast;

    use super::*;
    use crate::substate_cache::{FetchWatermark, SubstateCacheError};

    type Addr = String;

    /// A single-member network holding `live` and answering batches for all of it except `omitted_from_batches`,
    /// and with a down head for `down_in_batches`. Every answer is unproven.
    #[derive(Default)]
    struct FakeNetwork {
        live: HashMap<SubstateId, Substate>,
        omitted_from_batches: HashSet<SubstateId>,
        down_in_batches: HashSet<SubstateId>,
        /// Answered on a proven read as down at the given version, with the given proof.
        down_with_proof: HashMap<SubstateId, (SubstateVersion, SubstateProofData)>,
        batch_requests: AtomicUsize,
        single_requests: AtomicUsize,
        /// Single reads of these fail.
        failing_single_reads: HashSet<SubstateId>,
        /// Answered for every batch in place of what `live` would give.
        proven_batch: Option<SubstateBatch>,
        /// Members whose proven reads fail at once, so that a read can only settle on another member's answer.
        failing_members: HashSet<Addr>,
    }

    #[derive(Clone)]
    /// A client of the fake network, acting for the member at `.1`.
    struct FakeClient(Arc<FakeNetwork>, Option<Addr>);

    impl ValidatorNodeClientFactory<Addr> for FakeClient {
        type Client = Self;

        fn create_client(&self, address: &Addr) -> Self::Client {
            FakeClient(self.0.clone(), Some(address.clone()))
        }
    }

    impl ValidatorNodeRpcClient<Addr> for FakeClient {
        async fn submit_transaction(&mut self, _: Transaction) -> Result<TransactionId, ValidatorNodeRpcClientError> {
            unimplemented!()
        }

        async fn get_finalized_transaction_result(
            &mut self,
            _: TransactionId,
        ) -> Result<TransactionResultStatus, ValidatorNodeRpcClientError> {
            unimplemented!()
        }

        async fn get_substate(
            &mut self,
            substate_req: SubstateRequirementRef<'_>,
        ) -> Result<SubstateResult, ValidatorNodeRpcClientError> {
            self.0.single_requests.fetch_add(1, Ordering::Relaxed);
            if self.0.failing_single_reads.contains(substate_req.substate_id()) {
                return Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow::anyhow!(
                    "no answer for {}",
                    substate_req.substate_id()
                )));
            }
            Ok(match self.0.live.get(substate_req.substate_id()) {
                Some(substate) => SubstateResult::Up {
                    substate: Box::new(substate.clone()),
                },
                None => SubstateResult::DoesNotExist,
            })
        }

        async fn get_substate_with_proof(
            &mut self,
            substate_req: SubstateRequirementRef<'_>,
        ) -> Result<(SubstateResult, Option<SubstateProofData>), ValidatorNodeRpcClientError> {
            if self
                .1
                .as_ref()
                .is_some_and(|address| self.0.failing_members.contains(address))
            {
                return Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow::anyhow!(
                    "{} does not answer",
                    substate_req.substate_id()
                )));
            }
            if let Some((version, proof)) = self.0.down_with_proof.get(substate_req.substate_id()) {
                return Ok((SubstateResult::Down { version: *version }, Some(proof.clone())));
            }
            Ok((self.get_substate(substate_req).await?, None))
        }

        async fn get_substates_batch(
            &mut self,
            substate_ids: &[&SubstateId],
            _include_proofs: bool,
        ) -> Result<SubstateBatch, ValidatorNodeRpcClientError> {
            self.0.batch_requests.fetch_add(1, Ordering::Relaxed);
            if let Some(batch) = &self.0.proven_batch {
                return Ok(batch.clone());
            }
            let mut batch = SubstateBatch {
                commit_proof: None,
                substates: vec![],
                missing: vec![],
            };
            for &id in substate_ids {
                match self.0.live.get(id) {
                    Some(substate) if self.0.down_in_batches.contains(id) => {
                        batch.substates.push(tari_validator_node_rpc::client::BatchedSubstate {
                            substate_id: id.clone(),
                            result: SubstateResult::Down {
                                version: substate.version(),
                            },
                            value_proof: None,
                            proof_epoch: 0,
                            substate_down_proof: None,
                        })
                    },
                    Some(substate) if !self.0.omitted_from_batches.contains(id) => {
                        batch.substates.push(tari_validator_node_rpc::client::BatchedSubstate {
                            substate_id: id.clone(),
                            result: SubstateResult::Up {
                                substate: Box::new(substate.clone()),
                            },
                            value_proof: None,
                            proof_epoch: 0,
                            substate_down_proof: None,
                        })
                    },
                    _ => batch.missing.push(id.clone()),
                }
            }
            Ok(batch)
        }
    }

    /// Serves one committee for every shard group, except at `.1`, an epoch whose committees it does not know. `.2`
    /// counts the committee lookups.
    struct FakeEpochManager(Arc<Committee<Addr>>, Option<Epoch>, Arc<AtomicUsize>);

    impl EpochManagerReader for FakeEpochManager {
        type Addr = Addr;

        fn subscribe(&self) -> broadcast::Receiver<EpochManagerEvent> {
            unimplemented!()
        }

        async fn wait_for_initial_scanning_to_complete(&self) -> Result<(), EpochManagerError> {
            unimplemented!()
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

        async fn get_num_committees(&self, _: Epoch) -> Result<u32, EpochManagerError> {
            Ok(1)
        }

        async fn get_committee_by_shard_group(
            &self,
            epoch: Epoch,
            _: ShardGroup,
        ) -> Result<Arc<Committee<Addr>>, EpochManagerError> {
            self.2.fetch_add(1, Ordering::Relaxed);
            if self.1 == Some(epoch) {
                return Err(EpochManagerError::NoEpochFound(epoch));
            }
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

    #[derive(Default)]
    struct FakeCache(
        Mutex<HashMap<SubstateId, SubstateCacheEntry>>,
        Mutex<HashMap<(SubstateId, SubstateVersion), SubstateProofData>>,
    );

    impl SubstateCache for FakeCache {
        async fn watermark(&self, _: &SubstateId) -> Result<Option<FetchWatermark>, SubstateCacheError> {
            Ok(Some(FetchWatermark::new(0)))
        }

        async fn read(&self, id: &SubstateId) -> Result<Option<SubstateCacheEntry>, SubstateCacheError> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }

        async fn read_proof(
            &self,
            id: &SubstateId,
            version: SubstateVersion,
        ) -> Result<Option<SubstateProofData>, SubstateCacheError> {
            Ok(self.1.lock().unwrap().get(&(id.clone(), version)).cloned())
        }

        async fn write(
            &self,
            id: &SubstateId,
            entry: SubstateCacheEntryRef<'_>,
            _: FetchWatermark,
        ) -> Result<(), SubstateCacheError> {
            self.0.lock().unwrap().insert(id.clone(), SubstateCacheEntry {
                version: entry.version,
                substate_result: entry.substate_result.clone(),
                cached_at: entry.cached_at,
                verified: entry.verified,
            });
            if let (Some(version), Some(proof)) = (entry.version, entry.proof) {
                self.1.lock().unwrap().insert((id.clone(), version), proof.clone());
            }
            Ok(())
        }
    }

    fn component_id(n: u8) -> SubstateId {
        SubstateId::from_str(&format!("component_{}", hex(&[n; 32]))).unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn manager(
        network: FakeNetwork,
    ) -> (
        CachedSubstateManager<FakeEpochManager, FakeClient, FakeCache>,
        Arc<FakeNetwork>,
    ) {
        let network = Arc::new(network);
        let committee = Committee::new(vec![CommitteeMember {
            address: "vn".to_string(),
            public_key: Default::default(),
            vote_power: VotePower::of(1),
        }]);
        let manager = CachedSubstateManager::new(
            Network::LocalNet,
            NumPreshards::P256,
            FakeEpochManager(Arc::new(committee), None, Arc::default()),
            FakeClient(network.clone(), None),
            FakeCache::default(),
        );
        (manager, network)
    }

    fn live(ids: &[SubstateId]) -> HashMap<SubstateId, Substate> {
        ids.iter()
            .map(|id| {
                (
                    id.clone(),
                    Substate::new(0, SubstateValue::NonFungible(NonFungibleContainer::no_data())),
                )
            })
            .collect()
    }

    fn requirements(ids: &[SubstateId]) -> Vec<SubstateRequirementRef<'_>> {
        ids.iter().map(|id| SubstateRequirementRef::new(id, None)).collect()
    }

    fn down_proof_data(substate_down_proof: Option<Vec<u8>>) -> SubstateProofData {
        SubstateProofData {
            substate_value_proof: vec![1],
            commit_proof: vec![2],
            proof_epoch: 0,
            substate_down_proof,
            destroyed_at_state_version: Some(4),
        }
    }

    #[tokio::test]
    async fn a_down_without_a_down_proof_is_unproven() {
        let id = component_id(1);
        let (manager, _) = manager(FakeNetwork {
            down_with_proof: [(id.clone(), (SubstateVersion::new(3), down_proof_data(None)))].into(),
            ..Default::default()
        });
        let manager = manager.with_substate_proof_verification(true);
        let (result, proof) = manager
            .get_substate_from_vn(
                &"vn".to_string(),
                SubstateRequirementRef::new(&id, Some(SubstateVersion::new(3))),
            )
            .await
            .unwrap();
        assert!(matches!(result, SubstateResult::Down { .. }));
        assert!(proof.is_none());
    }

    #[tokio::test]
    async fn a_member_serving_an_invalid_down_proof_is_disqualified() {
        let id = component_id(1);
        let (manager, _) = manager(FakeNetwork {
            down_with_proof: [(id.clone(), (SubstateVersion::new(3), down_proof_data(Some(vec![0xff]))))].into(),
            ..Default::default()
        });
        let manager = manager.with_substate_proof_verification(true);
        let result = manager
            .get_substate_from_vn(
                &"vn".to_string(),
                SubstateRequirementRef::new(&id, Some(SubstateVersion::new(3))),
            )
            .await;
        assert!(
            matches!(result, Err(IndexerError::SubstateProofVerificationFailed { .. })),
            "{result:?}"
        );
    }

    #[derive(Debug)]
    struct TrustEverything;

    #[async_trait::async_trait]
    impl TrustedRootStore for TrustEverything {
        async fn is_trusted(&self, _: Epoch, _: ShardGroup, _: FixedHash) -> Result<bool, IndexerError> {
            Ok(true)
        }

        async fn record(&self, _: VerifiedBlockTip) -> Result<(), IndexerError> {
            Ok(())
        }
    }

    /// The tip `commit_proof`'s header describes, taken at its word: these commit proofs are not signed.
    fn unvalidated_tip(commit_proof: &CommittedBlockProof) -> VerifiedBlockTip {
        VerifiedBlockTip {
            epoch: commit_proof.epoch(),
            shard_group: commit_proof.shard_group().unwrap(),
            height: commit_proof.height(),
            block_id: commit_proof.block_id(),
            epoch_hash: commit_proof.epoch_hash(),
            state_merkle_root: commit_proof.state_merkle_root(),
        }
    }

    /// An unsigned commit proof of `root` that claims `height`.
    fn unsigned_commit_proof(shard_group: ShardGroup, height: u64, root: tari_state_tree::TreeHash) -> Vec<u8> {
        CommittedBlockProof::new(tari_sidechain::SidechainBlockCommitProof {
            header: test_header(shard_group, height, root),
            proof_elements: vec![],
        })
        .to_bytes()
    }

    /// A commit proof of `root` at `height`, its block committed by a 3-chain of certificates each signed by `signers`.
    fn signed_commit_proof(
        shard_group: ShardGroup,
        height: u64,
        root: tari_state_tree::TreeHash,
        signers: &[tari_crypto::ristretto::RistrettoSecretKey],
    ) -> Vec<u8> {
        use tari_common_types::types::CompressedPublicKey;
        use tari_consensus_types::ValidatorSchnorrSignature;
        use tari_crypto::{
            keys::{PublicKey as _, SecretKey as _},
            ristretto::{RistrettoPublicKey, RistrettoSecretKey},
            tari_utilities::ByteArray,
        };
        use tari_sidechain::{
            CommitProofElement,
            ProposalVoteMessage,
            QuorumCertificate,
            QuorumDecision,
            ValidatorBlockSignature,
            ValidatorQcSignature,
        };

        let header = test_header(shard_group, height, root);
        let sign = |secret: &RistrettoSecretKey, message: FixedHash| {
            // A nonce unique to the signer and message.
            let mut nonce = [0u8; 64];
            nonce[..32].copy_from_slice(message.as_slice());
            nonce[32..].copy_from_slice(secret.as_bytes());
            let nonce = RistrettoSecretKey::from_uniform_bytes(&nonce).unwrap();
            let signature =
                ValidatorSchnorrSignature::sign_with_nonce_and_message(secret, nonce, message.as_slice()).unwrap();
            ValidatorQcSignature {
                public_key: CompressedPublicKey::from_canonical_bytes(
                    RistrettoPublicKey::from_secret_key(secret).as_bytes(),
                )
                .unwrap(),
                signature: ValidatorBlockSignature::new(
                    CompressedPublicKey::from_canonical_bytes(signature.get_public_nonce().as_bytes()).unwrap(),
                    signature.get_signature().clone(),
                ),
            }
        };
        let certify = |parent_id: FixedHash, header_hash: FixedHash, qc_height: u64| {
            let mut qc = QuorumCertificate {
                header_hash,
                parent_id,
                protocol_version: header.protocol_version,
                epoch: header.epoch,
                height: qc_height,
                signatures: vec![],
                decision: QuorumDecision::Accept,
            };
            let block_id = qc.calculate_justified_block();
            let message = ProposalVoteMessage::new(
                header.protocol_version,
                &block_id,
                QuorumDecision::Accept,
                header.epoch,
                qc_height,
            )
            .calculate_hash();
            qc.signatures = signers.iter().map(|secret| sign(secret, message)).collect();
            qc
        };
        // The proof walks the chain from its tip back to the committed block: each certificate justifies the parent
        // of the one before it, and the last justifies the header's block.
        let qc1 = certify(header.parent_id, header.calculate_hash(), height);
        let qc2 = certify(qc1.calculate_justified_block(), FixedHash::from([0xb2; 32]), height + 1);
        let qc3 = certify(qc2.calculate_justified_block(), FixedHash::from([0xb3; 32]), height + 2);
        CommittedBlockProof::new(tari_sidechain::SidechainBlockCommitProof {
            header,
            proof_elements: vec![
                CommitProofElement::QuorumCertificate(qc3),
                CommitProofElement::QuorumCertificate(qc2),
                CommitProofElement::QuorumCertificate(qc1),
            ],
        })
        .to_bytes()
    }

    fn test_header(
        shard_group: ShardGroup,
        height: u64,
        root: tari_state_tree::TreeHash,
    ) -> tari_sidechain::SidechainBlockHeader {
        tari_sidechain::SidechainBlockHeader {
            network: Network::LocalNet.as_byte(),
            protocol_version: tari_engine_types::ProtocolVersion::at(Network::LocalNet, Epoch(1)).as_u32(),
            parent_id: FixedHash::zero(),
            justify_id: FixedHash::zero(),
            height,
            epoch: 1,
            epoch_hash: FixedHash::zero(),
            shard_group: tari_sidechain::ShardGroup {
                start: shard_group.start().as_u32(),
                end_inclusive: shard_group.end().as_u32(),
            },
            proposed_by: Default::default(),
            state_merkle_root: FixedHash::new(root.into_array()),
            command_merkle_root: FixedHash::zero(),
            transaction_merkle_root: None,
            signature: Default::default(),
            accumulated_data: Default::default(),
            metadata_hash: FixedHash::zero(),
        }
    }

    /// Two roots of one shard group in one epoch: `R_early`, before a substate was created, and `R_late`, while it is
    /// live. Claiming `R_late` is the lower of the two lets them "prove" the live substate down. The trusted-root store
    /// vouches for both roots but not for the heights their headers claim, so the heights must come from validated
    /// commit proofs, which these unsigned ones are not.
    #[tokio::test]
    async fn a_down_proof_with_forged_heights_is_rejected_even_for_trusted_roots() {
        use tari_ootle_common_types::{ToSubstateAddress, VersionedSubstateId};
        use tari_state_tree::{
            SPARSE_MERKLE_PLACEHOLDER_HASH,
            ShardGroupRootTree,
            SpreadPrefixStateTree,
            StateTreePayload,
            SubstateDownProof,
            SubstateTreeChange,
            SubstateValueProof,
            memory_store::MemoryTreeStore,
        };

        let num_preshards = NumPreshards::P256;
        let protocol_version = tari_engine_types::ProtocolVersion::at(Network::LocalNet, Epoch(1));
        let id = component_id(1);
        let live = VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO);
        let shard = live.to_substate_address().to_shard(num_preshards);
        let shard_group = ShardGroup::new(shard, shard);
        let other = VersionedSubstateId::new(component_id(2), SubstateVersion::ZERO);

        let mut store = MemoryTreeStore::<StateTreePayload>::new();
        let up = |id: &VersionedSubstateId, seed: u8| SubstateTreeChange::Up {
            id: id.clone(),
            value_hash: tari_template_lib_types::Hash32::from_array([seed; 32]),
        };
        let early_shard_root = SpreadPrefixStateTree::new(&mut store)
            .put_substate_changes(None, 1, vec![up(&other, 1)])
            .unwrap();
        let late_shard_root = SpreadPrefixStateTree::new(&mut store)
            .put_substate_changes(Some(1), 2, vec![up(&live, 2)])
            .unwrap();
        let group_tree = |root, version| {
            ShardGroupRootTree::build(protocol_version, [
                (
                    tari_ootle_common_types::shard::Shard::global(),
                    SPARSE_MERKLE_PLACEHOLDER_HASH,
                    0,
                ),
                (shard, root, version),
            ])
            .unwrap()
        };
        let early = group_tree(early_shard_root, 1);
        let late = group_tree(late_shard_root, 2);

        let (_, value, late_leaf) = SpreadPrefixStateTree::new(&mut store).get_proof(2, &live).unwrap();
        let (_, _, early_leaf) = SpreadPrefixStateTree::new(&mut store).get_proof(1, &live).unwrap();
        let down_commit_proof = unsigned_commit_proof(shard_group, 10, early.root());
        let down_proof = tari_bor::serde_codec::to_vec(&SubstateDownProof {
            up: SubstateValueProof::new(late_shard_root, 2, late.get_proof(shard).unwrap().1, late_leaf),
            up_value_hash: value.unwrap().0,
            up_commit_proof: unsigned_commit_proof(shard_group, 1, late.root()),
            down: SubstateValueProof::new(early_shard_root, 1, early.get_proof(shard).unwrap().1, early_leaf),
        })
        .unwrap();

        // Taken at their word, the forged anchors make a valid proof.
        let anchor =
            |bytes: &[u8]| DownProofAnchor::from(unvalidated_tip(&CommittedBlockProof::from_bytes(bytes).unwrap()));
        let decoded = decode_substate_down_proof(&down_proof).unwrap();
        verify_substate_down_proof_against_roots(
            &down_proof,
            &id,
            SubstateVersion::ZERO,
            Network::LocalNet,
            num_preshards,
            &anchor(&decoded.up_commit_proof),
            &anchor(&down_commit_proof),
        )
        .unwrap();

        let (manager, _) = manager(FakeNetwork {
            down_with_proof: [(
                id.clone(),
                (SubstateVersion::ZERO, SubstateProofData {
                    substate_value_proof: vec![],
                    commit_proof: down_commit_proof,
                    proof_epoch: 0,
                    substate_down_proof: Some(down_proof),
                    destroyed_at_state_version: Some(1),
                }),
            )]
            .into(),
            ..Default::default()
        });
        let manager = manager
            .with_substate_proof_verification(true)
            .with_trusted_root_store(Arc::new(TrustEverything));
        let result = manager
            .get_substate_from_vn(
                &"vn".to_string(),
                SubstateRequirementRef::new(&id, Some(SubstateVersion::ZERO)),
            )
            .await;
        assert!(
            matches!(result, Err(IndexerError::SubstateProofVerificationFailed { .. })),
            "{result:?}"
        );
    }

    /// A global substate up at `R1` and down at `R2`, with `R1`'s commit proof naming `up_group`.
    fn global_down_proof(up_group: ShardGroup) -> (SubstateId, Vec<u8>, DownProofAnchor, tari_state_tree::TreeHash) {
        global_down_proof_signed_by(up_group, &[])
    }

    /// [`global_down_proof`] with `R1`'s commit proof signed by `signers`, or unsigned if there are none.
    fn global_down_proof_signed_by(
        up_group: ShardGroup,
        signers: &[tari_crypto::ristretto::RistrettoSecretKey],
    ) -> (SubstateId, Vec<u8>, DownProofAnchor, tari_state_tree::TreeHash) {
        use tari_engine_types::published_template::PublishedTemplateAddress;
        use tari_ootle_common_types::{VersionedSubstateId, shard::Shard};
        use tari_state_tree::{
            ShardGroupRootTree,
            SpreadPrefixStateTree,
            StateTreePayload,
            SubstateDownProof,
            SubstateTreeChange,
            SubstateValueProof,
            memory_store::MemoryTreeStore,
        };

        let protocol_version = tari_engine_types::ProtocolVersion::at(Network::LocalNet, Epoch(1));
        let id = SubstateId::Template(PublishedTemplateAddress::from_hash(
            tari_template_lib_types::Hash32::from_array([4; 32]),
        ));
        let target = VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO);
        let mut store = MemoryTreeStore::<StateTreePayload>::new();
        let r1_shard_root = SpreadPrefixStateTree::new(&mut store)
            .put_substate_changes(None, 1, vec![SubstateTreeChange::Up {
                id: target.clone(),
                value_hash: tari_template_lib_types::Hash32::from_array([1; 32]),
            }])
            .unwrap();
        let r2_shard_root = SpreadPrefixStateTree::new(&mut store)
            .put_substate_changes(Some(1), 2, vec![SubstateTreeChange::Down { id: target.clone() }])
            .unwrap();
        let tree =
            |root, version| ShardGroupRootTree::build(protocol_version, [(Shard::global(), root, version)]).unwrap();
        let r1 = tree(r1_shard_root, 1);
        let r2 = tree(r2_shard_root, 2);
        let (_, value, up_leaf) = SpreadPrefixStateTree::new(&mut store).get_proof(1, &target).unwrap();
        let (_, _, down_leaf) = SpreadPrefixStateTree::new(&mut store).get_proof(2, &target).unwrap();
        let up_commit_proof = if signers.is_empty() {
            unsigned_commit_proof(up_group, 2, r1.root())
        } else {
            signed_commit_proof(up_group, 2, r1.root(), signers)
        };
        let up_root = DownProofAnchor::from(unvalidated_tip(
            &CommittedBlockProof::from_bytes(&up_commit_proof).unwrap(),
        ));
        let proof = tari_bor::serde_codec::to_vec(&SubstateDownProof {
            up: SubstateValueProof::new(r1_shard_root, 1, r1.get_proof(Shard::global()).unwrap().1, up_leaf),
            up_value_hash: value.unwrap().0,
            up_commit_proof,
            down: SubstateValueProof::new(r2_shard_root, 2, r2.get_proof(Shard::global()).unwrap().1, down_leaf),
        })
        .unwrap();
        (id, proof, up_root, r2.root())
    }

    fn group(start: u32, end: u32) -> ShardGroup {
        ShardGroup::new(
            tari_ootle_common_types::shard::Shard::from_u32(start),
            tari_ootle_common_types::shard::Shard::from_u32(end),
        )
    }

    #[tokio::test]
    async fn a_global_down_proved_across_shard_groups_is_unproven_not_invalid() {
        let (manager, _) = manager(FakeNetwork::default());
        let manager = manager.with_substate_proof_verification(true);
        for (up_group, expected) in [(group(1, 2), true), (group(3, 4), false)] {
            let (id, proof, up_root, r2_root) = global_down_proof(up_group);
            let proven = manager
                .check_down_proof(&id, SubstateVersion::ZERO, &proof, &up_root, &r2_anchor(r2_root))
                .unwrap();
            assert_eq!(proven, expected, "{up_group}");
        }
    }

    /// The anchor `global_down_proof`'s exclusion half verifies against.
    fn r2_anchor(r2_root: tari_state_tree::TreeHash) -> DownProofAnchor {
        unvalidated_tip(&CommittedBlockProof::from_bytes(&unsigned_commit_proof(group(1, 2), 4, r2_root)).unwrap())
            .into()
    }

    /// Before V2 a shard-group leaf is keyed by its value, so another shard's root proves any substate absent. Such a
    /// proof leaves the Down unproven without disqualifying the member.
    #[tokio::test]
    async fn a_down_proved_against_a_pre_v2_root_is_unproven_not_invalid() {
        let (mut manager, _) = manager(FakeNetwork::default());
        manager.network = Network::Esmeralda;
        assert!(matches!(
            tari_engine_types::ProtocolVersion::at(Network::Esmeralda, PROOF_EPOCH),
            tari_engine_types::ProtocolVersion::V0 | tari_engine_types::ProtocolVersion::V1
        ));
        let manager = manager.with_substate_proof_verification(true);
        let (id, proof, up_root, r2_root) = global_down_proof(group(1, 2));
        let proven = manager
            .check_down_proof(&id, SubstateVersion::ZERO, &proof, &up_root, &r2_anchor(r2_root))
            .unwrap();
        assert!(!proven);
    }

    /// A committee of `keys` (`vn0`, `vn1`, ...) of which `vn0` serves an honest down proof of a global substate,
    /// both of its commit proofs signed by all but the last member, and `failing` fail every proven read. Verification
    /// is on.
    fn signed_down_proof_setup(
        keys: &[(
            tari_crypto::ristretto::RistrettoSecretKey,
            tari_crypto::ristretto::RistrettoPublicKey,
        )],
        failing: HashSet<Addr>,
    ) -> (
        SubstateId,
        CachedSubstateManager<FakeEpochManager, FakeClient, FakeCache>,
    ) {
        use tari_crypto::tari_utilities::ByteArray;

        let committee = Committee::new(
            keys.iter()
                .enumerate()
                .map(|(i, (_, public))| CommitteeMember {
                    address: format!("vn{i}"),
                    public_key: RistrettoPublicKeyBytes::from_bytes(public.as_bytes()).unwrap(),
                    vote_power: VotePower::of(1),
                })
                .collect(),
        );
        let signers = keys[..keys.len() - 1]
            .iter()
            .map(|(secret, _)| secret.clone())
            .collect::<Vec<_>>();

        let (id, proof, _, r2_root) = global_down_proof_signed_by(group(1, 2), &signers);
        let (mut manager, _) = manager(FakeNetwork {
            down_with_proof: [(
                id.clone(),
                (SubstateVersion::ZERO, SubstateProofData {
                    substate_value_proof: vec![],
                    commit_proof: signed_commit_proof(group(1, 2), 4, r2_root, &signers),
                    proof_epoch: 0,
                    substate_down_proof: Some(proof),
                    destroyed_at_state_version: Some(2),
                }),
            )]
            .into(),
            failing_members: failing,
            ..Default::default()
        });
        manager.committee_provider.0 = Arc::new(committee);
        (id, manager.with_substate_proof_verification(true))
    }

    /// One member's valid down proof, with both commit proofs signed by a quorum of the committee, settles a read for
    /// the version it names, verified, while every other member fails. It does not settle a head read.
    #[tokio::test]
    async fn one_members_valid_down_proof_settles_a_read_for_its_version() {
        let keys = (1..=4)
            .map(tari_ootle_common_types::crypto::create_key_pair_from_seed)
            .collect::<Vec<_>>();
        let (id, manager) = signed_down_proof_setup(&keys, ["vn1", "vn2", "vn3"].map(String::from).into());
        let answering = "vn0".to_string();

        let (result, proof) = manager
            .get_substate_from_vn(
                &answering,
                SubstateRequirementRef::new(&id, Some(SubstateVersion::ZERO)),
            )
            .await
            .unwrap();
        assert!(matches!(result, SubstateResult::Down { .. }));
        assert!(proof.is_some(), "a valid down proof answers a read for its version");

        let (_, proof) = manager
            .get_substate_from_vn(&answering, SubstateRequirementRef::new(&id, None))
            .await
            .unwrap();
        assert!(proof.is_none(), "a down proof does not answer a head read");

        let lookup = tokio::time::timeout(
            Duration::from_secs(10),
            manager.get_substate(&id, Some(SubstateVersion::ZERO)),
        )
        .await
        .expect("one member's proven Down settles the read")
        .unwrap();
        assert!(matches!(lookup.result, SubstateResult::Down { .. }));
        assert!(lookup.verified);
    }

    /// A commit proof validated once is not validated again: repeated versioned reads of a destroyed version ask the
    /// committee once per commit proof.
    #[tokio::test]
    async fn a_validated_commit_proof_is_not_validated_again() {
        let keys = (1..=4)
            .map(tari_ootle_common_types::crypto::create_key_pair_from_seed)
            .collect::<Vec<_>>();
        let (id, manager) = signed_down_proof_setup(&keys, HashSet::new());
        let lookups = manager.committee_provider.2.clone();
        for _ in 0..3 {
            let (_, proof) = manager
                .get_substate_from_vn(
                    &"vn0".to_string(),
                    SubstateRequirementRef::new(&id, Some(SubstateVersion::ZERO)),
                )
                .await
                .unwrap();
            assert!(proof.is_some());
        }
        // One lookup for each of the two commit proofs a down proof cites.
        assert_eq!(lookups.load(Ordering::Relaxed), 2);
    }

    /// A down proof anchored before V2 cannot prove the Down, so it is left unproven before any committee is asked to
    /// validate its commit proofs.
    #[tokio::test]
    async fn a_down_proof_anchored_before_v2_is_unproven_without_validation() {
        let (id, proof, _, r2_root) = global_down_proof(group(1, 2));
        for (network, consulted) in [(Network::Esmeralda, false), (Network::LocalNet, true)] {
            let (mut manager, _) = manager(FakeNetwork {
                down_with_proof: [(
                    id.clone(),
                    (SubstateVersion::ZERO, SubstateProofData {
                        substate_value_proof: vec![],
                        commit_proof: unsigned_commit_proof(group(1, 2), 4, r2_root),
                        proof_epoch: 0,
                        substate_down_proof: Some(proof.clone()),
                        destroyed_at_state_version: Some(2),
                    }),
                )]
                .into(),
                ..Default::default()
            });
            manager.network = network;
            let lookups = manager.committee_provider.2.clone();
            let manager = manager.with_substate_proof_verification(true);
            let result = manager
                .get_substate_from_vn(
                    &"vn".to_string(),
                    SubstateRequirementRef::new(&id, Some(SubstateVersion::ZERO)),
                )
                .await;
            if consulted {
                // These commit proofs are unsigned, so validating them fails.
                assert!(
                    matches!(result, Err(IndexerError::SubstateProofVerificationFailed { .. })),
                    "{network}: {result:?}"
                );
                assert!(lookups.load(Ordering::Relaxed) > 0, "{network}");
            } else {
                let (result, proof) = result.unwrap();
                assert!(matches!(result, SubstateResult::Down { .. }));
                assert!(proof.is_none());
                assert_eq!(lookups.load(Ordering::Relaxed), 0, "{network}");
            }
        }
    }

    /// A head read cannot be settled by a down proof, so its down proof is not checked: an invalid one is an unproven
    /// Down, not grounds to disqualify the member. A read for the version it names checks it.
    #[tokio::test]
    async fn a_down_proof_is_checked_only_for_a_read_of_its_version() {
        let id = component_id(1);
        let (manager, _) = manager(FakeNetwork {
            down_with_proof: [(id.clone(), (SubstateVersion::new(3), down_proof_data(Some(vec![0xff]))))].into(),
            ..Default::default()
        });
        let manager = manager.with_substate_proof_verification(true);
        let (result, proof) = manager
            .get_substate_from_vn(&"vn".to_string(), SubstateRequirementRef::new(&id, None))
            .await
            .unwrap();
        assert!(matches!(result, SubstateResult::Down { .. }));
        assert!(proof.is_none());

        let result = manager
            .get_substate_from_vn(
                &"vn".to_string(),
                SubstateRequirementRef::new(&id, Some(SubstateVersion::new(3))),
            )
            .await;
        assert!(
            matches!(result, Err(IndexerError::SubstateProofVerificationFailed { .. })),
            "{result:?}"
        );
    }

    /// The epoch every test commit proof is of.
    const PROOF_EPOCH: Epoch = Epoch(1);

    #[tokio::test]
    async fn a_down_whose_earlier_root_has_no_known_committee_is_unproven() {
        let (mut manager, _) = manager(FakeNetwork::default());
        manager.committee_provider.1 = Some(PROOF_EPOCH);
        let manager = manager.with_substate_proof_verification(true);
        let (id, proof, _, r2_root) = global_down_proof(group(1, 2));
        let proven = manager
            .verify_down_proof_against(&id, SubstateVersion::ZERO, &proof, &r2_anchor(r2_root))
            .await
            .unwrap();
        assert!(!proven);
    }

    #[tokio::test]
    async fn a_down_whose_anchor_has_no_known_committee_is_unproven() {
        let (id, proof, _, r2_root) = global_down_proof(group(1, 2));
        let (mut manager, _) = manager(FakeNetwork {
            down_with_proof: [(
                id.clone(),
                (SubstateVersion::ZERO, SubstateProofData {
                    substate_value_proof: vec![],
                    commit_proof: unsigned_commit_proof(group(1, 2), 4, r2_root),
                    proof_epoch: 0,
                    substate_down_proof: Some(proof),
                    destroyed_at_state_version: Some(2),
                }),
            )]
            .into(),
            ..Default::default()
        });
        manager.committee_provider.1 = Some(PROOF_EPOCH);
        let manager = manager.with_substate_proof_verification(true);
        let (result, proof) = manager
            .get_substate_from_vn(
                &"vn".to_string(),
                SubstateRequirementRef::new(&id, Some(SubstateVersion::ZERO)),
            )
            .await
            .unwrap();
        assert!(matches!(result, SubstateResult::Down { .. }));
        assert!(proof.is_none());
    }

    /// A batch answers with heads, which a down proof cannot settle, so a batch's down proofs are not checked: one
    /// that would not verify, against an anchor its committee does not sign, does not fail the batch.
    #[tokio::test]
    async fn a_batch_does_not_check_its_down_proofs() {
        let (id, proof, _, r2_root) = global_down_proof(group(1, 2));
        let decoded = decode_substate_down_proof(&proof).unwrap();
        let batch = SubstateBatch {
            commit_proof: Some(unsigned_commit_proof(group(1, 2), 4, r2_root)),
            substates: vec![tari_validator_node_rpc::client::BatchedSubstate {
                substate_id: id,
                result: SubstateResult::Down {
                    version: SubstateVersion::ZERO,
                },
                value_proof: Some(tari_bor::serde_codec::to_vec(&decoded.down).unwrap()),
                proof_epoch: 0,
                substate_down_proof: Some(vec![0xff]),
            }],
            missing: vec![],
        };
        let (manager, _) = manager(FakeNetwork::default());
        let manager = manager
            .with_substate_proof_verification(true)
            .with_trusted_root_store(Arc::new(TrustEverything));
        let result = manager.verify_substate_batch(&batch).await;
        assert!(matches!(result, Ok(BatchTrust::Proven)), "{result:?}");
    }

    #[tokio::test]
    async fn it_fetches_live_inputs_in_one_batch() {
        let ids = (0..10).map(component_id).collect::<Vec<_>>();
        let (manager, network) = manager(FakeNetwork {
            live: live(&ids),
            ..Default::default()
        });

        let lookup = manager.get_input_substates(&requirements(&ids)).await.unwrap();

        let InputSubstatesLookup::AllUp(found) = lookup else {
            panic!("expected every input to be up, got {lookup:?}");
        };
        assert_eq!(found.len(), ids.len());
        assert_eq!(network.batch_requests.load(Ordering::Relaxed), 1);
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 0);
    }

    /// A transaction declaring inputs that do not exist cannot commit, so the lookup confirms the
    /// first such input and goes no further, however many more were declared.
    #[tokio::test]
    async fn it_stops_at_the_first_input_that_does_not_exist() {
        let ids = (0..100).map(component_id).collect::<Vec<_>>();
        let (manager, network) = manager(FakeNetwork::default());

        let lookup = manager.get_input_substates(&requirements(&ids)).await.unwrap();

        assert!(
            matches!(lookup, InputSubstatesLookup::DoesNotExist { .. }),
            "expected a missing input, got {lookup:?}"
        );
        assert_eq!(
            network.batch_requests.load(Ordering::Relaxed),
            ids.len().div_ceil(SUBSTATE_BATCH_SIZE)
        );
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 1);
    }

    /// An input one member omitted from its batch is confirmed with the committee, which finds it
    /// live.
    #[tokio::test]
    async fn it_confirms_an_input_a_batch_omitted() {
        let ids = (0..3).map(component_id).collect::<Vec<_>>();
        let (manager, network) = manager(FakeNetwork {
            live: live(&ids),
            omitted_from_batches: HashSet::from([ids[1].clone()]),
            ..Default::default()
        });

        let lookup = manager.get_input_substates(&requirements(&ids)).await.unwrap();

        let InputSubstatesLookup::AllUp(found) = lookup else {
            panic!("expected every input to be up, got {lookup:?}");
        };
        assert_eq!(found.len(), ids.len());
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 1);
    }

    /// A batch answers with heads, and a down head is not proven by its batch, so while verification is
    /// on the committee is asked before the lookup stops at it.
    #[tokio::test]
    async fn it_confirms_an_input_a_batch_reported_down() {
        let ids = (0..3).map(component_id).collect::<Vec<_>>();
        let (manager, network) = manager(FakeNetwork {
            live: live(&ids),
            down_in_batches: HashSet::from([ids[1].clone()]),
            ..Default::default()
        });
        let manager = manager.with_substate_proof_verification(true);

        let lookup = manager.get_input_substates(&requirements(&ids)).await.unwrap();

        let InputSubstatesLookup::AllUp(found) = lookup else {
            panic!("expected every input to be up, got {lookup:?}");
        };
        assert_eq!(found.len(), ids.len());
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 1);
    }

    /// A batch's down proofs are not checked, so peer bytes in one never ride along with a substate's verified proof.
    #[tokio::test]
    async fn a_proven_batch_entry_carries_no_down_proof() {
        use tari_engine_types::substate::hash_substate;
        use tari_ootle_common_types::{ToSubstateAddress, VersionedSubstateId, shard::Shard};
        use tari_state_tree::{
            SPARSE_MERKLE_PLACEHOLDER_HASH,
            ShardGroupRootTree,
            SpreadPrefixStateTree,
            StateTreePayload,
            SubstateTreeChange,
            SubstateValueProof,
            memory_store::MemoryTreeStore,
        };

        let id = component_id(1);
        let substate = live(std::slice::from_ref(&id)).remove(&id).unwrap();
        let versioned = VersionedSubstateId::new(id.clone(), substate.version());
        let shard = versioned.to_substate_address().to_shard(NumPreshards::P256);
        let mut store = MemoryTreeStore::<StateTreePayload>::new();
        let shard_root = SpreadPrefixStateTree::new(&mut store)
            .put_substate_changes(None, 1, vec![SubstateTreeChange::Up {
                id: versioned.clone(),
                value_hash: hash_substate(
                    Network::LocalNet,
                    substate.substate_value(),
                    substate.version(),
                    PROOF_EPOCH,
                ),
            }])
            .unwrap();
        let tree = ShardGroupRootTree::build(
            tari_engine_types::ProtocolVersion::at(Network::LocalNet, PROOF_EPOCH),
            [
                (Shard::global(), SPARSE_MERKLE_PLACEHOLDER_HASH, 0),
                (shard, shard_root, 1),
            ],
        )
        .unwrap();
        let (_, _, leaf_proof) = SpreadPrefixStateTree::new(&mut store).get_proof(1, &versioned).unwrap();
        let value_proof = SubstateValueProof::new(shard_root, 1, tree.get_proof(shard).unwrap().1, leaf_proof);
        let all_shards = group(1, NumPreshards::P256.as_u32());

        let (manager, _) = manager(FakeNetwork {
            proven_batch: Some(SubstateBatch {
                commit_proof: Some(unsigned_commit_proof(all_shards, 4, tree.root())),
                substates: vec![tari_validator_node_rpc::client::BatchedSubstate {
                    substate_id: id.clone(),
                    result: SubstateResult::Up {
                        substate: Box::new(substate),
                    },
                    value_proof: Some(tari_bor::serde_codec::to_vec(&value_proof).unwrap()),
                    proof_epoch: PROOF_EPOCH.as_u64(),
                    substate_down_proof: Some(vec![7, 7]),
                }],
                missing: vec![],
            }),
            ..Default::default()
        });
        // The unsigned anchor is taken as trusted, standing in for a validated one.
        let manager = manager
            .with_substate_proof_verification(true)
            .with_trusted_root_store(Arc::new(TrustEverything));

        let found = manager.fetch_and_cache_substates(&[id.clone()]).await.unwrap();

        let proof = found[&id].proof.as_ref().expect("the batch proved the substate");
        assert_eq!(proof.substate_down_proof, None);
    }

    /// A manager without cache writes, as the dry-run manager runs, leaves a cache it shares untouched however it
    /// reads, while one with them writes what it fetched without verification as before.
    #[tokio::test]
    async fn a_manager_without_cache_writes_leaves_the_cache_untouched() {
        let ids = (0..3).map(component_id).collect::<Vec<_>>();
        for cache_writes in [false, true] {
            let (manager, _) = manager(FakeNetwork {
                live: live(&ids),
                down_in_batches: HashSet::from([ids[1].clone()]),
                ..Default::default()
            });
            let manager = manager
                .with_substate_proof_verification(false)
                .with_cache_writes(cache_writes);

            manager.fetch_and_cache_substates(&ids).await.unwrap();
            manager.get_substate(&ids[2], None).await.unwrap();
            manager.get_input_substates(&requirements(&ids)).await.unwrap();

            let cached = manager.substate_cache.0.lock().unwrap().len();
            if cache_writes {
                assert_eq!(cached, ids.len());
            } else {
                assert_eq!(cached, 0);
            }
        }
    }

    /// One member answering a batch with an unproven Down cannot hide a substate the committee holds up.
    #[tokio::test]
    async fn a_batch_reporting_a_live_substate_down_does_not_drop_it() {
        let ids = (0..3).map(component_id).collect::<Vec<_>>();
        let (manager, network) = manager(FakeNetwork {
            live: live(&ids),
            down_in_batches: HashSet::from([ids[1].clone()]),
            ..Default::default()
        });
        let manager = manager.with_substate_proof_verification(true);

        let found = manager.fetch_and_cache_substates(&ids).await.unwrap();

        assert_eq!(found.len(), ids.len(), "{:?}", found.keys().collect::<Vec<_>>());
        assert!(found.contains_key(&ids[1]));
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 1);
    }

    /// A substate whose state the committee cannot confirm fails the call, rather than reading as not live.
    #[tokio::test]
    async fn a_batch_head_that_cannot_be_confirmed_fails_the_call() {
        let ids = (0..4).map(component_id).collect::<Vec<_>>();
        let (manager, _) = manager(FakeNetwork {
            live: live(&ids),
            down_in_batches: HashSet::from([ids[1].clone(), ids[2].clone()]),
            failing_single_reads: HashSet::from([ids[2].clone()]),
            ..Default::default()
        });
        let manager = manager.with_substate_proof_verification(true);

        let result = manager.fetch_and_cache_substates(&ids).await;

        assert!(result.is_err(), "{result:?}");
    }

    /// A member that leaves an id out of its batch cannot hide a substate the committee holds up.
    #[tokio::test]
    async fn an_id_a_batch_leaves_out_is_confirmed_with_the_committee() {
        let ids = (0..3).map(component_id).collect::<Vec<_>>();
        for verify in [true, false] {
            let (manager, network) = manager(FakeNetwork {
                live: live(&ids),
                omitted_from_batches: HashSet::from([ids[1].clone()]),
                ..Default::default()
            });
            let manager = manager.with_substate_proof_verification(verify);

            let found = manager.fetch_and_cache_substates(&ids).await.unwrap();

            assert_eq!(found.len(), ids.len(), "verify: {verify}");
            assert!(found.contains_key(&ids[1]), "verify: {verify}");
            assert_eq!(network.single_requests.load(Ordering::Relaxed), 1, "verify: {verify}");
        }
    }

    /// An id the committee holds absent is left out.
    #[tokio::test]
    async fn an_id_the_committee_holds_absent_is_left_out() {
        let ids = (0..3).map(component_id).collect::<Vec<_>>();
        let (manager, _) = manager(FakeNetwork {
            live: live(&ids[..2]),
            ..Default::default()
        });
        let manager = manager.with_substate_proof_verification(true);

        let found = manager.fetch_and_cache_substates(&ids).await.unwrap();

        assert_eq!(found.len(), 2);
        assert!(!found.contains_key(&ids[2]));
    }

    /// A cached nonexistence ends the lookup before anything is asked of the network.
    #[tokio::test]
    async fn it_stops_at_a_cached_nonexistent_input_without_fetching() {
        let ids = (0..5).map(component_id).collect::<Vec<_>>();
        let (manager, network) = manager(FakeNetwork::default());
        manager.get_substate(&ids[4], None).await.unwrap();
        network.single_requests.store(0, Ordering::Relaxed);

        let mut reqs = requirements(&ids);
        reqs.reverse();
        let lookup = manager.get_input_substates(&reqs).await.unwrap();

        assert!(
            matches!(lookup, InputSubstatesLookup::DoesNotExist { ref substate_id } if *substate_id == ids[4]),
            "expected the cached missing input, got {lookup:?}"
        );
        assert_eq!(network.batch_requests.load(Ordering::Relaxed), 0);
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 0);
    }

    /// A fresh head with no proof held for it cannot answer a read that has to return one.
    #[tokio::test]
    async fn a_cached_head_with_no_proof_held_is_fetched_again() {
        let ids = vec![component_id(1)];
        let (manager, network) = manager(FakeNetwork {
            live: live(&ids),
            ..Default::default()
        });
        manager.get_substate(&ids[0], None).await.unwrap();
        manager.get_substate(&ids[0], None).await.unwrap();
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 1);

        let lookup = manager.get_substate_with_proof(&ids[0], None).await.unwrap();

        assert!(lookup.result.into_up().is_some());
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn a_cached_head_is_served_with_the_proof_held_for_it() {
        let ids = vec![component_id(1)];
        let (manager, network) = manager(FakeNetwork {
            live: live(&ids),
            ..Default::default()
        });
        let result = SubstateResult::Up {
            substate: Box::new(live(&ids).remove(&ids[0]).unwrap()),
        };
        let proof = SubstateProofData {
            substate_value_proof: vec![1],
            commit_proof: vec![2],
            proof_epoch: 0,
            substate_down_proof: None,
            destroyed_at_state_version: None,
        };
        manager
            .substate_cache
            .write(
                &ids[0],
                SubstateCacheEntryRef {
                    version: result.version(),
                    substate_result: &result,
                    cached_at: SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    verified: true,
                    proof: Some(&proof),
                },
                FetchWatermark::new(0),
            )
            .await
            .unwrap();

        let lookup = manager.get_substate_with_proof(&ids[0], None).await.unwrap();

        assert_eq!(lookup.proof.unwrap().substate_value_proof, vec![1]);
        assert_eq!(network.single_requests.load(Ordering::Relaxed), 0);
    }
}
