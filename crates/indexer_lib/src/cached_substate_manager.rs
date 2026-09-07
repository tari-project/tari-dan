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
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use futures::FutureExt;
use log::*;
use ootle_network::Network;
use tari_common_types::types::FixedHash;
use tari_engine_types::substate::{Substate, SubstateId, SubstateValue};
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    NumPreshards,
    ShardGroup,
    SubstateAddress,
    SubstateRequirementRef,
    ToSubstateAddress,
    VotePower,
    committee::Committee,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_storage::{
    consensus_models::{CommittedBlockProof, VerifiedBlockTip},
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

/// The most substates a validator will answer for in one batch request.
const SUBSTATE_BATCH_SIZE: usize = 50;

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
        specific_version: Option<u32>,
    ) -> Result<SubstateLookupResult, IndexerError> {
        debug!(target: LOG_TARGET, "get_substate: {}v{}", substate_id, specific_version.display());
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
                    return Ok(SubstateLookupResult {
                        result: entry.substate_result,
                        verified: entry.verified,
                    });
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
        #[cfg(feature = "metrics")]
        self.metrics.as_ref().inspect(|m| m.inc_cache_misses());

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
            if is_head && admissible {
                let version = lookup_result.result.version();
                debug!(target: LOG_TARGET, "Updating cached substate {} with version {}", substate_id, version.display());
                let entry = SubstateCacheEntryRef {
                    version,
                    substate_result: &lookup_result.result,
                    cached_at: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs(),
                    verified: lookup_result.verified,
                };
                self.substate_cache.write(substate_id, entry, watermark).await?;
            }
        }

        Ok(lookup_result)
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
        substate_ids: &'a [SubstateId],
        epoch: Epoch,
        num_committees: u32,
    ) -> Result<HashMap<ShardGroup, (Arc<Committee<TAddr>>, Vec<&'a SubstateId>)>, IndexerError> {
        let mut map = HashMap::<_, (_, Vec<&'a SubstateId>)>::with_capacity(substate_ids.len());
        for substate_id in substate_ids {
            let shard_group = SubstateAddress::from_substate_id(substate_id, 0)
                .to_shard_group(NumPreshards::current(), num_committees);
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

    pub async fn fetch_and_cache_substates(
        &self,
        substate_ids: &[SubstateId],
    ) -> Result<HashMap<SubstateId, Substate>, IndexerError> {
        let epoch = self.committee_provider.current_epoch().await?;
        let num_committees = self.committee_provider.get_num_committees(epoch).await?;
        let committee_map = self.build_vn_committee_map(substate_ids, epoch, num_committees).await?;

        // Captured before any fetch so that a transition arriving while one is in flight can veto the
        // write it produces.
        let mut watermarks = HashMap::with_capacity(substate_ids.len());
        for substate_id in substate_ids {
            if let Some(watermark) = self.substate_cache.watermark(substate_id).await? {
                watermarks.insert(substate_id, watermark);
            }
        }

        let mut results = HashMap::with_capacity(substate_ids.len());
        for (shard_group, (committee, substate_ids)) in committee_map {
            debug!(target: LOG_TARGET, "Fetching {} substates from shard group {}", substate_ids.len(), shard_group);
            for chunk in substate_ids.chunks(SUBSTATE_BATCH_SIZE) {
                let (batch, verified) = self.race_substate_batch(&committee, chunk, shard_group).await?;
                if !batch.missing.is_empty() {
                    debug!(
                        target: LOG_TARGET,
                        "{} of {} requested substate(s) are unknown to {shard_group}",
                        batch.missing.len(),
                        chunk.len()
                    );
                }

                for substate in batch.substates {
                    if let Some(watermark) = watermarks.get(&substate.substate_id).copied() {
                        let entry = SubstateCacheEntryRef {
                            version: substate.result.version(),
                            substate_result: &substate.result,
                            cached_at: SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs(),
                            verified,
                        };
                        // An unverified entry is not cached while verification is on, so the next
                        // read retries for a proven copy instead of pinning an unproven value.
                        if verified || !self.verify_substate_proofs {
                            self.substate_cache
                                .write(&substate.substate_id, entry, watermark)
                                .await?;
                        }
                    }

                    // A batch answers with the head version; a caller asking for substates by id
                    // wants the live ones, and a down head is not one.
                    if let Some(up) = substate.result.into_up() {
                        results.insert(substate.substate_id, up);
                    }
                }
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

        let root = self.trusted_root_from_commit_proof(commit_proof).await?;
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
                Epoch(substate.proof_epoch),
                root,
            )
            .map_err(|e| IndexerError::SubstateProofVerificationFailed { details: e.to_string() })?;
        }

        Ok(BatchTrust::Proven)
    }

    async fn fetch_substate_from_committee(
        &self,
        substate_id: &SubstateId,
        specific_version: Option<u32>,
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

        let tally = CommitteeReadTally::new(committee.len(), self.verify_substate_proofs);
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
            Ok((substate_result, verified)) => {
                debug!(target: LOG_TARGET, "Got substate result for {} from vn {} (verified = {}): {:?}", substate_req, vn_addr, verified, substate_result);
            },
            Err(e) => {
                warn!(target: LOG_TARGET, "Could not get substate {} from vn {}: {}", substate_req, vn_addr, e);
            },
        }
        response
    }

    /// Gets a substate directly from querying a VN. The returned flag is true if the result came
    /// with a proof that verified against the committee.
    async fn get_substate_from_vn(
        &self,
        vn_addr: &TAddr,
        substate_requirement: SubstateRequirementRef<'_>,
    ) -> Result<(SubstateResult, bool), IndexerError> {
        // build a client with the VN
        let mut client = self.validator_node_client_factory.create_client(vn_addr);

        if !self.verify_substate_proofs {
            return client
                .get_substate(substate_requirement)
                .await
                .map(|result| (result, false))
                .map_err(|e| IndexerError::ValidatorNodeClientError(e.to_string()));
        }

        let (result, proof) = client
            .get_substate_with_proof(substate_requirement)
            .await
            .map_err(|e| IndexerError::ValidatorNodeClientError(e.to_string()))?;

        // The validator has nothing committed to anchor a proof against yet (e.g. immediately after
        // an epoch change). Return the result unverified and let the caller decide.
        let Some(proof) = proof else {
            return Ok((result, false));
        };

        // Verify up/down results against the committee. An invalid proof disqualifies this
        // validator's response (fail-closed) so the caller tries another member. `DoesNotExist` is
        // not provable and is left to the existing f+1 agreement.
        let verified = match &result {
            SubstateResult::Up { substate } => {
                self.verify_substate_proof(
                    substate_requirement.substate_id(),
                    substate.version(),
                    Some(substate.substate_value()),
                    proof,
                )
                .await?;
                true
            },
            SubstateResult::Down { version } => {
                self.verify_substate_proof(substate_requirement.substate_id(), *version, None, proof)
                    .await?;
                true
            },
            SubstateResult::DoesNotExist => false,
        };

        Ok((result, verified))
    }

    async fn verify_substate_proof(
        &self,
        substate_id: &SubstateId,
        version: u32,
        value: Option<&SubstateValue>,
        proof: SubstateProofData,
    ) -> Result<(), IndexerError> {
        let root = self.trusted_root_from_commit_proof(&proof.commit_proof).await?;
        verify_substate_value_proof_against_root(
            &proof.substate_value_proof,
            substate_id,
            version,
            value,
            self.network,
            Epoch(proof.proof_epoch),
            root,
        )
        .map_err(|e| IndexerError::SubstateProofVerificationFailed { details: e.to_string() })?;
        Ok(())
    }

    /// Establishes the shard-group state merkle root that value proofs anchored to `commit_proof`
    /// must verify against.
    ///
    /// The returned root is trusted because a quorum of the shard group signed the block header
    /// committing it, independently of any value proof that goes on to cite it. That is what makes it
    /// safe to establish once and reuse for a whole batch of value proofs, and to record for later
    /// reads.
    async fn trusted_root_from_commit_proof(&self, commit_proof: &[u8]) -> Result<FixedHash, IndexerError> {
        let commit_proof = CommittedBlockProof::from_bytes(commit_proof).map_err(|e| {
            IndexerError::SubstateProofVerificationFailed {
                details: format!("undecodable commit proof: {e}"),
            }
        })?;
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
            return Ok(root);
        }

        // Slow path: validate the commit proof against the shard group committee.
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

        let root = verified_tip.state_merkle_root;
        // Warm the store so subsequent reads at this tip hit the fast path. A write failure must not
        // fail an otherwise-verified read.
        if let Some(store) = &self.trusted_root_store &&
            let Err(e) = store.record(verified_tip).await
        {
            warn!(target: LOG_TARGET, "Failed to record verified root at epoch {epoch} {shard_group}: {e}");
        }

        Ok(root)
    }
}
