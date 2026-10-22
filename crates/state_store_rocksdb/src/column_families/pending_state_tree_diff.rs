//  Copyright 2025. The Tari Project
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

use minicbor::{CborLen, Decode, Encode};
use serde::Serialize;
use tari_consensus_types::BlockId;
use tari_ootle_common_types::shard::Shard;
use tari_ootle_storage::consensus_models::PendingShardStateTreeDiff;

use crate::{
    codecs::{BlockIdCodec, DefaultCodec, KeyPrefix},
    column_families::cf_names,
    prefixed,
    traits::Cf,
};

/// One shard's pending state tree diff.
#[derive(Debug, Clone, Serialize, Encode, Decode, CborLen)]
pub struct ShardStateTreeDiff {
    #[n(0)]
    pub shard: Shard,
    #[n(1)]
    pub diff: PendingShardStateTreeDiff,
}

/// [`ShardStateTreeDiff`] by reference, for writing a record without copying its diffs.
#[derive(Debug, Clone, Copy, Encode, CborLen)]
pub struct ShardStateTreeDiffRef<'a> {
    #[n(0)]
    pub shard: Shard,
    #[n(1)]
    pub diff: &'a PendingShardStateTreeDiff,
}

prefixed!(PendingStateTreeDiffRecordPrefix, KeyPrefix::PendingStateTreeDiffRecords);

/// A block's pending state tree diffs, one per shard it changes.
pub struct PendingStateTreeDiffRecordCf;

impl Cf for PendingStateTreeDiffRecordCf {
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
    type Prefix = PendingStateTreeDiffRecordPrefix;
    type Value = Vec<ShardStateTreeDiff>;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}
