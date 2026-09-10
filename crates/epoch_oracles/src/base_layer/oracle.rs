//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, VecDeque},
    future::{Future, poll_fn},
    pin::Pin,
    task::{Context, Poll},
};

use log::*;
use ootle_network::Network;
use tari_base_node_client::{
    BaseNodeClient,
    BaseNodeClientError,
    futures_util::{StreamExt, TryStreamExt, stream},
    grpc::GrpcBaseNodeClient,
    types::{BaseLayerConsensusConstants, BaseLayerMetadata},
};
use tari_common_types::types::FixedHash;
use tari_epoch_manager::epoch_event_oracle::{EpochEvent, EpochEventOracle, ValidatorNodeChange};
use tari_ootle_common_types::{Epoch, displayable::Displayable, optional::Optional};
use tari_ootle_storage::global::BlockHeaderModel;
use tokio::time;

use crate::{
    base_layer::{BaseLayerBlockHeaderStore, BaseLayerEpochOracleConfig, header_hasher::hash_header},
    store::{EpochOracleStore, StoreKey},
};

const LOG_TARGET: &str = "tari::ootle::epoch_oracles::base_layer_scanner";

/// Maximum number of `get_validator_node_changes` requests issued concurrently after a header batch
/// has been streamed. These per-epoch fetches are independent of one another, so they are pipelined
/// rather than being awaited one at a time mid-stream (see `resolve_changes_and_commit`).
const VALIDATOR_NODE_CHANGE_FETCH_CONCURRENCY: usize = 10;

/// Maximum number of epoch-boundary headers fetched concurrently in the boundary-only scan path
/// (see `scan_epoch_boundaries`).
const BOUNDARY_HEADER_FETCH_CONCURRENCY: usize = 16;

/// Maximum number of epoch boundaries scanned per batch in the boundary-only path. Bounds memory and
/// the number of concurrent header fetches for very long catch-ups; the remainder is picked up by
/// the next scan round via the normal `has_more` loop.
const MAX_BOUNDARIES_PER_BATCH: u64 = 2_000;

/// An event produced while scanning a batch of base-layer blocks.
///
/// `ValidatorNodeChanges` is a placeholder for an `ActiveValidatorNodeSetChanged` event whose
/// `get_validator_node_changes` fetch is deferred until the whole batch has been scanned. Issuing
/// that unary RPC inside the scan loop stalls it on every epoch boundary; recording a placeholder
/// lets us resolve all the fetches concurrently afterwards while preserving the exact position the
/// event must occupy in the emitted order.
enum PendingScanEvent {
    Ready(EpochEvent),
    ValidatorNodeChanges { epoch: Epoch },
}

/// The in-memory result of scanning one batch of base-layer blocks, before the deferred
/// validator-node-change fetches are resolved and the scan position is committed
/// (see `resolve_changes_and_commit`).
struct BatchScan {
    scan_events: Vec<PendingScanEvent>,
    last_validator_node_mr: Option<FixedHash>,
    last_epoch_hash: Option<FixedHash>,
    last_scanned_hash: Option<FixedHash>,
    last_scanned_height: u64,
    scan_epoch: Epoch,
}

type TaskOutput<TStore, TClient> = (
    Result<bool, BaseLayerOracleError>,
    Box<BaseLayerOracleInner<TStore, TClient>>,
);

/// The in-flight blockchain scan future, boxed so it can be held across poll calls.
type ScanTask<TStore, TClient> = Pin<Box<dyn Future<Output = TaskOutput<TStore, TClient>> + Send>>;

/// `TClient` defaults to [`GrpcBaseNodeClient`] for production; tests substitute a mock base node
/// client so the scan/reorg logic can be driven without a live base node.
#[allow(clippy::struct_excessive_bools)]
pub struct BaseLayerOracle<TStore, TClient = GrpcBaseNodeClient> {
    inner: Option<Box<BaseLayerOracleInner<TStore, TClient>>>,
    is_initialized: bool,
    is_done: bool,
    has_more: bool,
    sleep_or_shutdown: bool,
    task: Option<ScanTask<TStore, TClient>>,
    sleep_task: Option<Pin<Box<time::Sleep>>>,
}

struct BaseLayerOracleInner<TStore, TClient> {
    config: BaseLayerEpochOracleConfig,
    store: TStore,
    last_scanned_height: u64,
    last_scanned_tip: Option<FixedHash>,
    last_scanned_hash: Option<FixedHash>,
    last_epoch_hash: Option<FixedHash>,
    last_scanned_validator_node_mr: Option<FixedHash>,
    base_node_client: TClient,
    has_attempted_scan: bool,
    pending_events: VecDeque<EpochEvent>,
    // Stores the minimal `BlockHeaderModel` (all `Copy` fields) rather than the full `BlockHeader`,
    // so the heavy parts of a header — notably `pow.pow_data` — are dropped right after hashing
    // instead of being held for the whole batch.
    header_buf: Vec<BlockHeaderModel>,
    network: Network,
    /// Epoch length in base-layer blocks, cached from consensus constants on the first scan
    /// that obtains them. `None` until the first scan completes.
    cached_epoch_length: Option<u64>,
}

impl<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + 'static, TClient: BaseNodeClient>
    BaseLayerOracle<TStore, TClient>
{
    pub fn new(store: TStore, base_node_client: TClient, config: BaseLayerEpochOracleConfig, network: Network) -> Self {
        Self {
            inner: Some(Box::new(BaseLayerOracleInner {
                config,
                store,
                last_scanned_tip: None,
                last_scanned_height: 0,
                last_scanned_hash: None,
                last_epoch_hash: None,
                last_scanned_validator_node_mr: None,
                base_node_client,
                has_attempted_scan: false,
                pending_events: VecDeque::new(),
                header_buf: Vec::new(),
                network,
                cached_epoch_length: None,
            })),
            is_initialized: false,
            is_done: false,
            has_more: false,
            sleep_or_shutdown: false,
            task: None,
            sleep_task: None,
        }
    }
}

