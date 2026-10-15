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
use serde::{Deserialize, Serialize};
use tari_consensus_types::BlockId;
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_storage::consensus_models::{Block, ForeignProposal};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    codecs::{BlockIdCodec, DefaultCodec, EpochCodec, KeyPrefix, NodeHeightCodec, PublicKeyCodec, UnitCodec},
    column_families::cf_names,
    prefixed,
    traits::{Cf, QueryCf},
};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct ParkedBlockData {
    #[n(0)]
    pub block: Block,
    #[n(1)]
    pub foreign_proposals: Vec<ForeignProposal>,
}

prefixed!(ParkedBlockPrefix, KeyPrefix::ParkedBlocks);

pub struct ParkedBlockCf;

impl Cf for ParkedBlockCf {
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
    type Prefix = ParkedBlockPrefix;
    type Value = ParkedBlockData;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::BLOCK
    }
}

prefixed!(ParkedBlockEpochHeightPrefix, KeyPrefix::ParkedBlockEpochHeightIndex);

/// Every parked block, ordered by the view it was proposed for and then by its proposer. Written and deleted together
/// with the [`ParkedBlockCf`] row.
pub struct ParkedBlockEpochHeightIndex;

impl Cf for ParkedBlockEpochHeightIndex {
    type Key = (Epoch, NodeHeight, RistrettoPublicKeyBytes, BlockId);
    type KeyCodec = (EpochCodec, NodeHeightCodec, PublicKeyCodec, BlockIdCodec);
    type Prefix = ParkedBlockEpochHeightPrefix;
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        cf_names::BLOCK
    }
}

pub struct ByEpochHeightQuery;

impl QueryCf for ByEpochHeightQuery {
    type Cf = ParkedBlockEpochHeightIndex;
    type Key = (Epoch, NodeHeight);
    type KeyCodec = (EpochCodec, NodeHeightCodec);
}

pub struct ByProposerQuery;

impl QueryCf for ByProposerQuery {
    type Cf = ParkedBlockEpochHeightIndex;
    type Key = (Epoch, NodeHeight, RistrettoPublicKeyBytes);
    type KeyCodec = (EpochCodec, NodeHeightCodec, PublicKeyCodec);
}

pub fn epoch_height_index_key(block: &Block) -> (Epoch, NodeHeight, RistrettoPublicKeyBytes, BlockId) {
    (block.epoch(), block.height(), *block.proposed_by(), *block.id())
}

#[derive(Debug, Clone, Serialize, Encode, CborLen)]
pub struct ParkedBlockDataRef<'a> {
    #[b(0)]
    pub block: &'a Block,
    #[b(1)]
    pub foreign_proposals: &'a [ForeignProposal],
}
