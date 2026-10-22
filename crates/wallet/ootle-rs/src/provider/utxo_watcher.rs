//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeMap, sync::Weak};

use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tari_indexer_client::{
    error::IndexerRestClientError,
    protobuf::{self, UtxoUpdatePayload},
    protobuf_stream::ProtobufStreamError,
    rest_api_client::IndexerRestApiClient,
    types::GetUtxoUpdatesRequest,
};
use tari_ootle_common_types::{
    Epoch,
    NumPreshards,
    StateVersion,
    SubstateVersion,
    array_utils::copy_fixed_checked,
    shard::Shard,
};
use tari_template_lib_types::{
    ResourceAddress,
    UtxoId,
    crypto::{RistrettoPublicKeyBytes, UtxoTag},
};
use tracing::{debug, error, warn};

/// Request to watch stealth UTXO updates for a resource, resuming from a per-shard cursor.
#[derive(Debug, Clone)]
pub struct StealthUtxoWatchRequest {
    pub resource_address: ResourceAddress,
    /// Lower bound epoch for the scan. Individual shards resume from `shard_state_versions`.
    pub from_epoch: Epoch,
    /// Per-shard resume cursor, from [`ShardCursor::resume`]. Shards not present default to `StateVersion::zero` on
    /// the indexer.
    pub shard_state_versions: Vec<(Shard, StateVersion)>,
    /// When true, only currently-unspent UTXOs are streamed (no `Spent`/`Burnt` frames).
    /// A monitor that must track spends has to set this to `false`.
    pub unspent_only: bool,
    /// Number of UTXO rows the indexer reads per shard per pass, before `from_epoch` and
    /// `unspent_only` filter them. A pass may return fewer updates and still have more to give, and
    /// may overrun this to keep one state version whole, so
    /// [`StealthUtxoFrame::StartOfShard::has_more`] — not a count comparison — says whether the
    /// shard has updates left to drain.
    pub per_shard_limit: u32,
}

impl StealthUtxoWatchRequest {
    fn into_request(self) -> GetUtxoUpdatesRequest {
        GetUtxoUpdatesRequest {
            from_epoch: self.from_epoch,
            shard_state_versions: self.shard_state_versions,
            resource_address: self.resource_address,
            unspent_only: self.unspent_only,
            per_shard_limit: self.per_shard_limit,
        }
    }
}

