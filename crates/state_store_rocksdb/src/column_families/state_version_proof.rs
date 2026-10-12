//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus_types::BlockId;
use tari_ootle_common_types::{Epoch, shard::Shard};
use tari_ootle_storage::consensus_models::StateVersionProof;
use tari_state_tree::Version;

use crate::{
    codecs::{BlockIdCodec, BytesCodec, EpochCodec, KeyPrefix, NumberCodec, SerdeBridgeCodec, ShardCodec, UnitCodec},
    column_families::cf_names,
    prefixed,
    traits::{Cf, QueryCf},
};

prefixed!(StateVersionProofPrefix, KeyPrefix::StateVersionProofs);

/// Per shard and state version, what this node holds to prove that state to a peer syncing it, or to complete a
/// substate down proof record that cites the version.
///
/// A proof point is kept for good, since state sync proves every one from the start of the shard's history. Any other
/// version has a [`StateVersionProofRetentionCf`] entry, and epoch GC prunes it once that entry's epoch ages past
/// `down_proof_history_length`.
pub struct StateVersionProofCf;

impl Cf for StateVersionProofCf {
    type Key = (Shard, Version);
    type KeyCodec = (ShardCodec, NumberCodec<Version>);
    type Prefix = StateVersionProofPrefix;
    type Value = StateVersionProof;
    type ValueCodec = SerdeBridgeCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}

prefixed!(BlockCommitProofPrefix, KeyPrefix::BlockCommitProofs);

/// The CBOR-encoded commit proof of each committed block that produced state versions, which the
/// [`StateVersionProofCf`] entries naming the block are served with once the block is pruned. Deleted with the last
/// of those entries, through [`BlockCommitProofCitationCf`].
pub struct BlockCommitProofCf;

impl Cf for BlockCommitProofCf {
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
    type Prefix = BlockCommitProofPrefix;
    type Value = Vec<u8>;
    type ValueCodec = BytesCodec;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}

prefixed!(StateVersionProofRetentionPrefix, KeyPrefix::StateVersionProofRetention);

/// Per [`StateVersionProofCf`] entry that is not a proof point, the epoch it is kept through: the later of the epoch
/// of the block that produced the version and the epoch of the latest destruction whose down proof record cites it.
/// Since a record is pruned once the epoch of its destruction ages past the same window, no record outlives the proof
/// it cites.
pub struct StateVersionProofRetentionCf;

impl Cf for StateVersionProofRetentionCf {
    type Key = (Shard, Version);
    type KeyCodec = (ShardCodec, NumberCodec<Version>);
    type Prefix = StateVersionProofRetentionPrefix;
    type Value = Epoch;
    type ValueCodec = EpochCodec;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}

prefixed!(
    StateVersionProofEpochIndexPrefix,
    KeyPrefix::StateVersionProofEpochIndex
);

/// Each [`StateVersionProofRetentionCf`] key under the epoch it is kept through, so that epoch GC reaches them in
/// epoch order. An entry whose epoch is no longer the key's retention epoch is stale: GC drops it and keeps the
/// proof.
pub struct StateVersionProofEpochIndex;

impl Cf for StateVersionProofEpochIndex {
    type Key = (Epoch, Shard, Version);
    type KeyCodec = (EpochCodec, ShardCodec, NumberCodec<Version>);
    type Prefix = StateVersionProofEpochIndexPrefix;
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}

pub struct StateVersionProofByEpochQuery;

impl QueryCf for StateVersionProofByEpochQuery {
    type Cf = StateVersionProofEpochIndex;
    type Key = Epoch;
    type KeyCodec = EpochCodec;
}

prefixed!(BlockCommitProofCitationPrefix, KeyPrefix::BlockCommitProofCitations);

/// Per committed block, each shard whose [`StateVersionProofCf`] entry names the block as its source. A block moves a
/// shard to at most one new state version, so the shard identifies the entry.
pub struct BlockCommitProofCitationCf;

impl Cf for BlockCommitProofCitationCf {
    type Key = (BlockId, Shard);
    type KeyCodec = (BlockIdCodec, ShardCodec);
    type Prefix = BlockCommitProofCitationPrefix;
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}

pub struct BlockCommitProofCitationByBlockQuery;

impl QueryCf for BlockCommitProofCitationByBlockQuery {
    type Cf = BlockCommitProofCitationCf;
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
}