impl<TStore: EpochOracleStore + BaseLayerBlockHeaderStore, TClient: BaseNodeClient>
    BaseLayerOracleInner<TStore, TClient>
{
    /// The configured start height adjusted for the height lag. This is the minimum height
    /// that the scanner will scan from.
    fn lag_start_height(&self) -> u64 {
        self.config.start_height.saturating_sub(self.config.height_lag)
    }

    /// The effective last scanned height, which is the maximum of the last scanned height and
    /// the lag start height. This ensures that we never scan below the lag start height.
    fn effective_last_scanned_height(&self) -> u64 {
        self.last_scanned_height.max(self.lag_start_height())
    }

    fn load_initial_state(&mut self) -> Result<(), BaseLayerOracleError> {
        self.last_scanned_tip = self
            .store
            .get(StoreKey::BaseLayerLastScannedTip.as_key_bytes())
            .map_err(BaseLayerOracleError::StoreError)?;
        self.last_scanned_hash = self
            .store
            .get(StoreKey::BaseLayerLastScannedBlockHash.as_key_bytes())
            .map_err(BaseLayerOracleError::StoreError)?;
        self.last_epoch_hash = self
            .store
            .get(StoreKey::BaseLayerLastEpochHash.as_key_bytes())
            .map_err(BaseLayerOracleError::StoreError)?;
        self.last_scanned_height = self
            .store
            .get(StoreKey::BaseLayerLastScannedBlockHeight.as_key_bytes())
            .map_err(BaseLayerOracleError::StoreError)?
            .unwrap_or(0);
        Ok(())
    }

    async fn scan_blockchain(&mut self, force_sync: bool) -> Result<bool, BaseLayerOracleError> {
        // fetch the new base layer info since the previous scan
        let tip = self.base_node_client.get_tip_info().await?;
        if force_sync {
            info!(
                target: LOG_TARGET,
                "⛓️ Forcing blockchain sync. We last scanned {}/{}",
                self.last_scanned_height,
                tip.height_of_longest_chain
                    .saturating_sub(self.config.height_lag)
            );
            self.has_attempted_scan = true;
            return self.sync_blockchain(tip).await;
        }

        match self.get_blockchain_progression(&tip).await? {
            BlockchainProgression::Progressed => {
                info!(
                    target: LOG_TARGET,
                    "⛓️ Blockchain has progressed to height {}. We last scanned {}/{}",
                    tip.height_of_longest_chain,
                    self.effective_last_scanned_height(),
                    tip.height_of_longest_chain
                        .saturating_sub(self.config.height_lag)
                );
                self.has_attempted_scan = true;
                self.sync_blockchain(tip).await
            },
            BlockchainProgression::Reorged => {
                warn!(
                    target: LOG_TARGET,
                    "⚠️ Base layer reorg detected at scanned height {}. Locating fork point.",
                    self.last_scanned_height
                );
                self.handle_reorg(&tip).await?;
                self.has_attempted_scan = true;
                self.sync_blockchain(tip).await
            },
            BlockchainProgression::NoProgress => {
                trace!(target: LOG_TARGET, "No new blocks to scan.");
                if !self.has_attempted_scan {
                    let constants = self
                        .base_node_client
                        .get_consensus_constants(tip.height_of_longest_chain)
                        .await?;
                    self.cached_epoch_length = Some(constants.epoch_length());
                    let lagged_height = tip.height_of_longest_chain.saturating_sub(self.config.height_lag);
                    let epoch = constants.height_to_epoch(lagged_height);
                    // If no progress has been made since restarting, we still need to tell the epoch manager that
                    // scanning is done
                    self.pending_events.push_back(EpochEvent::DoneForNow {
                        epoch,
                        epoch_hash: self.last_epoch_hash.unwrap_or_else(FixedHash::zero),
                    });
                }

                self.has_attempted_scan = true;
                Ok(false)
            },
        }
    }

    async fn get_blockchain_progression(
        &mut self,
        tip: &BaseLayerMetadata,
    ) -> Result<BlockchainProgression, BaseLayerOracleError> {
        if tip.height_of_longest_chain <= self.lag_start_height() {
            return Ok(BlockchainProgression::NoProgress);
        }
        // Short-circuit when the un-lagged tip is unchanged — nothing to do.
        if self.last_scanned_tip.as_ref() == Some(&tip.tip_hash) {
            return Ok(BlockchainProgression::NoProgress);
        }
        // Probe for reorgs using our deepest scanned block, NOT the un-lagged tip. The un-lagged
        // tip naturally gets displaced every block or two when a competing miner's tip is
        // orphaned — that lookup returns NotFound and declares "reorg" even though nothing at our
        // (lagged) scan depth has changed. Using last_scanned_hash restricts reorg detection to
        // changes that actually invalidate data we've stored.
        match &self.last_scanned_hash {
            Some(hash) => {
                let header = self.base_node_client.get_header_by_hash(hash).await.optional()?;
                if header.is_some() {
                    Ok(BlockchainProgression::Progressed)
                } else {
                    Ok(BlockchainProgression::Reorged)
                }
            },
            None => Ok(BlockchainProgression::Progressed),
        }
    }

    /// Recovers from a detected base-layer reorg by rewinding the scanner to the fork point.
    ///
    /// The fork point is the highest height whose canonical block we have already stored — everything
    /// above it was built on the orphaned chain. We delete those stale headers and rewind our scan
    /// position to the fork point, so the subsequent `sync_blockchain` re-fetches and stores the canonical
    /// headers in their place. The epoch database therefore converges on the correct chain rather than
    /// accumulating orphaned headers.
    ///
    /// We only attempt surgical recovery for reorgs within the confirmation depth (`height_lag`); a reorg
    /// deeper than that is treated as unrecoverable — we discard all stored headers and rescan from the
    /// start height. This matches the confirmation-depth guarantee (boundaries are only emitted once
    /// `height_lag`-buried) and bounds the candidate window to `height_lag` heights.
    ///
    /// The whole candidate window is fetched in a single `stream_headers` call (one round-trip rather than
    /// one per height) and the fork point is then located locally. "Canonical block at height `h` is in
    /// our store" is monotonic — true at and below the fork, false above it — so scanning the window from
    /// the top down stops at the fork.
    async fn handle_reorg(&mut self, tip: &BaseLayerMetadata) -> Result<(), BaseLayerOracleError> {
        let floor = self.lag_start_height();

        // Without persisted headers there is nothing in the epoch DB to repair; rewind to the start of
        // our range and let the normal scan re-emit events on the new chain.
        if !self.config.features.sync_headers {
            self.rewind_to_floor();
            return Ok(());
        }

        let constants = self
            .base_node_client
            .get_consensus_constants(tip.height_of_longest_chain)
            .await?;
        // Bound the lookup epoch by the tip epoch (never excludes a stored header, even if the L1
        // epoch_length constant changed since the header was stored).
        let max_epoch = constants.height_to_epoch(tip.height_of_longest_chain);

        // Candidate window is (search_floor, top]: the fork cannot be above the new canonical tip nor above
        // the deepest height we scanned, and we only recover within `height_lag` of our scan position.
        let top = self.last_scanned_height.min(tip.height_of_longest_chain);
        let search_floor = self
            .last_scanned_height
            .saturating_sub(self.config.height_lag)
            .max(floor);
        if top > search_floor {
            // Fetch the whole candidate window in one streamed request (the base node returns up to its
            // new tip, so a chain-shortening reorg simply yields fewer headers). `window_len` is bounded by
            // `height_lag`. Scoped so the stream (which borrows the base node client) is dropped before the
            // store lookups below.
            let window_len = top - search_floor;
            let canonical = {
                let mut stream = self
                    .base_node_client
                    .stream_headers(search_floor + 1, window_len)
                    .await?;
                let mut headers = Vec::with_capacity(window_len as usize);
                while let Some(header) = stream.try_next().await? {
                    headers.push(header);
                }
                headers
            };

            // Highest stored canonical block is the fork point (predicate is monotonic, so the first hit
            // from the top is the answer).
            for header in canonical.into_iter().rev() {
                let canonical_hash = hash_header(self.network, &header);
                if self
                    .store
                    .find_block_header_by_hash(max_epoch, &canonical_hash)
                    .map_err(BaseLayerOracleError::StoreError)?
                    .is_some()
                {
                    let fork_height = header.height;
                    let num_deleted = self
                        .store
                        .delete_block_headers_above(fork_height)
                        .map_err(BaseLayerOracleError::StoreError)?;
                    info!(
                        target: LOG_TARGET,
                        "⚓ Base layer fork point at height {fork_height} ({canonical_hash}). Deleted \
                         {num_deleted} stale header(s) above it; rewinding scanner."
                    );
                    self.last_scanned_height = fork_height;
                    self.last_scanned_hash = Some(canonical_hash);
                    self.last_scanned_validator_node_mr = Some(header.validator_node_mr);
                    self.reset_last_epoch_hash_to_boundary(constants.height_to_epoch(fork_height), &constants)?;
                    return Ok(());
                }
            }
        }

        // No common ancestor within the recoverable range: discard everything and rebuild from the start
        // height. The re-scan re-crosses (and re-emits) every epoch boundary above the floor, so the epoch
        // hashes self-correct.
        let num_deleted = self
            .store
            .delete_block_headers_above(floor)
            .map_err(BaseLayerOracleError::StoreError)?;
        warn!(
            target: LOG_TARGET,
            "⚠️ Base layer reorg deeper than the recoverable range (start height {floor}). Deleted {num_deleted} \
             header(s); rescanning from the start."
        );
        self.rewind_to_floor();
        Ok(())
    }

    /// After rewinding to a fork point in `fork_epoch`, points `last_epoch_hash` back at the boundary block
    /// that opened that epoch. Boundaries at or below the fork point are canonical, so this discards any
    /// orphaned boundary hash carried over from a chain-shortening reorg — the re-scan resumes *above* the
    /// fork point and so never re-crosses the fork epoch's own boundary to correct it.
    ///
    /// If that boundary block sits below our scan range (so we never emitted it), there is no stored hash
    /// to restore. We clear `last_epoch_hash` rather than leave it: in that case it can only hold a
    /// boundary from an epoch *above* the fork epoch (the fork epoch's own boundary is below the floor and
    /// was never crossed), which a chain-shortening reorg has orphaned. `None` is the same "unknown"
    /// sentinel used at startup, and the re-scan re-populates it on the next boundary it crosses.
    fn reset_last_epoch_hash_to_boundary(
        &mut self,
        fork_epoch: Epoch,
        constants: &BaseLayerConsensusConstants,
    ) -> Result<(), BaseLayerOracleError> {
        let boundary_height = fork_epoch.as_u64().saturating_mul(constants.epoch_length());
        match self
            .store
            .get_first_block_header_in_epoch(fork_epoch)
            .map_err(BaseLayerOracleError::StoreError)?
        {
            Some(boundary) if boundary.height == boundary_height => {
                self.last_epoch_hash = Some(boundary.block_hash);
            },
            _ => {
                debug!(
                    target: LOG_TARGET,
                    "Fork epoch {fork_epoch} boundary block is not within the scan range; clearing last_epoch_hash"
                );
                self.last_epoch_hash = None;
            },
        }
        Ok(())
    }

    /// Resets the in-memory scan position to the start of our scan range, as on a fresh start. The
    /// persisted position is corrected by the next successful `set_last_scanned_block`. `last_epoch_hash`
    /// is cleared too so a reorg that orphaned the last boundary we crossed does not leave a stale hash;
    /// the re-scan re-emits every boundary above the floor and re-populates it.
    fn rewind_to_floor(&mut self) {
        self.last_scanned_height = self.lag_start_height();
        self.last_scanned_hash = None;
        self.last_scanned_validator_node_mr = None;
        self.last_epoch_hash = None;
    }

    #[allow(clippy::too_many_lines)]
    async fn sync_blockchain(&mut self, tip: BaseLayerMetadata) -> Result<bool, BaseLayerOracleError> {
        let Some(lag_tip_height) = tip.height_of_longest_chain.checked_sub(self.config.height_lag) else {
            debug!(
                target: LOG_TARGET,
                "Base layer blockchain is not yet at the required height to start scanning it"
            );
            return Ok(false);
        };
        let start_scan_height = self.effective_last_scanned_height() + 1;
        if lag_tip_height < start_scan_height {
            info!(
                target: LOG_TARGET,
                "⛓️ Base layer blockchain has not progressed beyond the start scan height {}. Lagged tip height is {}.",
                start_scan_height,
                lag_tip_height
            );
            return Ok(false);
        }

        let Some(num_blocks) = lag_tip_height.checked_sub(start_scan_height - 1) else {
            debug!(
                target: LOG_TARGET,
                "Base layer blockchain has not progressed beyond the last scanned height {} (lagged tip height \
                 {}).",
                start_scan_height,
                lag_tip_height
            );
            return Ok(false);
        };

        // Recover the last scanned validator node MR if it is not set yet, i.e the node has scanned BL blocks
        // previously.
        if self.last_scanned_validator_node_mr.is_none() &&
            let Some(ref hash) = self.last_scanned_hash
        {
            let header = self.base_node_client.get_header_by_hash(hash).await?;
            self.last_scanned_validator_node_mr = Some(header.validator_node_mr);
            // cached_header = Some(header);
        }

        let constants = self
            .base_node_client
            .get_consensus_constants(tip.height_of_longest_chain)
            .await?;
        self.cached_epoch_length = Some(constants.epoch_length());

        // Acquire and scan this batch's blocks. The validator node needs every header (e.g. for
        // burn-proof validation), so it streams and stores the full contiguous range. The indexer
        // only needs epoch boundaries (and the validator-node MR sampled at them), so it fetches
        // just the boundary headers — skipping the bulk of the range and the per-header stream cost.
        let batch = if self.config.features.sync_headers {
            self.scan_full_headers(&constants, start_scan_height, num_blocks)
                .await?
        } else {
            self.scan_epoch_boundaries(&constants, start_scan_height, num_blocks)
                .await?
        };

        let scan_epoch = batch.scan_epoch;
        self.resolve_changes_and_commit(batch, &tip).await?;

        // let scan_height = constants.epoch_to_height(scan_epoch);
        // if self.last_scanned_validator_node_mr.is_none() {
        //     // Initial validator node download
        //     let mut stream = self.base_node_client.get_validator_nodes(scan_height).await?;
        //     while let Some(node) = stream.try_next().await? {
        //         self.pending_events
        //             .push_back(EpochEvent::ActiveValidatorNodeSetChanged {
        //                 epoch: scan_epoch,
        //                 node_changes: vec![ValidatorNodeChange::Add {
        //                     claim_public_key: node.public_key,
        //                     validator_node_public_key: node.public_key,
        //                     activation_epoch: scan_epoch,
        //                     minimum_value_promise: 0,
        //                     shard_key: node.shard_key,
        //                 }],
        //             });
        //     }
        // }

        if self.last_scanned_height < lag_tip_height {
            info!(
                target: LOG_TARGET,
                "⛓️ Completed scanning up to height {}. Continuing scan to catch up to lagged tip height {}.",
                self.last_scanned_height,
                lag_tip_height
            );
            return Ok(true);
        }

        let latest = self.base_node_client.get_tip_info().await?;
        let lagged_height = latest.height_of_longest_chain.saturating_sub(self.config.height_lag);
        if lagged_height > lag_tip_height {
            info!(
                target: LOG_TARGET,
                "Base layer blockchain has progressed since the last scan. Last scanned block height: {}. \
                 Latest block height: {}",
                lag_tip_height,
                latest.height_of_longest_chain
            );
            Ok(true)
        } else {
            self.pending_events.push_back(EpochEvent::DoneForNow {
                epoch: scan_epoch,
                epoch_hash: self.last_epoch_hash.unwrap_or_else(FixedHash::zero),
            });
            Ok(false)
        }
    }

    /// Scans a contiguous batch of up to 1000 headers, buffering every header for storage. Used when
    /// `sync_headers` is enabled (validator node), which needs the full header set (e.g. for
    /// burn-proof validation).
    ///
    /// The single network call this loop used to make — `get_validator_node_changes` — is deferred:
    /// a `ValidatorNodeChanges` placeholder is recorded at the exact position its
    /// `ActiveValidatorNodeSetChanged` event must occupy, and the fetch is resolved later in
    /// [`resolve_changes_and_commit`] so it never stalls the header server-stream.
    #[allow(clippy::too_many_lines)]
    async fn scan_full_headers(
        &mut self,
        constants: &BaseLayerConsensusConstants,
        start_scan_height: u64,
        num_blocks: u64,
    ) -> Result<BatchScan, BaseLayerOracleError> {
        // We'll buffer 1000 headers at a time
        // note: 10_000 is the maximum permitted by the base node gRPC service
        let limit = 1_000.min(num_blocks);

        let mut base_node_client = self.base_node_client.clone();
        info!(
            target: LOG_TARGET,
            "⛓️Starting header stream from {}-{}", start_scan_height, start_scan_height + limit
        );
        let mut header_stream = base_node_client.stream_headers(start_scan_height, limit).await?;

        if let Some(additional) = usize::try_from(limit)
            .ok()
            .and_then(|l| l.checked_sub(self.header_buf.capacity()))
        {
            self.header_buf.reserve(additional);
        }

        // Scan state evolves in locals here and is only committed to `self`/the store once the
        // deferred fetches succeed. This makes the batch atomic: a failed fetch leaves the persisted
        // scan position untouched, so the whole batch is safely re-scanned next round (header inserts
        // are idempotent). `header_buf` is cleared so a re-scan after such a failure does not
        // accumulate the previous attempt's (unflushed) headers.
        self.header_buf.clear();
        let mut scan_events: Vec<PendingScanEvent> = Vec::new();
        let mut last_validator_node_mr = self.last_scanned_validator_node_mr;
        let mut last_epoch_hash = self.last_epoch_hash;
        let mut last_scanned_hash = self.last_scanned_hash;
        let mut last_scanned_height = self.last_scanned_height;
        let mut scan_epoch = constants.height_to_epoch(start_scan_height);
        while let Some(header) = header_stream.try_next().await? {
            let current_epoch = constants.height_to_epoch(header.height);
            let header_height = header.height;
            // Note: Cant use header.hash() because it uses CURRENT_NETWORK global
            let header_hash = hash_header(self.network, &header);
            let current_validator_node_mr = header.validator_node_mr;
            // Keep only the fields the store persists; the full `header` (incl. its pow_data blob)
            // is dropped at the end of this iteration.
            self.header_buf.push(BlockHeaderModel {
                epoch: current_epoch,
                height: header_height,
                block_hash: header_hash,
                kernel_merkle_root: header.kernel_mr,
                validator_node_merkle_root: header.validator_node_mr,
            });

            // Record validator node MR changes BEFORE epoch changes so that new registrations are
            // ordered ahead of the boundary's EpochChanged — assign_validators_for_epoch must see
            // them first. This matters when the base layer updates the validator_node_mr at epoch
            // boundary blocks.
            if last_validator_node_mr != Some(current_validator_node_mr) {
                debug!(
                    target: LOG_TARGET,
                    "⛓️ last_scanned_validator_node_mr = {} current = {}", last_validator_node_mr.display(), current_validator_node_mr
                );

                if self.config.features.sync_validator_node_changes {
                    // Deferred: the actual fetch happens in resolve_changes_and_commit. Here we only
                    // mark where the resulting event belongs in the emitted order.
                    scan_events.push(PendingScanEvent::ValidatorNodeChanges { epoch: current_epoch });
                }
                last_validator_node_mr = Some(current_validator_node_mr);
            }

            if header_height % constants.epoch_length() == 0 {
                info!(
                    target: LOG_TARGET,
                    "🟩 New epoch block {} {} {}", current_epoch, header_height, header_hash
                );
                last_epoch_hash = Some(header_hash);

                // Emit EpochChanged for every boundary so every (epoch, epoch_hash) pair is persisted
                // by the epoch manager. Consensus's get_epoch_hash(epoch) lookup depends on that
                // row being present for any epoch it may transition into — including epochs
                // crossed during a long catch-up.
                info!(
                    target: LOG_TARGET,
                    "🟩 epoch change {}->{} (height({}) hash({}))", scan_epoch, current_epoch, header_height, header_hash
                );
                scan_events.push(PendingScanEvent::Ready(EpochEvent::EpochChanged {
                    epoch: current_epoch,
                    // Set above
                    epoch_hash: last_epoch_hash.unwrap_or_default(),
                }));
                scan_epoch = current_epoch;
            }

            last_scanned_hash = Some(header_hash);
            last_scanned_height = header_height;
        }
        // End the header server-stream before the deferred fetches issue their own requests.
        drop(header_stream);

        Ok(BatchScan {
            scan_events,
            last_validator_node_mr,
            last_epoch_hash,
            last_scanned_hash,
            last_scanned_height,
            scan_epoch,
        })
    }

    /// Scans only the epoch-boundary headers in the next batch window, fetching them concurrently
    /// instead of streaming every header. Used when `sync_headers` is disabled (indexer): only epoch
    /// boundaries — and the validator-node MR sampled at them — are needed, so the non-boundary
    /// headers (the bulk of the range, and the per-header stream-delivery cost) are never fetched.
    ///
    /// Relies on the base-layer invariant that `validator_node_mr` only changes at epoch-boundary
    /// blocks, so sampling the MR at boundaries catches every validator-set change. The boundary's
    /// epoch then drives the same deferred `get_validator_node_changes` fetch as the full path. A
    /// trailing position-only header (the window end) advances the scan position when the window
    /// ends mid-epoch; it carries no events and is not MR-sampled (its MR equals its epoch
    /// boundary's, already handled).
    async fn scan_epoch_boundaries(
        &mut self,
        constants: &BaseLayerConsensusConstants,
        start_scan_height: u64,
        num_blocks: u64,
    ) -> Result<BatchScan, BaseLayerOracleError> {
        let epoch_length = constants.epoch_length().max(1);
        // Bound the window so very long catch-ups split across batches via the has_more loop.
        let limit = MAX_BOUNDARIES_PER_BATCH.saturating_mul(epoch_length).min(num_blocks);
        let window_end = start_scan_height + limit - 1;

        // Every epoch boundary in [start_scan_height, window_end], plus window_end itself so the scan
        // position (and its reorg-detection hash) advances to the window even when it ends mid-epoch.
        let first_boundary = start_scan_height.div_ceil(epoch_length).saturating_mul(epoch_length);
        let mut fetch_heights: Vec<u64> = (first_boundary..=window_end).step_by(epoch_length as usize).collect();
        if fetch_heights.last() != Some(&window_end) {
            fetch_heights.push(window_end);
        }

        info!(
            target: LOG_TARGET,
            "⛓️ Fetching {} epoch-boundary header(s) in {}-{}", fetch_heights.len(), start_scan_height, window_end
        );

        // Fetch the targeted headers concurrently; they are independent. Each task extracts only the
        // hash and validator-node MR the scan needs, so the full header (incl. its pow_data blob) is
        // dropped immediately rather than collecting up to MAX_BOUNDARIES_PER_BATCH of them.
        let base_client = self.base_node_client.clone();
        let network = self.network;
        let mut samples: Vec<(u64, FixedHash, FixedHash)> = stream::iter(fetch_heights)
            .map(move |height| {
                let mut client = base_client.clone();
                async move {
                    let mut header_stream = client.stream_headers(height, 1).await?;
                    let header = header_stream.try_next().await?.ok_or_else(|| {
                        BaseLayerOracleError::InvalidBaseNodeResponse(format!(
                            "base node returned no header at height {height}"
                        ))
                    })?;
                    let header_hash = hash_header(network, &header);
                    Ok::<_, BaseLayerOracleError>((height, header_hash, header.validator_node_mr))
                }
            })
            .buffer_unordered(BOUNDARY_HEADER_FETCH_CONCURRENCY)
            .try_collect()
            .await?;
        samples.sort_unstable_by_key(|(height, _, _)| *height);

        let mut scan_events: Vec<PendingScanEvent> = Vec::new();
        let mut last_validator_node_mr = self.last_scanned_validator_node_mr;
        let mut last_epoch_hash = self.last_epoch_hash;
        let mut last_scanned_hash = self.last_scanned_hash;
        let mut last_scanned_height = self.last_scanned_height;
        let mut scan_epoch = constants.height_to_epoch(start_scan_height);
        for (height, header_hash, validator_node_mr) in samples {
            // Only boundary headers carry epoch changes / MR samples. A non-boundary sample here is
            // the trailing window-end marker, used solely to advance the scan position.
            if height % epoch_length == 0 {
                let current_epoch = constants.height_to_epoch(height);
                if last_validator_node_mr != Some(validator_node_mr) {
                    if self.config.features.sync_validator_node_changes {
                        scan_events.push(PendingScanEvent::ValidatorNodeChanges { epoch: current_epoch });
                    }
                    last_validator_node_mr = Some(validator_node_mr);
                }
                last_epoch_hash = Some(header_hash);
                info!(
                    target: LOG_TARGET,
                    "🟩 epoch change {}->{} (height({}) hash({}))", scan_epoch, current_epoch, height, header_hash
                );
                scan_events.push(PendingScanEvent::Ready(EpochEvent::EpochChanged {
                    epoch: current_epoch,
                    epoch_hash: header_hash,
                }));
                scan_epoch = current_epoch;
            }

            last_scanned_hash = Some(header_hash);
            last_scanned_height = height;
        }

        Ok(BatchScan {
            scan_events,
            last_validator_node_mr,
            last_epoch_hash,
            last_scanned_hash,
            last_scanned_height,
            scan_epoch,
        })
    }

    /// Resolves the batch's deferred validator-node-change fetches concurrently, emits the batch's
    /// events in order, persists any buffered headers, and advances the scan position. Shared by the
    /// full and boundary scan paths.
    async fn resolve_changes_and_commit(
        &mut self,
        batch: BatchScan,
        tip: &BaseLayerMetadata,
    ) -> Result<(), BaseLayerOracleError> {
        let BatchScan {
            scan_events,
            last_validator_node_mr,
            last_epoch_hash,
            last_scanned_hash,
            last_scanned_height,
            scan_epoch: _,
        } = batch;

        // Resolve the deferred fetches concurrently. Each is keyed only by epoch (sidechain_id is
        // fixed for the whole scan) and is independent of the rest, so they are pipelined with
        // bounded concurrency instead of being awaited serially.
        let fetch_targets: Vec<(usize, Epoch)> = scan_events
            .iter()
            .enumerate()
            .filter_map(|(idx, event)| match event {
                PendingScanEvent::ValidatorNodeChanges { epoch } => Some((idx, *epoch)),
                PendingScanEvent::Ready(_) => None,
            })
            .collect();

        let mut fetched_changes: HashMap<usize, Vec<ValidatorNodeChange>> = HashMap::new();
        if !fetch_targets.is_empty() {
            let base_client = self.base_node_client.clone();
            let sidechain_id = self.config.sidechain_id;
            fetched_changes = stream::iter(fetch_targets)
                .map(move |(idx, epoch)| {
                    let mut client = base_client.clone();
                    async move {
                        let node_changes = client
                            .get_validator_node_changes(epoch, sidechain_id.as_ref())
                            .await
                            .map_err(BaseLayerOracleError::BaseNodeError)?;
                        let node_changes = node_changes
                            .into_iter()
                            .map(TryInto::try_into)
                            .collect::<Result<Vec<ValidatorNodeChange>, _>>()
                            .map_err(|e| {
                                BaseLayerOracleError::InvalidBaseNodeResponse(format!(
                                    "Failed to convert validator node change: {}",
                                    e
                                ))
                            })?;
                        Ok::<_, BaseLayerOracleError>((idx, node_changes))
                    }
                })
                .buffer_unordered(VALIDATOR_NODE_CHANGE_FETCH_CONCURRENCY)
                .try_collect()
                .await?;
        }

        // Replay the batch's events in order, splicing in each resolved validator-node-change set. A
        // set may be empty when the MR changed because of *other* sidechains — drop those, as the
        // inline version did.
        for (idx, event) in scan_events.into_iter().enumerate() {
            match event {
                PendingScanEvent::Ready(event) => self.pending_events.push_back(event),
                PendingScanEvent::ValidatorNodeChanges { epoch } => {
                    if let Some(node_changes) = fetched_changes.remove(&idx) &&
                        !node_changes.is_empty()
                    {
                        self.pending_events
                            .push_back(EpochEvent::ActiveValidatorNodeSetChanged { epoch, node_changes });
                    }
                },
            }
        }

        // Persist headers and advance the scan position now the batch is complete. `last_scanned_tip`
        // is set by set_last_scanned_block once the whole batch is persisted; it is intentionally not
        // advanced earlier, so an interrupted sync re-scans rather than letting
        // get_blockchain_progression short-circuit and silently skip the remaining headers.
        if !self.header_buf.is_empty() {
            self.store
                .add_block_headers(self.header_buf.drain(..))
                .map_err(BaseLayerOracleError::StoreError)?;
        }
        self.last_scanned_validator_node_mr = last_validator_node_mr;
        self.set_last_scanned_block(
            last_scanned_hash.unwrap_or_else(FixedHash::zero),
            last_scanned_height,
            tip.tip_hash,
        )?;

        if let Some(hash) = last_epoch_hash {
            self.set_last_epoch_block(hash)?;
        }

        Ok(())
    }

    fn set_last_scanned_block(
        &mut self,
        header_hash: FixedHash,
        height: u64,
        tip: FixedHash,
    ) -> Result<(), BaseLayerOracleError> {
        self.store
            .set(StoreKey::BaseLayerLastScannedTip.as_key_bytes(), &tip)
            .map_err(BaseLayerOracleError::StoreError)?;
        self.store
            .set(StoreKey::BaseLayerLastScannedBlockHash.as_key_bytes(), &header_hash)
            .map_err(BaseLayerOracleError::StoreError)?;
        self.store
            .set(StoreKey::BaseLayerLastScannedBlockHeight.as_key_bytes(), &height)
            .map_err(BaseLayerOracleError::StoreError)?;
        self.last_scanned_tip = Some(tip);
        self.last_scanned_hash = Some(header_hash);
        self.last_scanned_height = height;
        Ok(())
    }

    fn set_last_epoch_block(&mut self, hash: FixedHash) -> Result<(), BaseLayerOracleError> {
        self.store
            .set(StoreKey::BaseLayerLastEpochHash.as_key_bytes(), &hash)
            .map_err(BaseLayerOracleError::StoreError)?;
        self.last_epoch_hash = Some(hash);
        Ok(())
    }

    fn shrink_events(&mut self) {
        const CAP_TO_SHRINK: usize = 1000;
        const TARGET_CAP: usize = 500;
        if self.pending_events.capacity() >= CAP_TO_SHRINK {
            self.pending_events.shrink_to(TARGET_CAP);
        }
    }

    /// Returns the boundary block hash we have stored for `epoch`, or `None` if we have not scanned
    /// it. The stored header only counts as the boundary when its height is exactly the epoch's first
    /// height: a scan range that starts mid-epoch stores a first header that is not the boundary, and
    /// ratifying against that would compare the wrong hash.
    fn observed_epoch_boundary_hash(&self, epoch: Epoch) -> anyhow::Result<Option<FixedHash>> {
        // The epoch length is only known once a scan has obtained the L1 constants; until then we
        // cannot say which height opens the epoch.
        let Some(epoch_length) = self.cached_epoch_length else {
            return Ok(None);
        };
        let Some(boundary_height) = epoch.as_u64().checked_mul(epoch_length) else {
            return Ok(None);
        };
        let Some(boundary) = self.store.get_first_block_header_in_epoch(epoch)? else {
            return Ok(None);
        };
        Ok((boundary.height == boundary_height).then_some(boundary.block_hash))
    }

    /// Returns true when our lagged scanner position is within `epoch_end_spread_blocks` of the
    /// next epoch boundary. Used by consensus to accept `EndEpoch` proposals speculatively when
    /// peers' oracles have already crossed and ours is almost there.
    fn is_within_epoch_end_spread(&self, current_epoch: Epoch) -> bool {
        if self.config.epoch_end_spread_blocks == 0 {
            return false;
        }
        let Some(epoch_length) = self.cached_epoch_length else {
            return false;
        };
        if epoch_length == 0 {
            return false;
        }
        let Some(next_start) = current_epoch
            .as_u64()
            .checked_add(1)
            .and_then(|e| e.checked_mul(epoch_length))
        else {
            return false;
        };
        self.last_scanned_height
            .saturating_add(self.config.epoch_end_spread_blocks) >=
            next_start
    }
}