/// A single decoded frame from the stealth UTXO update stream.
///
/// Frames arrive grouped per shard: a `StartOfShard`, then zero or more `Unspent`/`Spent`/`Burnt`
/// updates, terminated by an `EndOfShard`. `EndOfShard::max_state_version` is the shard's resume
/// point — every update at or below it has either been delivered or was excluded by the request's
/// own filters — so advance the cursor from it via [`ShardCursor::observe`].
#[derive(Debug, Clone)]
pub enum StealthUtxoFrame {
    StartOfShard {
        shard: Shard,
        max_state_version: StateVersion,
        num_updates: u32,
        /// The shard has updates beyond this pass. Poll again to drain it rather than waiting for
        /// the next scheduled pass.
        has_more: bool,
    },
    /// A currently-unspent output. Carries only the `(tag, public_nonce)` fetch key — the commitment
    /// and viewable-balance ciphertext must be resolved via
    /// [`fetch_unspent_utxos`](crate::provider::IndexerProvider::fetch_unspent_utxos).
    Unspent {
        tag: UtxoTag,
        public_nonce: RistrettoPublicKeyBytes,
    },
    Spent {
        id: UtxoId,
        version: SubstateVersion,
    },
    Burnt {
        id: UtxoId,
        version: SubstateVersion,
    },
    /// Terminates a shard's updates. `max_state_version` is the resume point for the shard: the
    /// state version the indexer's read reached, which can be past the last delivered update when
    /// the request's filters excluded the rows after it. A shard can end with no updates at all
    /// and still move the cursor.
    EndOfShard {
        shard: Shard,
        max_state_version: StateVersion,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum UtxoWatcherError {
    #[error("Indexer REST client has been dropped")]
    ClientDropped,
    #[error("Indexer REST client error: {0}")]
    IndexerClientError(#[from] IndexerRestClientError),
    #[error("UTXO stream error: {0}")]
    StreamError(#[from] ProtobufStreamError),
    #[error("Failed to decode UTXO update frame: {0}")]
    DecodeError(String),
}

/// A stream of stealth UTXO updates for a resource. Created via
/// [`IndexerProvider::watch_stealth_utxos`](crate::provider::IndexerProvider::watch_stealth_utxos).
///
/// Each call drains a single pass over the requested shards (paged by `per_shard_limit`) and then
/// ends. A monitor loops: build the request from its persisted [`ShardCursor`] with
/// [`ShardCursor::resume`], drain the stream, advance the cursor from `EndOfShard` watermarks, persist it, then poll
/// again.
pub struct StealthUtxoStream {
    client: Weak<IndexerRestApiClient>,
    request: StealthUtxoWatchRequest,
}

impl StealthUtxoStream {
    pub(crate) fn new(client: Weak<IndexerRestApiClient>, request: StealthUtxoWatchRequest) -> Self {
        Self { client, request }
    }

    /// Consume into a futures Stream of typed [`StealthUtxoFrame`] items.
    pub fn into_stream(self) -> impl Stream<Item = Result<StealthUtxoFrame, UtxoWatcherError>> {
        async_stream::stream! {
            let client = match self.client.upgrade() {
                Some(client) => client,
                None => {
                    error!("Indexer REST client has been dropped");
                    yield Err(UtxoWatcherError::ClientDropped);
                    return;
                },
            };

            let mut stream = match client.stream_utxo_updates_protobuf(self.request.into_request()).await {
                Ok(stream) => stream,
                Err(err) => {
                    error!(%err, "Failed to start stealth UTXO update stream");
                    yield Err(UtxoWatcherError::IndexerClientError(err));
                    return;
                },
            };

            // `EndOfShard` on the wire does not carry the shard number; stamp it from the preceding
            // `StartOfShard` so each frame is self-describing.
            let mut current_shard: Option<Shard> = None;
            loop {
                match stream.next().await {
                    Some(Ok(payload)) => {
                        let UtxoUpdatePayload { sos, update, eos } = payload;
                        if let Some(sos) = sos {
                            let shard = Shard::from(sos.shard);
                            current_shard = Some(shard);
                            yield Ok(StealthUtxoFrame::StartOfShard {
                                shard,
                                max_state_version: StateVersion::from(sos.max_state_version),
                                num_updates: sos.num_updates,
                                has_more: sos.has_more,
                            });
                        }
                        if let Some(update) = update {
                            match convert_update(update) {
                                Ok(frame) => yield Ok(frame),
                                Err(err) => {
                                    yield Err(err);
                                    return;
                                },
                            }
                        }
                        if let Some(eos) = eos {
                            // Take the shard so a following `EndOfShard` (or any frame) that is not
                            // preceded by a fresh `StartOfShard` errors out rather than being
                            // misattributed to this shard and corrupting its cursor.
                            let Some(shard) = current_shard.take() else {
                                yield Err(UtxoWatcherError::DecodeError(
                                    "EndOfShard received before any StartOfShard".to_string(),
                                ));
                                return;
                            };
                            yield Ok(StealthUtxoFrame::EndOfShard {
                                shard,
                                max_state_version: StateVersion::from(eos.max_state_version),
                            });
                        }
                    },
                    Some(Err(err)) => {
                        error!(%err, "Error receiving stealth UTXO update");
                        yield Err(UtxoWatcherError::StreamError(err));
                        return;
                    },
                    None => return,
                }
            }
        }
    }
}

fn convert_update(update: protobuf::WalletUtxoUpdate) -> Result<StealthUtxoFrame, UtxoWatcherError> {
    match update {
        protobuf::WalletUtxoUpdate::Unspent(unspent) => {
            let public_nonce = RistrettoPublicKeyBytes::from_bytes(&unspent.public_nonce)
                .map_err(|e| UtxoWatcherError::DecodeError(format!("public nonce: {e}")))?;
            Ok(StealthUtxoFrame::Unspent {
                tag: unspent.tag.into(),
                public_nonce,
            })
        },
        protobuf::WalletUtxoUpdate::Spent(spent) => {
            let id = copy_fixed_checked(&spent.id)
                .map(UtxoId::from_array)
                .ok_or_else(|| UtxoWatcherError::DecodeError("UTXO id: incorrect length".to_string()))?;
            Ok(StealthUtxoFrame::Spent {
                id,
                version: SubstateVersion::new(spent.version),
            })
        },
        protobuf::WalletUtxoUpdate::Burnt(burnt) => {
            let id = copy_fixed_checked(&burnt.id)
                .map(UtxoId::from_array)
                .ok_or_else(|| UtxoWatcherError::DecodeError("UTXO id: incorrect length".to_string()))?;
            Ok(StealthUtxoFrame::Burnt {
                id,
                version: SubstateVersion::new(burnt.version),
            })
        },
    }
}

/// A claim expires once the provider verifies a checkpoint at least this many epochs after the latest one it had
/// verified when the claim began. The network may have ended one more epoch than the provider had verified by then, so
/// the checkpoint after that is the first that certainly ended after the claim began, and an honest claim cannot be
/// past it.
pub const CLAIM_EXPIRY_EPOCHS: u64 = 2;

/// The state version a shard reached at the end of `epoch`, from that epoch's verified end-of-epoch checkpoint of the
/// shard's group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointStateVersion {
    pub epoch: Epoch,
    pub state_version: StateVersion,
}

/// A shard's cursor as an indexer reported it, past what a verified checkpoint commits for the shard. A pass resumes
/// from it, and the shard's verified cursor stays where the claim began, until a verified checkpoint that ended after
/// the claim began expires it. The shard is then scanned again from the verified cursor, so that an indexer's claim
/// hides no update for longer than that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimedStateVersion {
    pub state_version: StateVersion,
    /// The state version the claim began at. An honest indexer reported no more than the network had committed then,
    /// so a checkpoint that ended after the claim began commits at least this.
    pub first_state_version: StateVersion,
    /// The epoch of the latest verified checkpoint of the shard when the claim began, or `None` without one.
    pub verified_epoch: Option<Epoch>,
}

/// How far a [`ShardCursor`] moves each shard's verified cursor. Fetch it for each pass with
/// [`IndexerProvider::shard_state_version_bound`](crate::provider::IndexerProvider::shard_state_version_bound).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShardStateVersionBound {
    /// Every shard's verified cursor moves as the indexer reports it, for a provider that trusts its indexer.
    Unbounded,
    /// Each shard's verified cursor moves no further than its state version in a verified end-of-epoch checkpoint,
    /// which the network committed. A shard without one keeps its verified cursor. What the indexer reports past that
    /// is held as a claim.
    Verified(BTreeMap<Shard, CheckpointStateVersion>),
}

impl Default for ShardStateVersionBound {
    /// No shard state version verified, under which a pass holds its progress only as claims.
    fn default() -> Self {
        Self::Verified(BTreeMap::new())
    }
}

impl ShardStateVersionBound {
    fn checkpoint(&self, shard: Shard) -> Option<&CheckpointStateVersion> {
        match self {
            Self::Unbounded => None,
            Self::Verified(versions) => versions.get(&shard),
        }
    }

