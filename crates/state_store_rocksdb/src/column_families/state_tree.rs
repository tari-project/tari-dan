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

use tari_ootle_common_types::shard::Shard;
use tari_state_tree::{Node, NodeKey, StaleTreeNode, StateTreePayload, Version};

use crate::{
    codecs::{KeyPrefix, NodeKeyCodec, NumberCodec, ShardCodec, StaleTreeNodesCodec, TreeNodeCodec},
    column_families::cf_names,
    prefixed,
    traits::{Cf, QueryCf},
};

prefixed!(StateTreePrefix, KeyPrefix::StateTree);
pub struct StateTreeCf;

impl Cf for StateTreeCf {
    type Key = (Shard, NodeKey);
    type KeyCodec = (ShardCodec, NodeKeyCodec);
    type Prefix = StateTreePrefix;
    type Value = Node<StateTreePayload>;
    type ValueCodec = TreeNodeCodec;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}

pub struct ByShardStateVersionQuery;

impl QueryCf for ByShardStateVersionQuery {
    type Cf = StateTreeCf;
    type Key = (Shard, Version);
    // Depends on NodeKeyCodec first serializing the Shard, then the Version.
    type KeyCodec = (ShardCodec, NumberCodec<Version>);
}

prefixed!(StateTreeStaleNodesPrefix, KeyPrefix::StateTreeStaleTreeNodesIndex);

pub struct StateTreeStaleNodesCf;

impl Cf for StateTreeStaleNodesCf {
    type Key = (Shard, Version);
    type KeyCodec = (ShardCodec, NumberCodec<Version>);
    type Prefix = StateTreeStaleNodesPrefix;
    type Value = Vec<StaleTreeNode>;
    type ValueCodec = StaleTreeNodesCodec;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}

pub struct ByStateTreeStaleShardQuery;

impl QueryCf for ByStateTreeStaleShardQuery {
    type Cf = StateTreeStaleNodesCf;
    type Key = Shard;
    type KeyCodec = ShardCodec;
}

/// Range query variant over `StateTreeStaleNodesCf` keyed by `(Shard, Version)` — lets
/// callers scan only the range `(shard, start_version)..(shard, end_version)` instead of
/// iterating every stale record for the shard and filtering.
pub struct ByStateTreeStaleShardVersionQuery;

impl QueryCf for ByStateTreeStaleShardVersionQuery {
    type Cf = StateTreeStaleNodesCf;
    type Key = (Shard, Version);
    type KeyCodec = (ShardCodec, NumberCodec<Version>);
}
