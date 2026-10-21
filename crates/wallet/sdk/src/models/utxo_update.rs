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
/// scan resumes from it, and the shard's verified cursor stays where the claim began, until a verified checkpoint that
/// ended after the claim began expires it. The shard is then scanned again from the verified cursor, so that an
/// indexer's claim hides no update for longer than that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedStateVersion {
    pub state_version: StateVersion,
    /// The epoch of the latest verified checkpoint of the shard when the claim began, or `None` without one.
    pub verified_epoch: Option<Epoch>,
}