    /// The verified checkpoint of `shard` that ended after `claim` began, if any. For a claim that began without a
    /// verified checkpoint, that is the first one.
    fn checkpoint_after(&self, shard: Shard, claim: &ClaimedStateVersion) -> Option<&CheckpointStateVersion> {
        self.checkpoint(shard).filter(|checkpoint| {
            claim
                .verified_epoch
                .is_none_or(|epoch| checkpoint.epoch.as_u64() >= epoch.as_u64().saturating_add(CLAIM_EXPIRY_EPOCHS))
        })
    }

    /// Whether `claim` has expired, and its shard is to be scanned again from the verified cursor.
    fn expires(&self, shard: Shard, claim: &ClaimedStateVersion) -> bool {
        self.checkpoint_after(shard, claim).is_some()
    }

    /// Whether a verified checkpoint that certainly ended after `claim` began commits a lower state version than the
    /// claim began at, which proves the indexer reported updates the network never committed.
    fn disproves(&self, shard: Shard, claim: &ClaimedStateVersion) -> bool {
        claim.verified_epoch.is_some() &&
            self.checkpoint_after(shard, claim)
                .is_some_and(|checkpoint| claim.first_state_version > checkpoint.state_version)
    }
}

/// One shard's position in a [`ShardCursor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardPosition {
    /// The state version the shard was scanned to, no further than a verified checkpoint commits for the shard.
    pub verified: StateVersion,
    /// The indexer's report past `verified`, which a pass resumes from until it expires.
    pub claim: Option<ClaimedStateVersion>,
}

