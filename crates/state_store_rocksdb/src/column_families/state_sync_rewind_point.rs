//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::{SubstateAddress, shard::Shard};
use tari_state_tree::Version;

use crate::{
    codecs::{FixedBytesCodec, KeyPrefix, NumberCodec, ShardCodec},
    column_families::cf_names,
    prefixed,
    traits::Cf,
};

prefixed!(StateSyncRewindPointPrefix, KeyPrefix::StateSyncRewindPoint);

/// Per shard, the version state sync rewinds to if the versions it committed above it are never verified.
pub struct StateSyncRewindPointCf;

impl Cf for StateSyncRewindPointCf {
    type Key = Shard;
    type KeyCodec = ShardCodec;
    type Prefix = StateSyncRewindPointPrefix;
    type Value = Version;
    type ValueCodec = NumberCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}

prefixed!(StateSyncMissingValuePrefix, KeyPrefix::StateSyncMissingValues);

/// Per shard, every up substate that state sync committed as its value hash alone, with the state version that created
/// it. A sync is complete only once each has been destroyed, since this node does not hold its value.
pub struct StateSyncMissingValueCf;

impl Cf for StateSyncMissingValueCf {
    type Key = (Shard, SubstateAddress);
    type KeyCodec = (ShardCodec, FixedBytesCodec<{ SubstateAddress::LENGTH }>);
    type Prefix = StateSyncMissingValuePrefix;
    type Value = Version;
    type ValueCodec = NumberCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::STATE_TREE
    }
}
