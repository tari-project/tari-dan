//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus_types::BlockId;
use tari_ootle_storage::consensus_models::SubstateChange;

use crate::{
    codecs::{BlockIdCodec, DefaultCodec, KeyPrefix},
    column_families::block::BlockCf,
    prefixed,
    traits::Cf,
};

prefixed!(BlockDiffRecordPrefix, KeyPrefix::BlockDiffRecords);

/// A block's substate changes in the order it made them.
pub struct BlockDiffRecordCf;

impl Cf for BlockDiffRecordCf {
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
    type Prefix = BlockDiffRecordPrefix;
    type Value = Vec<SubstateChange>;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        BlockCf::name()
    }
}