impl Default for ShardPosition {
    fn default() -> Self {
        Self {
            verified: StateVersion::zero(),
            claim: None,
        }
    }
}

impl ShardPosition {
    /// The state version a pass resumes the shard from.
    pub fn resume_version(&self) -> StateVersion {
        self.claim.map_or(self.verified, |claim| claim.state_version)
    }
}

/// A per-shard resume cursor for the stealth UTXO stream, which the application persists between passes (it is
/// serde-serializable, or see [`ShardCursor::positions`]).
///
/// Seed a fresh monitor with [`ShardCursor::genesis`] (all shards at `StateVersion::zero`). For each pass, fetch a
/// [`ShardStateVersionBound`], pass [`ShardCursor::resume`] into a [`StealthUtxoWatchRequest`], and advance the cursor
/// from each [`StealthUtxoFrame::EndOfShard`] watermark via [`ShardCursor::observe`] under the same bound.
///
/// The watermarks are the indexer's claims, and a pass resumes past every version its cursor holds. An indexer that
/// claims a version past the shard's real one would hide every later update of the shard from each later pass,
/// whichever indexer serves it. So each shard's verified cursor moves no further than the shard's state version in a
/// verified end-of-epoch checkpoint, and the report past it is a claim that expires [`CLAIM_EXPIRY_EPOCHS`] verified
/// checkpoints after it began. The shard is then scanned again from the verified cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardCursor {
    positions: BTreeMap<Shard, ShardPosition>,
}

impl ShardCursor {
    /// A cursor covering every shard of `num_preshards`, each at `StateVersion::zero` (full backfill).
    pub fn genesis(num_preshards: NumPreshards) -> Self {
        num_preshards
            .all_shards_iter()
            .map(|shard| (shard, ShardPosition::default()))
            .collect()
    }

