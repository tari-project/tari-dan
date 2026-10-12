//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::{SubstateAddress, shard::Shard};
use tari_ootle_storage::consensus_models::SubstateDownProofRecord;

use crate::{
    codecs::{FixedBytesCodec, KeyPrefix, SerdeBridgeCodec, ShardCodec},
    column_families::cf_names,
    prefixed,
    traits::Cf,
};

prefixed!(SubstateDownProofPrefix, KeyPrefix::SubstateDownProofs);

/// Per destroyed substate (keyed by its shard and the address of its `(id, version)`), the proof that it was
/// committed before it went down.
///
/// Records are not pruned with downed substate values: they are what keeps a destruction provable once the tree
/// nodes and blocks it was proved from are gone. Each holds a level-1 leaf proof and a level-2 shard-root proof; the
/// commit proof it relies on is the one its state version proof cites, a received proof's own bytes or the commit
/// proof stored per committed block, so the column holds no commit proofs. It grows with the number of destroyed
/// substate versions.
pub struct SubstateDownProofCf;

impl Cf for SubstateDownProofCf {
    type Key = (Shard, SubstateAddress);
    type KeyCodec = (ShardCodec, FixedBytesCodec<{ SubstateAddress::LENGTH }>);
    type Prefix = SubstateDownProofPrefix;
    type Value = SubstateDownProofRecord;
    type ValueCodec = SerdeBridgeCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::SUBSTATES
    }
}