impl<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Send + 'static, TClient: BaseNodeClient + 'static>
    BaseLayerOracle<TStore, TClient>
{
    fn poll_next_event(&mut self, cx: &mut Context) -> Poll<Option<EpochEvent>> {
        if self.is_done {
            return Poll::Ready(None);
        }

        if !self.is_initialized {
            match self
                .inner
                .as_mut()
                .expect("inner must be Some if not initialized")
                .load_initial_state()
            {
                Ok(()) => {
                    trace!(target: LOG_TARGET, "Initialized");
                    self.is_initialized = true;
                },
                Err(err) => {
                    self.is_done = true;
                    return Poll::Ready(Some(EpochEvent::error(err)));
                },
            }
        }

        loop {
            if let Some(inner_mut) = self.inner.as_mut() {
                if let Some(event) = inner_mut.pending_events.pop_front() {
                    trace!(target: LOG_TARGET, "Pop event {event}");
                    return Poll::Ready(Some(event));
                }
                inner_mut.shrink_events();
            }

            if let Some(mut task) = self.task.take() {
                trace!(target: LOG_TARGET, "Work on sync");
                match task.as_mut().poll(cx) {
                    Poll::Ready((Ok(has_more), inner)) => {
                        trace!(target: LOG_TARGET, "Sync complete Ok");
                        self.inner = Some(inner);
                        self.has_more = has_more;
                        trace!(target: LOG_TARGET, "has_more={has_more}");
                        // There may be events to return, do that straight away
                        continue;
                    },
                    Poll::Ready((Err(err), inner)) => {
                        debug!(target: LOG_TARGET, "Sync complete Err({err})");
                        self.inner = Some(inner);
                        return Poll::Ready(Some(EpochEvent::error(err)));
                    },
                    Poll::Pending => {
                        self.task = Some(task);
                        return Poll::Pending;
                    },
                }
            }

            // On the first call of this method, sleep_or_shutdown is false and scanning will immediately begin
            if !self.has_more && self.sleep_or_shutdown && self.sleep_task.is_none() {
                let scanning_interval = self
                    .inner
                    .as_ref()
                    .expect("inner must be Some when starting sleep task")
                    .config
                    .scanning_interval;
                trace!(target: LOG_TARGET, "Starting sleep task {:?}", scanning_interval);
                self.sleep_task = Some(Box::pin(time::sleep(scanning_interval)));
            }

            if let Some(sleep) = self.sleep_task.as_mut() {
                trace!(target: LOG_TARGET, "Sleep poll");
                if sleep.as_mut().poll(cx).is_pending() {
                    return Poll::Pending;
                }
            }
            self.sleep_task = None;

            let mut inner = self.inner.take().expect("inner is None");
            let has_more = self.has_more;
            self.task = Some(Box::pin(async move {
                trace!(target: LOG_TARGET, "Starting blockchain scan task");
                let result = inner.scan_blockchain(has_more).await;
                (result, inner)
            }));

            self.sleep_or_shutdown = true;
        }
    }
}