    /// Every tracked shard's position, ascending by shard, for an application that persists them itself. Collect them
    /// back into a cursor with [`FromIterator`].
    pub fn positions(&self) -> impl Iterator<Item = (Shard, ShardPosition)> + '_ {
        self.positions.iter().map(|(shard, position)| (*shard, *position))
    }

    /// The position of `shard` (`StateVersion::zero` without a claim if untracked).
    pub fn position(&self, shard: Shard) -> ShardPosition {
        self.positions.get(&shard).copied().unwrap_or_default()
    }

    /// The state version a pass resumes `shard` from (`StateVersion::zero` if untracked).
    pub fn get(&self, shard: Shard) -> StateVersion {
        self.position(shard).resume_version()
    }

    /// Lowers each verified cursor past `bound` to it and drops each claim that `bound` expires or that is no longer
    /// past the verified cursor, then returns the `(shard, state_version)` pairs (ascending by shard) that a watch
    /// request resumes from. Under [`ShardStateVersionBound::Unbounded`] every claim is dropped.
    pub fn resume(&mut self, bound: &ShardStateVersionBound) -> Vec<(Shard, StateVersion)> {
        let mut lowered = 0usize;
        let mut expired = 0usize;
        let mut disproved = 0usize;
        let unbounded = matches!(bound, ShardStateVersionBound::Unbounded);
        for (shard, position) in &mut self.positions {
            if let Some(checkpoint) = bound.checkpoint(*shard) &&
                position.verified > checkpoint.state_version
            {
                position.verified = checkpoint.state_version;
                lowered += 1;
            }
            let Some(claim) = position.claim else {
                continue;
            };
            if unbounded || bound.expires(*shard, &claim) || claim.state_version <= position.verified {
                if bound.disproves(*shard, &claim) {
                    disproved += 1;
                }
                position.claim = None;
                expired += 1;
            }
        }

        if disproved > 0 {
            warn!(
                "{disproved} stealth scan cursor(s) that an indexer claimed are past the state version a later \
                 verified checkpoint commits, so that indexer reported updates the network never committed."
            );
        }
        if expired > 0 {
            debug!(
                "{expired} claimed stealth scan cursor(s) expired. Rescanning those shards from the verified cursor."
            );
        }
        if lowered > 0 {
            warn!(
                "{lowered} stealth scan cursor(s) were past the verified shard state version. Rescanning those shards \
                 from it."
            );
        }

        self.positions
            .iter()
            .map(|(shard, position)| (*shard, position.resume_version()))
            .collect()
    }

    /// Advances `shard` to the indexer's `max_state_version` under the `bound` the pass resumed under. Monotonic:
    /// never moves the resume version backwards.
    ///
    /// A shard resumed from a claim advances the claim, which keeps where it began so that it expires on schedule and
    /// is checked against what the indexer first reported, and its verified cursor stays put. Otherwise the verified
    /// cursor moves to `max_state_version`, no further than `bound` allows, and a claim begins for the rest.
    pub fn observe(&mut self, shard: Shard, max_state_version: StateVersion, bound: &ShardStateVersionBound) {
        let position = self.positions.entry(shard).or_default();
        if max_state_version <= position.resume_version() {
            return;
        }
        if let Some(claim) = &mut position.claim {
            claim.state_version = max_state_version;
            return;
        }
        let ShardStateVersionBound::Verified(versions) = bound else {
            position.verified = max_state_version;
            return;
        };
        let checkpoint = versions.get(&shard);
        if let Some(checkpoint) = checkpoint {
            position.verified = position.verified.max(max_state_version.min(checkpoint.state_version));
        }
        if max_state_version > position.verified {
            position.claim = Some(ClaimedStateVersion {
                state_version: max_state_version,
                first_state_version: max_state_version,
                verified_epoch: checkpoint.map(|checkpoint| checkpoint.epoch),
            });
        }
    }
}

