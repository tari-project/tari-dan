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
    pub bandwidth: Bandwidth,
    pub disk: Disk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bandwidth {
    /// Upload to sustain block propagation at maximum block size: forwarding each block to the
    /// gossipsub mesh. The dominant term, and the reason the floor is not small.
    pub block_upload_mbps: f64,
    /// Download for the same, including duplicate mesh deliveries.
    pub block_download_mbps: f64,
    /// Votes exchanged across the committee each view.
    pub vote_mbps: f64,
    /// What a validator needs before any user traffic exists at all: block propagation plus votes
    /// at maximum block size. Publish this — it does not move with adoption.
    pub consensus_floor_mbps: f64,
    /// Transaction gossip per sustained transaction per second, so an adoption assumption can be
    /// multiplied through rather than guessed at once and frozen.
    pub gossip_mbps_per_tps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Disk {
    /// Ootle blocks per epoch, as supplied — this follows the layer-one epoch length and is not
    /// derivable offline.
    pub blocks_per_epoch: u64,
    pub bytes_per_block: u64,
    /// Worst-case block data for one epoch.
    pub bytes_per_epoch: u64,
    /// The retention window's ceiling: blocks and foreign proposals are pruned beyond
    /// `epoch_history_length` epochs, so this component of the database is bounded, not boundless.
    pub history_ceiling_bytes: u64,
    pub epoch_history_length: u64,
}

pub fn project(
    wire: &WireMeasurement,
    budgets: &Budgets,
    blocks_per_epoch: u64,
    epoch_history_length: u64,
) -> CapacityProjection {
    let block_bytes = wire.max_block_command_bytes as f64 + BLOCK_FIXED_OVERHEAD_BYTES;
    let per_view = budgets.block_time_secs;

    let to_mbps = |bytes_per_view: f64| bytes_per_view * 8.0 / per_view / 1_000_000.0;

    let block_upload_mbps = to_mbps(block_bytes * GOSSIPSUB_MESH_N);
    let block_download_mbps = to_mbps(block_bytes * DUPLICATE_DELIVERY_FACTOR);
    let vote_mbps = to_mbps(VOTE_BYTES * f64::from(budgets.committee_size_per_shard_group));

    // A transaction is gossiped once to the mesh and forwarded to it, so one transaction per second
    // costs its encoded size times the mesh degree, in each direction.
    let gossip_mbps_per_tps = wire.transaction_bytes as f64 * (GOSSIPSUB_MESH_N + 1.0) * 8.0 / 1_000_000.0;

    let bytes_per_block = block_bytes as u64;
    let bytes_per_epoch = bytes_per_block.saturating_mul(blocks_per_epoch);

    CapacityProjection {
        bandwidth: Bandwidth {
            block_upload_mbps,
            block_download_mbps,
            vote_mbps,
            consensus_floor_mbps: block_upload_mbps + block_download_mbps + vote_mbps,
            gossip_mbps_per_tps,
        },
        disk: Disk {
            blocks_per_epoch,
            bytes_per_block,
            bytes_per_epoch,
            history_ceiling_bytes: bytes_per_epoch.saturating_mul(epoch_history_length.max(1)),
            epoch_history_length,
        },
    }
}
