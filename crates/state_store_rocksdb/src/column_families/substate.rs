//  Copyright 2024. The Tari Project
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
use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{Epoch, SubstateAddress, shard::Shard};
use tari_state_tree::Version;

use crate::{
    codecs::{
        DefaultCodec,
        DefaultVersionedCodec,
        EpochCodec,
        FixedBytesCodec,
        KeyPrefix,
        NumberCodec,
        ShardCodec,
        SubstateIdCodec,
    },
    column_families::cf_names,
    prefixed,
    traits::{Cf, QueryCf},
    versioned_types::{LatestSubstateRecord, VersionedSubstateRecord},
};

prefixed!(SubstatePrefix, KeyPrefix::Substates);
pub struct SubstateCf;

impl Cf for SubstateCf {
    type Key = SubstateAddress;
    type KeyCodec = FixedBytesCodec<{ SubstateAddress::LENGTH }>;
    type Prefix = SubstatePrefix;
    type Value = LatestSubstateRecord;
    type ValueCodec = DefaultVersionedCodec<VersionedSubstateRecord>;

    fn name() -> &'static str {
        cf_names::SUBSTATES
    }
}

prefixed!(SubstatesHeadIndexPrefix, KeyPrefix::SubstatesHeadIndex);
pub struct HeadIndex;

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct SubstateHeadData {
    #[n(0)]
    pub version: u64,
    #[n(1)]
    pub is_up: bool,
}

impl Cf for HeadIndex {
    type Key = SubstateId;
    type KeyCodec = SubstateIdCodec;
    type Prefix = SubstatesHeadIndexPrefix;
    type Value = SubstateHeadData;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::SUBSTATES
    }
}

prefixed!(
    SubstatesUnprunedDownedValuesIndexPrefix,
    KeyPrefix::SubstatesUnprunedDownedValuesIndex
);

pub struct UnprunedDownedValuesIndex;

impl Cf for UnprunedDownedValuesIndex {
    type Key = (Epoch, Shard, Version);
    type KeyCodec = (EpochCodec, ShardCodec, NumberCodec<Version>);
    type Prefix = SubstatesUnprunedDownedValuesIndexPrefix;
    type Value = Vec<SubstateAddress>;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::SUBSTATES
    }
}

pub struct UnprunedDownedValuesEpochQuery;

impl QueryCf for UnprunedDownedValuesEpochQuery {
    type Cf = UnprunedDownedValuesIndex;
    type Key = Epoch;
    type KeyCodec = EpochCodec;
}
