//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Bandwidth and disk requirements, projected from measured encoded sizes and the consensus
//! constants.
//!
//! Neither of these is a property of the machine under test — every validator on the network faces
//! the same numbers — so nothing here is graded per host. They are reported because an operator
//! provisioning a box needs them alongside the figures that are host-specific, and because both
//! decompose the same way:
//!
//! * A **floor** the protocol fixes, which is computable today and does not move with adoption. For bandwidth this is
//!   what a saturated committee costs; for disk it is the retention window. These are the numbers to publish.
//! * A **rate** that scales with network activity, expressed per unit of traffic so it can be multiplied by whatever
//!   adoption assumption is being planned against, rather than baked into a single figure that goes stale.
//!
//! The floor is the important half, and for the same reason the CPU floor was: a validator has to
//! survive the worst its peers can send it, not the average. A link sized for today's traffic drops
//! proposals the first time a committee saturates.
//!
//! What is *not* here: the growth of live substate state, which is the genuinely unbounded term.
//! Bounding it needs bytes-per-committed-substate measured against a running network with
//! representative traffic, which no offline tool can supply.

use serde::{Deserialize, Serialize};

use crate::{report::Budgets, wire::WireMeasurement};

/// Gossipsub mesh degree — how many peers a node forwards each message to, and therefore the
/// multiplier on every validator's upload bill.
///
/// This is the libp2p default: `networking/swarm/src/behaviour.rs` sets `max_transmit_size`,
/// validation mode and the message id function, and leaves the mesh parameters alone. It is
/// consequently a number nobody chose, which is worth knowing given it multiplies the largest term
/// below.
const GOSSIPSUB_MESH_N: f64 = 6.0;

/// Header, justify certificate and signatures a block carries regardless of how many commands are
/// in it. Small against a full block's command payload, included so the per-view figure is not an
/// underestimate for near-empty blocks.
const BLOCK_FIXED_OVERHEAD_BYTES: f64 = 4096.0;

/// A vote: block id, decision and a signature.
const VOTE_BYTES: f64 = 128.0;

/// Duplicate deliveries a mesh peer receives before the message id function suppresses them.
/// Gossipsub delivers from several mesh peers at once, so download is a small multiple of the
/// message size rather than exactly one copy.
const DUPLICATE_DELIVERY_FACTOR: f64 = 2.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityProjection {
    /// One projection per block interval. Bandwidth and disk both scale inversely with the interval,
    /// and the interval is not a constant, so a single figure would be an assumption wearing the
    /// costume of a measurement.
    pub scenarios: Vec<Scenario>,
    /// Transaction gossip per sustained transaction per second, so an adoption assumption can be
    /// multiplied through. Independent of block interval.
    pub gossip_mbps_per_tps: f64,
    pub epoch_secs: f64,
    pub epoch_history_length: u64,
}

/// Requirements at one block production rate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// Why this interval, so the projection can be argued with rather than taken on faith.
    pub basis: String,
    pub block_interval_secs: f64,
    pub blocks_per_epoch: u64,
    /// Forwarding each block to the gossipsub mesh — the dominant term.
    pub upload_mbps: f64,
    pub download_mbps: f64,
    /// Block propagation plus committee votes, at maximum block size. What a validator needs before
    /// any user traffic exists.
    pub consensus_floor_mbps: f64,
    pub bytes_per_epoch: u64,
    /// Bounded: blocks prune beyond `epoch_history_length` epochs.
    pub history_ceiling_bytes: u64,
}

/// Projects requirements at each supplied block interval.
///
/// `saturation_interval_secs` is the interval a busy network actually runs at, when known. Under
/// load the next block is proposed as soon as the previous one's quorum certificate forms
/// (`on_receive_vote` beats the pacemaker), so the rate is set by execution plus one round of vote
/// collection — not by `pacemaker_block_time`, which is the liveness ceiling for a *quiet* network.
/// Projecting only at the ceiling understates a busy network by the ratio between the two.
pub fn project(
    wire: &WireMeasurement,
    budgets: &Budgets,
    epoch_secs: f64,
    epoch_history_length: u64,
    saturation_interval_secs: Option<f64>,
) -> CapacityProjection {
    let block_bytes = wire.block_command_bytes as f64 + BLOCK_FIXED_OVERHEAD_BYTES;

    let mut scenarios = vec![scenario(
        "quiet (pacemaker ceiling)",
        format!(
            "no backlog, so blocks come at pacemaker_block_time = {:.0}s",
            budgets.block_time_secs
        ),
        budgets.block_time_secs,
        block_bytes,
        budgets,
        epoch_secs,
        epoch_history_length,
    )];

    if let Some(interval) = saturation_interval_secs.filter(|i| *i > 0.0 && *i < budgets.block_time_secs) {
        scenarios.push(scenario(
            "saturated",
            format!(
                "a backlog keeps proposing: {interval:.2}s = block execution measured on this host plus one vote \
                 round trip"
            ),
            interval,
            block_bytes,
            budgets,
            epoch_secs,
            epoch_history_length,
        ));
    }

    CapacityProjection {
        scenarios,
        gossip_mbps_per_tps: wire.transaction_bytes as f64 * (GOSSIPSUB_MESH_N + 1.0) * 8.0 / 1_000_000.0,
        epoch_secs,
        epoch_history_length,
    }
}

fn scenario(
    name: &str,
    basis: String,
    interval_secs: f64,
    block_bytes: f64,
    budgets: &Budgets,
    epoch_secs: f64,
    epoch_history_length: u64,
) -> Scenario {
    let to_mbps = |bytes_per_block: f64| bytes_per_block * 8.0 / interval_secs / 1_000_000.0;

    let upload_mbps = to_mbps(block_bytes * GOSSIPSUB_MESH_N);
    let download_mbps = to_mbps(block_bytes * DUPLICATE_DELIVERY_FACTOR);
    let vote_mbps = to_mbps(VOTE_BYTES * f64::from(budgets.committee_size_per_shard_group));

    let blocks_per_epoch = (epoch_secs / interval_secs) as u64;
    let bytes_per_epoch = (block_bytes as u64).saturating_mul(blocks_per_epoch);

    Scenario {
        name: name.to_string(),
        basis,
        block_interval_secs: interval_secs,
        blocks_per_epoch,
        upload_mbps,
        download_mbps,
        consensus_floor_mbps: upload_mbps + download_mbps + vote_mbps,
        bytes_per_epoch,
        history_ceiling_bytes: bytes_per_epoch.saturating_mul(epoch_history_length.max(1)),
    }
}