impl<TStore: EpochOracleStore + BaseLayerBlockHeaderStore + Send + 'static, TClient: BaseNodeClient + 'static>
    EpochEventOracle for BaseLayerOracle<TStore, TClient>
{
    /// Returns a Future that returns the next event, completing a round of scanning if necessary.
    /// This Future is cancel-safe. Returns None if a shutdown is triggered.
    async fn next_epoch_event(&mut self) -> Option<EpochEvent> {
        poll_fn(|cx| self.poll_next_event(cx)).await
    }

    fn is_within_epoch_end_spread(&self, current_epoch: Epoch) -> bool {
        // `inner` is briefly None while a scan task is in flight; return false then so voting
        // falls back to the strict em_epoch > current_epoch check.
        self.inner
            .as_deref()
            .map(|inner| inner.is_within_epoch_end_spread(current_epoch))
            .unwrap_or(false)
    }

    fn observed_epoch_boundary_hash(&self, epoch: Epoch) -> anyhow::Result<Option<FixedHash>> {
        // `inner` is briefly None while a scan task is in flight; the voter then falls back to the
        // activated epoch hash alone.
        match self.inner.as_deref() {
            Some(inner) => inner.observed_epoch_boundary_hash(epoch),
            None => Ok(None),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BaseLayerOracleError {
    #[error("Store error: {0}")]
    StoreError(anyhow::Error),
    #[error("Base node client error: {0}")]
    BaseNodeError(#[from] BaseNodeClientError),
    #[error("Invalid base node response: {0}")]
    InvalidBaseNodeResponse(String),
}

enum BlockchainProgression {
    /// The blockchain has progressed since the last scan
    Progressed,
    /// Reorg was detected
    Reorged,
    /// The blockchain has not progressed since the last scan
    NoProgress,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        sync::{Arc, Mutex},
        time::Duration,
    };

    use ootle_network::Network;
    use tari_base_node_client::{
        BaseNodeClient,
        BaseNodeClientError,
        ValidatorNodeChange,
        futures_util::stream::{self, Stream},
        tonic,
        types::{BaseLayerConsensusConstants, BaseLayerMetadata, BaseLayerValidatorNode, SideChainUtxos},
    };
    use tari_common_types::types::FixedHash;
    use tari_epoch_manager::epoch_event_oracle::EpochEvent;
    use tari_node_components::blocks::BlockHeader;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_storage::global::BlockHeaderModel;
    use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;
    use tari_transaction_components::{tari_amount::MicroMinotari, transaction_components::CodeTemplateRegistration};

    use super::{BaseLayerOracleInner, hash_header};
    use crate::{
        base_layer::{
            BaseLayerBlockHeaderStore,
            config::{BaseLayerEpochOracleConfig, BaseLayerEpochOracleFeatures},
        },
        store::EpochOracleStore,
    };

    const NETWORK: Network = Network::LocalNet;
    const EPOCH_LENGTH: u64 = 5;

    /// In-memory implementation of the oracle's store traits. Cloneable: the test holds one handle for
    /// assertions while the oracle owns another.
    #[derive(Clone, Default)]
    struct InMemoryStore {
        metadata: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
        headers: Arc<Mutex<Vec<BlockHeaderModel>>>,
    }

    impl InMemoryStore {
        /// Sorted list of stored header heights.
        fn heights(&self) -> Vec<u64> {
            let mut heights: Vec<u64> = self.headers.lock().unwrap().iter().map(|m| m.height).collect();
            heights.sort_unstable();
            heights
        }

        fn hash_at(&self, height: u64) -> Option<FixedHash> {
            self.headers
                .lock()
                .unwrap()
                .iter()
                .find(|m| m.height == height)
                .map(|m| m.block_hash)
        }
    }

    impl EpochOracleStore for InMemoryStore {
        fn get<T: serde::de::DeserializeOwned>(&self, key: &[u8]) -> anyhow::Result<Option<T>> {
            match self.metadata.lock().unwrap().get(key) {
                Some(bytes) => Ok(Some(serde_json::from_slice(bytes)?)),
                None => Ok(None),
            }
        }

        fn set<T: serde::Serialize>(&self, key: &[u8], value: &T) -> anyhow::Result<()> {
            self.metadata
                .lock()
                .unwrap()
                .insert(key.to_vec(), serde_json::to_vec(value)?);
            Ok(())
        }
    }

    impl BaseLayerBlockHeaderStore for InMemoryStore {
        fn add_block_headers<I: IntoIterator<Item = BlockHeaderModel>>(&self, headers: I) -> anyhow::Result<()> {
            let mut stored = self.headers.lock().unwrap();
            for header in headers {
                // Mirror the SQL on_conflict(block_hash, epoch).do_nothing() idempotency.
                if stored
                    .iter()
                    .any(|m| m.block_hash == header.block_hash && m.epoch == header.epoch)
                {
                    continue;
                }
                stored.push(header);
            }
            Ok(())
        }

        fn find_block_header_by_hash(
            &self,
            max_epoch: Epoch,
            block_hash: &FixedHash,
        ) -> anyhow::Result<Option<BlockHeaderModel>> {
            Ok(self
                .headers
                .lock()
                .unwrap()
                .iter()
                .find(|m| m.block_hash == *block_hash && m.epoch <= max_epoch)
                .cloned())
        }

        fn get_first_block_header_in_epoch(&self, epoch: Epoch) -> anyhow::Result<Option<BlockHeaderModel>> {
            Ok(self
                .headers
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m.epoch == epoch)
                .min_by_key(|m| m.height)
                .cloned())
        }

        fn delete_block_headers_above(&self, height: u64) -> anyhow::Result<usize> {
            let mut stored = self.headers.lock().unwrap();
            let before = stored.len();
            stored.retain(|m| m.height <= height);
            Ok(before - stored.len())
        }
    }

    /// Mock base node serving a configurable canonical chain (indexed by height). Swapping the chain
    /// simulates a reorg.
    #[derive(Clone)]
    struct MockBaseNode {
        chain: Arc<Mutex<Vec<BlockHeader>>>,
        /// Validator-node changes served per epoch by `get_validator_node_changes`. Empty by default.
        vn_changes: Arc<Mutex<HashMap<u64, Vec<ValidatorNodeChange>>>>,
        /// Epochs `get_validator_node_changes` was called with. The deferred fetches run concurrently,
        /// so the recorded order is non-deterministic — assert against the sorted set.
        vn_calls: Arc<Mutex<Vec<u64>>>,
        /// When true, `get_validator_node_changes` returns an error instead of data.
        vn_fail: Arc<Mutex<bool>>,
    }

    impl MockBaseNode {
        fn new(chain: Vec<BlockHeader>) -> Self {
            Self {
                chain: Arc::new(Mutex::new(chain)),
                vn_changes: Arc::new(Mutex::new(HashMap::new())),
                vn_calls: Arc::new(Mutex::new(Vec::new())),
                vn_fail: Arc::new(Mutex::new(false)),
            }
        }

        fn set_chain(&self, chain: Vec<BlockHeader>) {
            *self.chain.lock().unwrap() = chain;
        }

        fn set_vn_changes(&self, changes: HashMap<u64, Vec<ValidatorNodeChange>>) {
            *self.vn_changes.lock().unwrap() = changes;
        }

        fn fail_vn_changes(&self) {
            *self.vn_fail.lock().unwrap() = true;
        }

        fn vn_call_epochs_sorted(&self) -> Vec<u64> {
            let mut calls = self.vn_calls.lock().unwrap().clone();
            calls.sort_unstable();
            calls
        }
    }

    impl BaseNodeClient for MockBaseNode {
        async fn test_connection(&mut self) -> Result<(), BaseNodeClientError> {
            Ok(())
        }

        async fn get_network(&mut self) -> Result<u8, BaseNodeClientError> {
            Ok(NETWORK.as_byte())
        }

        async fn get_tip_info(&mut self) -> Result<BaseLayerMetadata, BaseNodeClientError> {
            let chain = self.chain.lock().unwrap();
            let tip = chain.last().expect("chain must not be empty");
            Ok(BaseLayerMetadata {
                height_of_longest_chain: tip.height,
                tip_hash: hash_header(NETWORK, tip),
            })
        }

        async fn get_validator_node_changes(
            &mut self,
            epoch: Epoch,
            _sidechain_id: Option<&RistrettoPublicKeyBytes>,
        ) -> Result<Vec<ValidatorNodeChange>, BaseNodeClientError> {
            self.vn_calls.lock().unwrap().push(epoch.as_u64());
            if *self.vn_fail.lock().unwrap() {
                return Err(BaseNodeClientError::GrpcStatus {
                    code: tonic::Code::Internal,
                    message: "injected validator node change failure".to_string(),
                });
            }
            Ok(self
                .vn_changes
                .lock()
                .unwrap()
                .get(&epoch.as_u64())
                .cloned()
                .unwrap_or_default())
        }

        async fn get_validator_nodes(
            &mut self,
            _height: u64,
        ) -> Result<impl Stream<Item = Result<BaseLayerValidatorNode, BaseNodeClientError>> + Send, BaseNodeClientError>
        {
            Ok(stream::iter(
                Vec::<Result<BaseLayerValidatorNode, BaseNodeClientError>>::new(),
            ))
        }

        async fn get_template_registrations(
            &mut self,
            _start_hash: Option<FixedHash>,
            _count: u64,
        ) -> Result<Vec<CodeTemplateRegistration>, BaseNodeClientError> {
            unimplemented!("not used by the reorg tests")
        }

        async fn get_header_by_hash(&mut self, block_hash: &FixedHash) -> Result<BlockHeader, BaseNodeClientError> {
            self.chain
                .lock()
                .unwrap()
                .iter()
                .find(|h| hash_header(NETWORK, h) == *block_hash)
                .cloned()
                .ok_or(BaseNodeClientError::GrpcStatus {
                    code: tonic::Code::NotFound,
                    message: "header not found".to_string(),
                })
        }

        async fn stream_headers(
            &mut self,
            from_height: u64,
            limit: u64,
        ) -> Result<impl Stream<Item = Result<BlockHeader, BaseNodeClientError>> + Unpin + Send, BaseNodeClientError>
        {
            let chain = self.chain.lock().unwrap();
            let headers: Vec<_> = chain
                .iter()
                .filter(|h| h.height >= from_height && h.height < from_height.saturating_add(limit))
                .cloned()
                .map(Ok)
                .collect();
            Ok(stream::iter(headers))
        }

        async fn get_consensus_constants(
            &mut self,
            _tip: u64,
        ) -> Result<BaseLayerConsensusConstants, BaseNodeClientError> {
            Ok(BaseLayerConsensusConstants {
                epoch_length: EPOCH_LENGTH,
                validator_node_registration_min_deposit_amount: MicroMinotari::from(0u64),
            })
        }

        async fn get_sidechain_utxos(
            &mut self,
            _start_hash: Option<FixedHash>,
            _count: u64,
        ) -> Result<Vec<SideChainUtxos>, BaseNodeClientError> {
            unimplemented!("not used by the reorg tests")
        }
    }

    fn make_header(height: u64, salt: u64) -> BlockHeader {
        let mut header = BlockHeader::new(0);
        header.height = height;
        // nonce feeds hash_header, so distinct salts give distinct block hashes.
        header.nonce = salt;
        header
    }

    fn make_inner(
        store: InMemoryStore,
        client: MockBaseNode,
        height_lag: u64,
    ) -> BaseLayerOracleInner<InMemoryStore, MockBaseNode> {
        BaseLayerOracleInner {
            config: BaseLayerEpochOracleConfig {
                start_height: 0,
                height_lag,
                scanning_interval: Duration::from_secs(1),
                sidechain_id: None,
                features: BaseLayerEpochOracleFeatures {
                    sync_headers: true,
                    sync_validator_node_changes: false,
                },
                epoch_end_spread_blocks: 0,
            },
            store,
            last_scanned_height: 0,
            last_scanned_tip: None,
            last_scanned_hash: None,
            last_epoch_hash: None,
            last_scanned_validator_node_mr: None,
            base_node_client: client,
            has_attempted_scan: false,
            pending_events: VecDeque::new(),
            header_buf: Vec::new(),
            network: NETWORK,
            cached_epoch_length: None,
        }
    }

    /// Drives the scanner until it has fully caught up with the tip, mirroring the poll loop's
    /// `scan_blockchain(has_more)` driving.
    async fn sync_to_tip(inner: &mut BaseLayerOracleInner<InMemoryStore, MockBaseNode>) {
        let mut has_more = false;
        loop {
            has_more = inner.scan_blockchain(has_more).await.unwrap();
            inner.pending_events.clear();
            if !has_more {
                break;
            }
        }
    }

    fn linear_chain(heights: std::ops::RangeInclusive<u64>) -> Vec<BlockHeader> {
        heights.map(|h| make_header(h, h)).collect()
    }

    #[tokio::test]
    async fn observed_boundary_hash_is_readable_before_the_epoch_is_activated() {
        let store = InMemoryStore::default();
        let chain = linear_chain(0..=20);
        let client = MockBaseNode::new(chain.clone());
        let mut inner = make_inner(store.clone(), client, 5);

        // sync_to_tip drops the emitted events, standing in for an epoch manager that has scanned the
        // boundary but not yet applied the resulting EpochChanged.
        sync_to_tip(&mut inner).await;
        assert_eq!(inner.last_scanned_height, 15);

        // Epoch 3 opens at height 15, which we have scanned.
        assert_eq!(
            inner.observed_epoch_boundary_hash(Epoch(3)).unwrap(),
            Some(hash_header(NETWORK, &chain[15]))
        );
        // Epoch 4 opens at height 20, above our lagged scan position.
        assert_eq!(inner.observed_epoch_boundary_hash(Epoch(4)).unwrap(), None);
    }

    #[tokio::test]
    async fn observed_boundary_hash_rejects_a_first_header_that_is_not_the_boundary() {
        let store = InMemoryStore::default();
        let client = MockBaseNode::new(linear_chain(0..=20));
        let mut inner = make_inner(store.clone(), client, 5);
        inner.cached_epoch_length = Some(EPOCH_LENGTH);

        // A scan range opening mid-epoch makes height 12 epoch 2's lowest stored header; the epoch's
        // boundary at height 10 was never scanned, so there is nothing to ratify against.
        store
            .add_block_headers([BlockHeaderModel {
                epoch: Epoch(2),
                height: 12,
                block_hash: FixedHash::default(),
                kernel_merkle_root: FixedHash::default(),
                validator_node_merkle_root: FixedHash::default(),
            }])
            .unwrap();

        assert_eq!(inner.observed_epoch_boundary_hash(Epoch(2)).unwrap(), None);
    }

    #[tokio::test]
    async fn reorg_rewinds_to_fork_point_and_prunes_orphans() {
        // height_lag = 5, tip = 20 -> lagged tip = 15. Scans heights 1..=15.
        let store = InMemoryStore::default();
        let chain_a = linear_chain(0..=20);
        let client = MockBaseNode::new(chain_a.clone());
        let mut inner = make_inner(store.clone(), client.clone(), 5);

        sync_to_tip(&mut inner).await;
        assert_eq!(inner.last_scanned_height, 15);
        assert_eq!(store.heights(), (1..=15).collect::<Vec<_>>());

        // Reorg diverging at height 13 (fork point 12), within the recoverable depth (15 - 5 = 10).
        let mut chain_b = chain_a[..=12].to_vec();
        chain_b.extend((13..=20).map(|h| make_header(h, h + 10_000)));
        client.set_chain(chain_b.clone());

        let tip_b = inner.base_node_client.get_tip_info().await.unwrap();
        inner.handle_reorg(&tip_b).await.unwrap();

        // Orphaned headers 13..=15 pruned; scan rewound to the fork point.
        assert_eq!(inner.last_scanned_height, 12);
        assert_eq!(store.heights(), (1..=12).collect::<Vec<_>>());
        assert_eq!(inner.last_scanned_hash, Some(hash_header(NETWORK, &chain_b[12])));
        // last_epoch_hash reset to the boundary that opened the fork epoch (epoch 2 -> height 10),
        // not left at the orphaned epoch-3 boundary (height 15).
        assert_eq!(inner.last_epoch_hash, Some(hash_header(NETWORK, &chain_a[10])));
    }

    #[tokio::test]
    async fn reorg_scan_converges_on_new_chain() {
        let store = InMemoryStore::default();
        let chain_a = linear_chain(0..=20);
        let client = MockBaseNode::new(chain_a.clone());
        let mut inner = make_inner(store.clone(), client.clone(), 5);

        sync_to_tip(&mut inner).await;

        let mut chain_b = chain_a[..=12].to_vec();
        chain_b.extend((13..=20).map(|h| make_header(h, h + 10_000)));
        client.set_chain(chain_b.clone());

        // A full scan round detects the reorg, repairs the store, and re-scans the new chain.
        sync_to_tip(&mut inner).await;

        assert_eq!(inner.last_scanned_height, 15);
        assert_eq!(store.heights(), (1..=15).collect::<Vec<_>>());
        // Heights above the fork now carry chain B's hashes; the orphaned chain A headers are gone.
        for h in 13..=15u64 {
            assert_eq!(store.hash_at(h), Some(hash_header(NETWORK, &chain_b[h as usize])));
        }
        // Heights at and below the fork are untouched.
        for h in 1..=12u64 {
            assert_eq!(store.hash_at(h), Some(hash_header(NETWORK, &chain_a[h as usize])));
        }
    }

    #[tokio::test]
    async fn reorg_deeper_than_lag_triggers_full_rescan() {
        let store = InMemoryStore::default();
        let chain_a = linear_chain(0..=20);
        let client = MockBaseNode::new(chain_a.clone());
        let mut inner = make_inner(store.clone(), client.clone(), 5);

        sync_to_tip(&mut inner).await;
        assert_eq!(inner.last_scanned_height, 15);

        // Diverge at height 8 (fork point 7), below the recoverable range (15 - 5 = 10): the scanner
        // cannot find a common ancestor in [11, 15] and rebuilds from the start height.
        let mut chain_b = chain_a[..=7].to_vec();
        chain_b.extend((8..=20).map(|h| make_header(h, h + 10_000)));
        client.set_chain(chain_b);

        let tip_b = inner.base_node_client.get_tip_info().await.unwrap();
        inner.handle_reorg(&tip_b).await.unwrap();

        assert!(store.heights().is_empty());
        assert_eq!(inner.last_scanned_height, 0);
        assert_eq!(inner.last_scanned_hash, None);
    }

    fn make_inner_syncing_vn_changes(
        store: InMemoryStore,
        client: MockBaseNode,
        height_lag: u64,
    ) -> BaseLayerOracleInner<InMemoryStore, MockBaseNode> {
        let mut inner = make_inner(store, client, height_lag);
        inner.config.features.sync_validator_node_changes = true;
        inner
    }

    /// Linear chain whose `validator_node_mr` is a function of the epoch, so the MR changes exactly
    /// at each epoch boundary — the trigger for a deferred `get_validator_node_changes` fetch.
    fn chain_with_epoch_mr(heights: std::ops::RangeInclusive<u64>) -> Vec<BlockHeader> {
        heights
            .map(|h| {
                let mut header = make_header(h, h);
                header.validator_node_mr = FixedHash::from([(h / EPOCH_LENGTH) as u8; 32]);
                header
            })
            .collect()
    }

    /// A minimal, convertible grpc validator-node-change (a Remove with a valid 32-byte key).
    fn vn_remove() -> ValidatorNodeChange {
        use minotari_app_grpc::tari_rpc;
        tari_rpc::ValidatorNodeChange {
            change: Some(tari_rpc::validator_node_change::Change::Remove(
                tari_rpc::ValidatorNodeChangeRemove {
                    public_key: vec![0u8; 32],
                },
            )),
        }
    }

    /// The deferred fetches (pass 2) must still produce `ActiveValidatorNodeSetChanged` events in the
    /// same stream order as the inline version did — each one immediately ahead of its epoch's
    /// `EpochChanged`, with empty change sets dropped.
    #[tokio::test]
    async fn validator_node_changes_are_emitted_in_order_before_each_epoch_change() {
        // height_lag = 5, tip = 20 -> lagged tip = 15. One batch scans heights 1..=15, crossing epoch
        // boundaries at heights 5, 10, 15 (epochs 1, 2, 3). The MR also changes at height 1 (epoch 0).
        let store = InMemoryStore::default();
        let client = MockBaseNode::new(chain_with_epoch_mr(0..=20));
        // Serve changes for epochs 1..=3; epoch 0 is intentionally left empty to exercise the path
        // where the MR changed (because of other sidechains) but our change set is empty -> dropped.
        let mut changes = HashMap::new();
        for epoch in 1..=3u64 {
            changes.insert(epoch, vec![vn_remove()]);
        }
        client.set_vn_changes(changes);

        let mut inner = make_inner_syncing_vn_changes(store, client.clone(), 5);

        // The whole range fits in a single batch (15 < the 1000-header batch limit).
        inner.scan_blockchain(false).await.unwrap();

        let ordered: Vec<(&str, u64)> = inner
            .pending_events
            .iter()
            .filter_map(|e| match e {
                EpochEvent::ActiveValidatorNodeSetChanged { epoch, node_changes } => {
                    assert!(
                        !node_changes.is_empty(),
                        "empty change sets must be dropped, not emitted"
                    );
                    Some(("vn", epoch.as_u64()))
                },
                EpochEvent::EpochChanged { epoch, .. } => Some(("epoch", epoch.as_u64())),
                _ => None,
            })
            .collect();

        assert_eq!(
            ordered,
            vec![
                ("vn", 1),
                ("epoch", 1),
                ("vn", 2),
                ("epoch", 2),
                ("vn", 3),
                ("epoch", 3),
            ],
            "each ActiveValidatorNodeSetChanged must immediately precede its epoch's EpochChanged"
        );

        // Every MR change (epochs 0,1,2,3) triggered exactly one fetch — including epoch 0, whose
        // empty result was dropped above.
        assert_eq!(client.vn_call_epochs_sorted(), vec![0, 1, 2, 3]);
    }

    /// A failed deferred fetch must abort the batch atomically: because the fetch now happens after
    /// the whole header stream is drained, the scan position must not be advanced (and no events may
    /// leak), so the entire batch is safely re-scanned on the next round.
    #[tokio::test]
    async fn failed_validator_node_change_fetch_leaves_scan_position_unadvanced() {
        let store = InMemoryStore::default();
        let client = MockBaseNode::new(chain_with_epoch_mr(0..=20));
        client.fail_vn_changes();
        let mut inner = make_inner_syncing_vn_changes(store.clone(), client.clone(), 5);

        let result = inner.scan_blockchain(false).await;
        assert!(
            result.is_err(),
            "a failed validator-node-change fetch must fail the scan"
        );

        assert_eq!(
            inner.last_scanned_height, 0,
            "scan position must not advance on failure"
        );
        assert_eq!(inner.last_scanned_hash, None);
        assert!(inner.last_scanned_validator_node_mr.is_none());
        assert!(
            inner.pending_events.is_empty(),
            "no events may leak from a failed batch"
        );
        assert!(
            store.heights().is_empty(),
            "no headers may be persisted from a failed batch"
        );
    }

    /// Indexer mode: `sync_headers` disabled, so only epoch boundaries are scanned.
    fn make_inner_boundary_mode(
        store: InMemoryStore,
        client: MockBaseNode,
        height_lag: u64,
    ) -> BaseLayerOracleInner<InMemoryStore, MockBaseNode> {
        let mut inner = make_inner(store, client, height_lag);
        inner.config.features.sync_headers = false;
        inner
    }

    /// Indexer mode that also resolves validator-node changes at the boundaries it crosses.
    fn make_inner_boundary_mode_syncing_vn(
        store: InMemoryStore,
        client: MockBaseNode,
        height_lag: u64,
    ) -> BaseLayerOracleInner<InMemoryStore, MockBaseNode> {
        let mut inner = make_inner_boundary_mode(store, client, height_lag);
        inner.config.features.sync_validator_node_changes = true;
        inner
    }

    /// Drives the scanner to the tip, collecting every emitted event (mirrors the poll loop draining
    /// `pending_events` between scan rounds).
    async fn run_scan_collecting_events(
        inner: &mut BaseLayerOracleInner<InMemoryStore, MockBaseNode>,
    ) -> Vec<EpochEvent> {
        let mut events = Vec::new();
        let mut has_more = false;
        loop {
            has_more = inner.scan_blockchain(has_more).await.unwrap();
            events.extend(inner.pending_events.drain(..));
            if !has_more {
                break;
            }
        }
        events
    }

    fn epoch_changes(events: &[EpochEvent]) -> Vec<(u64, FixedHash)> {
        events
            .iter()
            .filter_map(|e| match e {
                EpochEvent::EpochChanged { epoch, epoch_hash } => Some((epoch.as_u64(), *epoch_hash)),
                _ => None,
            })
            .collect()
    }

    /// The boundary-only path (indexer) must emit exactly the same `EpochChanged` events — same
    /// epochs and same boundary hashes — as the full streaming path (validator node), while storing
    /// no headers.
    #[tokio::test]
    async fn boundary_scan_emits_same_epoch_changes_as_full_scan() {
        // height_lag = 5, tip = 20 -> scans heights 1..=15, crossing boundaries at 5, 10, 15.
        let chain = linear_chain(0..=20);

        let full_store = InMemoryStore::default();
        let mut full = make_inner(full_store.clone(), MockBaseNode::new(chain.clone()), 5);
        let full_events = run_scan_collecting_events(&mut full).await;

        let boundary_store = InMemoryStore::default();
        let mut boundary = make_inner_boundary_mode(boundary_store.clone(), MockBaseNode::new(chain), 5);
        let boundary_events = run_scan_collecting_events(&mut boundary).await;

        // Same epoch boundaries with identical hashes.
        assert_eq!(
            epoch_changes(&full_events).iter().map(|(e, _)| *e).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(epoch_changes(&full_events), epoch_changes(&boundary_events));

        // Full mode stored every header; boundary mode stored none.
        assert_eq!(full_store.heights(), (1..=15).collect::<Vec<_>>());
        assert!(boundary_store.heights().is_empty());

        // Both reached the lagged tip (no runaway has_more loop).
        assert_eq!(full.last_scanned_height, 15);
        assert_eq!(boundary.last_scanned_height, 15);
    }

    /// Boundary mode still resolves validator-node changes — deferred and concurrent — for each
    /// boundary it crosses, ordered ahead of that epoch's `EpochChanged`.
    #[tokio::test]
    async fn boundary_scan_detects_validator_changes_at_boundaries() {
        let store = InMemoryStore::default();
        let client = MockBaseNode::new(chain_with_epoch_mr(0..=20));
        let mut changes = HashMap::new();
        for epoch in 1..=3u64 {
            changes.insert(epoch, vec![vn_remove()]);
        }
        client.set_vn_changes(changes);

        let mut inner = make_inner_boundary_mode_syncing_vn(store.clone(), client.clone(), 5);
        let events = run_scan_collecting_events(&mut inner).await;

        let ordered: Vec<(&str, u64)> = events
            .iter()
            .filter_map(|e| match e {
                EpochEvent::ActiveValidatorNodeSetChanged { epoch, node_changes } => {
                    assert!(!node_changes.is_empty());
                    Some(("vn", epoch.as_u64()))
                },
                EpochEvent::EpochChanged { epoch, .. } => Some(("epoch", epoch.as_u64())),
                _ => None,
            })
            .collect();

        assert_eq!(ordered, vec![
            ("vn", 1),
            ("epoch", 1),
            ("vn", 2),
            ("epoch", 2),
            ("vn", 3),
            ("epoch", 3),
        ]);
        // Boundary mode samples the MR only at boundaries, so it queries the epochs it crosses
        // (1, 2, 3) — not the pre-start epoch 0 that the full path samples at its mid-epoch start.
        assert_eq!(client.vn_call_epochs_sorted(), vec![1, 2, 3]);
        assert!(store.heights().is_empty());
    }
}