impl FromIterator<(Shard, ShardPosition)> for ShardCursor {
    fn from_iter<T: IntoIterator<Item = (Shard, ShardPosition)>>(positions: T) -> Self {
        Self {
            positions: positions.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARD: Shard = Shard::from_u32(1);

    fn v(state_version: u64) -> StateVersion {
        StateVersion::from(state_version)
    }

    fn bound_at(epoch: u64, state_version: u64) -> ShardStateVersionBound {
        ShardStateVersionBound::Verified(BTreeMap::from([(SHARD, CheckpointStateVersion {
            epoch: Epoch(epoch),
            state_version: v(state_version),
        })]))
    }

    fn claim(first_state_version: u64, state_version: u64, verified_epoch: Option<u64>) -> ClaimedStateVersion {
        ClaimedStateVersion {
            state_version: v(state_version),
            first_state_version: v(first_state_version),
            verified_epoch: verified_epoch.map(Epoch),
        }
    }

    fn cursor_at(verified: u64, claim: Option<ClaimedStateVersion>) -> ShardCursor {
        ShardCursor::from_iter([(SHARD, ShardPosition {
            verified: v(verified),
            claim,
        })])
    }

    /// The state version a pass under `bound` resumes [`SHARD`] from.
    fn resume_version(cursor: &mut ShardCursor, bound: &ShardStateVersionBound) -> StateVersion {
        let pairs = cursor.resume(bound);
        pairs
            .into_iter()
            .find(|(shard, _)| *shard == SHARD)
            .map(|(_, version)| version)
            .unwrap()
    }

    #[test]
    fn genesis_covers_every_shard_at_zero() {
        let mut cursor = ShardCursor::genesis(NumPreshards::P4);
        let pairs = cursor.resume(&ShardStateVersionBound::default());
        assert_eq!(pairs.len(), 4);
        assert!(pairs.iter().all(|(_, v)| *v == StateVersion::zero()));
    }

    #[test]
    fn a_trusting_cursor_advances_monotonically() {
        let mut cursor = ShardCursor::default();
        let bound = ShardStateVersionBound::Unbounded;
        cursor.observe(SHARD, v(250), &bound);
        cursor.observe(SHARD, v(100), &bound);
        assert_eq!(cursor.position(SHARD), ShardPosition {
            verified: v(250),
            claim: None,
        });
    }

    #[test]
    fn a_poisoned_watermark_is_held_as_a_claim_and_rescanned_from_the_verified_state_version_once_it_expires() {
        let mut cursor = ShardCursor::genesis(NumPreshards::P4);
        let bound = bound_at(2, 100);
        assert_eq!(resume_version(&mut cursor, &bound), v(0));

        cursor.observe(SHARD, v(u64::MAX), &bound);
        assert_eq!(cursor.position(SHARD), ShardPosition {
            verified: v(100),
            claim: Some(claim(u64::MAX, u64::MAX, Some(2))),
        });
        assert_eq!(resume_version(&mut cursor, &bound), v(u64::MAX));
        assert_eq!(resume_version(&mut cursor, &bound_at(3, 150)), v(u64::MAX));

        let expiring = bound_at(4, 200);
        assert!(expiring.disproves(SHARD, &cursor.position(SHARD).claim.unwrap()));
        assert_eq!(resume_version(&mut cursor, &expiring), v(100));
        assert_eq!(cursor.position(SHARD).claim, None);
    }

    #[test]
    fn a_poisoned_watermark_with_no_verified_checkpoint_expires_at_the_first() {
        let mut cursor = ShardCursor::genesis(NumPreshards::P4);
        let unverified = ShardStateVersionBound::default();
        cursor.observe(SHARD, v(u64::MAX), &unverified);
        assert_eq!(cursor.position(SHARD), ShardPosition {
            verified: v(0),
            claim: Some(claim(u64::MAX, u64::MAX, None)),
        });
        assert_eq!(resume_version(&mut cursor, &unverified), v(u64::MAX));

        assert_eq!(resume_version(&mut cursor, &bound_at(2, 100)), v(0));
    }

    /// A lying indexer claims a cursor just ahead of the network, which the network then overtakes. An update
    /// committed below the claim is never delivered while passes resume from it, so once the claim expires the shard is
    /// scanned again from the verified cursor, where the claim began.
    #[test]
    fn an_expired_claim_that_the_network_overtook_is_rescanned_from_the_verified_cursor() {
        const OWN_OUTPUT: u64 = 103;
        let mut cursor = cursor_at(100, None);
        let bound = bound_at(2, 100);
        cursor.observe(SHARD, v(105), &bound);

        let overtaken = bound_at(3, 200);
        assert!(resume_version(&mut cursor, &overtaken) > v(OWN_OUTPUT));
        cursor.observe(SHARD, v(250), &overtaken);
        assert!(resume_version(&mut cursor, &overtaken) > v(OWN_OUTPUT));

        let expiring = bound_at(4, 300);
        assert!(!expiring.disproves(SHARD, &cursor.position(SHARD).claim.unwrap()));
        assert!(resume_version(&mut cursor, &expiring) < v(OWN_OUTPUT));
        cursor.observe(SHARD, v(300), &expiring);
        assert_eq!(cursor.position(SHARD), ShardPosition {
            verified: v(300),
            claim: None,
        });
    }

    #[test]
    fn a_pass_resumed_from_a_claim_advances_the_claim_but_not_the_verified_cursor() {
        let mut cursor = cursor_at(100, Some(claim(120, 130, Some(2))));
        let bound = bound_at(3, 200);
        assert_eq!(resume_version(&mut cursor, &bound), v(130));

        cursor.observe(SHARD, v(180), &bound);
        assert_eq!(cursor.position(SHARD), ShardPosition {
            verified: v(100),
            claim: Some(claim(120, 180, Some(2))),
        });
        cursor.observe(SHARD, v(150), &bound);
        assert_eq!(cursor.get(SHARD), v(180));
    }

    #[test]
    fn a_watermark_at_or_below_the_verified_state_version_moves_the_verified_cursor() {
        let mut cursor = cursor_at(50, None);
        let bound = bound_at(2, 100);
        cursor.observe(SHARD, v(100), &bound);
        assert_eq!(cursor.position(SHARD), ShardPosition {
            verified: v(100),
            claim: None,
        });
    }

    #[test]
    fn a_verified_cursor_past_the_verified_state_version_is_lowered_to_it() {
        let mut cursor = cursor_at(u64::MAX, None);
        assert_eq!(resume_version(&mut cursor, &bound_at(2, 100)), v(100));
        assert_eq!(cursor.position(SHARD).verified, v(100));
    }

    #[test]
    fn a_claim_too_recent_for_the_verified_checkpoint_to_expire_is_resumed_from() {
        let mut cursor = cursor_at(100, Some(claim(105, 105, Some(2))));
        assert_eq!(resume_version(&mut cursor, &bound_at(3, 100)), v(105));
    }

    #[test]
    fn a_claim_at_or_below_the_verified_cursor_is_dropped() {
        let mut cursor = cursor_at(100, Some(claim(90, 100, Some(2))));
        assert_eq!(resume_version(&mut cursor, &bound_at(2, 100)), v(100));
        assert_eq!(cursor.position(SHARD).claim, None);
    }

    #[test]
    fn a_trusting_pass_drops_every_claim() {
        let mut cursor = cursor_at(100, Some(claim(u64::MAX, u64::MAX, Some(2))));
        assert_eq!(resume_version(&mut cursor, &ShardStateVersionBound::Unbounded), v(100));
        assert_eq!(cursor.position(SHARD).claim, None);
    }

    #[test]
    fn an_honest_claim_that_advanced_past_the_expiring_checkpoint_is_not_disproved() {
        let bound = bound_at(4, 100);
        let claim = claim(60, 130, Some(2));

        assert!(bound.expires(SHARD, &claim));
        assert!(!bound.disproves(SHARD, &claim));
    }

    #[test]
    fn a_claim_that_began_past_the_expiring_checkpoint_is_disproved() {
        let bound = bound_at(4, 100);

        assert!(bound.disproves(SHARD, &claim(101, 101, Some(2))));
    }

    #[test]
    fn a_claim_is_not_disproved_by_a_checkpoint_that_may_have_ended_before_it_began() {
        let bound = bound_at(3, 100);
        let claim = claim(101, 101, Some(2));

        assert!(!bound.expires(SHARD, &claim));
        assert!(!bound.disproves(SHARD, &claim));
    }

    #[test]
    fn a_claim_begun_without_a_verified_checkpoint_expires_without_being_disproved() {
        let bound = bound_at(4, 100);
        let claim = claim(101, 101, None);

        assert!(bound.expires(SHARD, &claim));
        assert!(!bound.disproves(SHARD, &claim));
    }

    #[test]
    fn a_cursor_round_trips_through_json() {
        let cursor = ShardCursor::from_iter([
            (SHARD, ShardPosition {
                verified: v(100),
                claim: Some(claim(u64::MAX, u64::MAX, Some(2))),
            }),
            (Shard::from_u32(2), ShardPosition {
                verified: v(7),
                claim: None,
            }),
        ]);
        let json = serde_json::to_string(&cursor).unwrap();
        assert_eq!(serde_json::from_str::<ShardCursor>(&json).unwrap(), cursor);
    }
}
