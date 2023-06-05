// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{convert::TryFrom, fmt::Debug};

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;

mod base_layer_metadata;
mod committee;
mod hot_stuff_message;
mod hot_stuff_tree_node;
mod leaf_node;
mod node;
mod payload;
mod sidechain_metadata;
mod substate_shard_data;
mod tari_dan_payload;
mod vote_message;

pub use base_layer_metadata::BaseLayerMetadata;
pub use committee::Committee;
pub use hot_stuff_message::HotStuffMessage;
pub use hot_stuff_tree_node::{HotStuffTreeNode, HotstuffPhase};
pub use leaf_node::LeafNode;
pub use node::Node;
pub use payload::{Payload, PayloadResult};
pub use sidechain_metadata::SidechainMetadata;
pub use substate_shard_data::SubstateShardData;
use tari_dan_common_types::{NodeHeight, TreeNodeHash};
pub use tari_dan_payload::{CheckpointData, TariDanPayload};
// pub use validator_node::ValidatorNode;
pub use vote_message::VoteMessage;

use crate::StorageError;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize)]
pub enum HotStuffMessageType {
    NewView,
    Proposal,
}

impl Default for HotStuffMessageType {
    fn default() -> Self {
        Self::NewView
    }
}

impl HotStuffMessageType {
    pub fn as_u8(&self) -> u8 {
        match self {
            HotStuffMessageType::NewView => 0,
            HotStuffMessageType::Proposal => 1,
        }
    }
}

impl TryFrom<u8> for HotStuffMessageType {
    type Error = StorageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(HotStuffMessageType::NewView),
            1 => Ok(HotStuffMessageType::Proposal),
            _ => Err(StorageError::InvalidTypeCasting {
                reason: "Not a value message type".to_string(),
            }),
        }
    }
}

impl TryFrom<i32> for HotStuffMessageType {
    type Error = StorageError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(HotStuffMessageType::NewView),
            1 => Ok(HotStuffMessageType::Proposal),
            _ => Err(StorageError::InvalidTypeCasting {
                reason: "Not a value message type".to_string(),
            }),
        }
    }
}

pub trait ConsensusHash {
    fn consensus_hash(&self) -> FixedHash;
}

impl ConsensusHash for &str {
    fn consensus_hash(&self) -> FixedHash {
        let mut hash = [0u8; FixedHash::byte_size()];
        hash[..self.len()].copy_from_slice(self.as_bytes());
        hash.into()
    }
}

impl ConsensusHash for String {
    fn consensus_hash(&self) -> FixedHash {
        self.as_str().consensus_hash()
    }
}

pub trait Event: Clone + Send + Sync {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusWorkerState {
    Starting,
    Synchronizing,
    Prepare,
    PreCommit,
    Commit,
    Decide,
    NextView,
    Idle,
}

#[derive(Copy, Clone, Debug)]
pub struct ChainHeight(u64);

impl From<ChainHeight> for u64 {
    fn from(c: ChainHeight) -> Self {
        c.0
    }
}

impl From<u64> for ChainHeight {
    fn from(v: u64) -> Self {
        ChainHeight(v)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentTransaction {
    pub payload_id: Vec<u8>,
    pub timestamp: NaiveDateTime,
    pub meta: String,
    pub instructions: String,
}

// TODO: These should be well-formed structs, no SQL in core
#[derive(Debug, Serialize)]
pub struct SQLTransaction {
    pub node_hash: Vec<u8>,
    pub parent_node_hash: Vec<u8>,
    pub shard: Vec<u8>,
    pub height: i64,
    pub payload_height: i64,
    pub total_votes: i64,
    pub total_leader_proposals: i64,
    pub timestamp: NaiveDateTime,
    pub justify: String,
    pub proposed_by: Vec<u8>,
    pub leader_round: i64,
}

#[derive(Debug, Serialize)]
pub struct SQLSubstate {
    pub shard_id: Vec<u8>,
    pub address: String,
    pub version: i64,
    pub data: String,
    pub created_justify: String,
    pub destroyed_justify: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CurrentLeaderStates {
    pub payload_id: Vec<u8>,
    pub shard_id: Vec<u8>,
    pub leader_round: i64,
    pub leader: Vec<u8>,
    pub timestamp: NaiveDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimLeaderFees {
    pub justify_leader_public_key: String,
    pub created_at_epoch: i64,
    pub destroyed_at_epoch: Option<i64>,
    pub fee_paid_for_created_justify: i64,
    pub fee_paid_for_destroyed_justify: i64,
}
