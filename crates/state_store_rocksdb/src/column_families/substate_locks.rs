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
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{Epoch, NodeHeight};
use tari_ootle_storage::consensus_models::SubstateLock;

use crate::{
    codecs::{BlockIdCodec, DefaultCodec, KeyPrefix},
    column_families::cf_names,
    prefixed,
    traits::Cf,
};

/// The locks a block granted that are still held.
#[derive(Debug, Clone, Serialize, Encode, Decode, CborLen)]
pub struct BlockLockSet {
    #[n(0)]
    pub block_epoch: Epoch,
    #[n(1)]
    pub block_height: NodeHeight,
    #[n(2)]
    pub substates: Vec<SubstateLockGrants>,
}

/// A substate's locks in the order the block granted them.
#[derive(Debug, Clone, Serialize, Encode, Decode, CborLen)]
pub struct SubstateLockGrants {
    #[n(0)]
    pub substate_id: SubstateId,
    #[n(1)]
    pub locks: Vec<SubstateLock>,
}

prefixed!(BlockLockSetPrefix, KeyPrefix::SubstateLockSets);

pub struct BlockLockSetCf;

impl Cf for BlockLockSetCf {
    type Key = BlockId;
    type KeyCodec = BlockIdCodec;
    type Prefix = BlockLockSetPrefix;
    type Value = BlockLockSet;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::SUBSTATES
    }
}
