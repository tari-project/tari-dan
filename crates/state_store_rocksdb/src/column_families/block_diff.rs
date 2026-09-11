//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_consensus_types::BlockId;
use tari_engine_types::substate::SubstateId;
use tari_ootle_storage::consensus_models::SubstateChange;

use crate::{
    codecs::{
        BlockDiffKeyCodec,
        BlockIdCodec,
        BlockIdSeqSubstateIdVersion,
        DefaultCodec,
        KeyPrefix,
        SubstateIdBlockIdVersionSeq,
        SubstateIdCodec,
        UnitCodec,
    },
    column_families::block::BlockCf,
    prefixed,
    traits::{Cf, QueryCf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDiffEntry {
    pub block_id: BlockId,
    pub substate_id: SubstateId,
    pub change: SubstateChange,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockDiffInsertEntry<'a> {
    pub substate_id: &'a SubstateId,
    pub change: &'a SubstateChange,
}

pub struct BlockDiffKey {
    pub block_id: BlockId,
    pub substate_id: SubstateId,
    pub version: u64,
    pub is_up: bool,
    /// Retains the ordering of the substate changes in the block. This limits the maximum number of substate changes
    /// in a block to u32::MAX (4,294,967,295).
    pub sequence: u32,
}

prefixed!(BlockDiffPrefix, KeyPrefix::BlockDiffs);

/// Ordered substate transitions for a block.
/// Schema: (BlockId, Seq, SubstateId, Version, IsUp) -> Vec<SubstateChange>
pub struct BlockDiffCf;

impl Cf for BlockDiffCf {
    type Key = BlockDiffKey;
    type KeyCodec = BlockDiffKeyCodec<BlockIdSeqSubstateIdVersion>;
    type Prefix = BlockDiffPrefix;
    type Value = SubstateChange;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        BlockCf::name()
    }
}

/// Query for block diffs by block id. Since the sequence number is the second part of the key, iterating over this
/// query will return them in the original order.
pub struct ByBlockIdQuery;

impl QueryCf for ByBlockIdQuery {
    type Cf = BlockDiffCf;
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
}

prefixed!(SubstateIdBlockDiffPrefix, KeyPrefix::BlockDiffsBySubstateId);

/// Query for block diffs by substate id. This is a secondary index that allows querying by substate id.
/// Schema: (SubstateId, BlockId, Version, Seq) -> ()
pub struct SubstateIdIndex;

impl Cf for SubstateIdIndex {
    type Key = BlockDiffKey;
    type KeyCodec = BlockDiffKeyCodec<SubstateIdBlockIdVersionSeq>;
    type Prefix = SubstateIdBlockDiffPrefix;
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        BlockCf::name()
    }
}

pub struct BySubstateIdQuery;

impl QueryCf for BySubstateIdQuery {
    type Cf = SubstateIdIndex;
    type Key = SubstateId;
    type KeyCodec = SubstateIdCodec;
}
