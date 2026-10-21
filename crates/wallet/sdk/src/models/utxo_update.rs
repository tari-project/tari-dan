//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub use tari_indexer_client::types::{
    UtxoBurnt,
    UtxoSpent,
    UtxoStateUpdateSet,
    UtxoUnspent,
    UtxoUpdateSet,
    WalletUtxoUpdate,
};
use tari_ootle_common_types::{Epoch, StateVersion, shard::Shard};

#[derive(Debug, Clone)]
pub struct UtxoUpdatePayload {
    pub sos: Option<StartOfShard>,
    pub update: Option<WalletUtxoUpdate>,
    pub eos: Option<EndOfShard>,
}

#[derive(Debug, Clone)]
pub struct StartOfShard {
    pub shard: Shard,
    pub max_state_version: StateVersion,
    /// The shard holds updates beyond this pass, as reported by the indexer.
    pub has_more: bool,
}

#[derive(Debug, Clone)]
pub struct EndOfShard {
    pub max_state_version: StateVersion,
}

/// A shard's stealth scan cursor as an indexer reported it, past what a verified checkpoint commits for the shard. A
/// scan resumes from it until a checkpoint that ended after it was stored commits a lower state version for the shard,
/// which proves the indexer claimed updates the network never committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedStateVersion {
    pub state_version: StateVersion,
    /// The epoch of the latest verified checkpoint of the shard when the claim was stored, or `None` without one.
    pub verified_epoch: Option<Epoch>,
}
