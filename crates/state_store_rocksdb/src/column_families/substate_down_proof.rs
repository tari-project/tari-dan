//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::{Epoch, SubstateAddress, shard::Shard};
use tari_ootle_storage::consensus_models::SubstateDownProofRecord;

use crate::{
    codecs::{EpochCodec, FixedBytesCodec, KeyPrefix, SerdeBridgeCodec, ShardCodec, UnitCodec},
    column_families::cf_names,
    prefixed,
    traits::{Cf, QueryCf},
};

prefixed!(SubstateDownProofPrefix, KeyPrefix::SubstateDownProofs);

/// Per destroyed substate (keyed by its shard and the address of its `(id, version)`), the proof that it was
/// committed before it went down.
///
/// Records outlive downed substate values: they are what keeps a destruction provable once the tree nodes and blocks
/// it was proved from are gone. Each holds a level-1 leaf proof and a level-2 shard-root proof; the commit proof it
/// relies on is the one its state version proof cites, a received proof's own bytes or the commit proof stored per
/// committed block, so the column holds no commit proofs. Epoch GC prunes a record once the epoch of its destruction
/// ages past `down_proof_history_length`, through [`SubstateDownProofEpochIndex`].
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

prefixed!(
    SubstateDownProofEpochIndexPrefix,
    KeyPrefix::SubstateDownProofEpochIndex
);

/// Each [`SubstateDownProofCf`] key, under the epoch of the destruction it proves, so that epoch GC reaches records in
/// epoch order. Written and deleted together with the record.
pub struct SubstateDownProofEpochIndex;

impl Cf for SubstateDownProofEpochIndex {
    type Key = (Epoch, Shard, SubstateAddress);
    type KeyCodec = (EpochCodec, ShardCodec, FixedBytesCodec<{ SubstateAddress::LENGTH }>);
    type Prefix = SubstateDownProofEpochIndexPrefix;
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        cf_names::SUBSTATES
    }
}

pub struct SubstateDownProofByEpochQuery;

impl QueryCf for SubstateDownProofByEpochQuery {
    type Cf = SubstateDownProofEpochIndex;
    type Key = Epoch;
    type KeyCodec = EpochCodec;
}
