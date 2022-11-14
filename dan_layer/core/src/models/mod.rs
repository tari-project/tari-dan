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

use std::{
    cmp::Ordering,
    convert::{Infallible, TryFrom},
    fmt::{Debug, Display, Formatter},
    ops::Add,
};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;

mod base_layer_metadata;
mod base_layer_output;
mod committee;
pub mod domain_events;
mod error;
mod hot_stuff_message;
mod hot_stuff_tree_node;
mod leaf_node;
mod node;
mod payload;
mod quorum_certificate;
mod sidechain_metadata;
mod tari_dan_payload;
mod tree_node_hash;
mod validator_node;
mod view;
mod view_id;
pub mod vote_message;

pub use base_layer_metadata::BaseLayerMetadata;
pub use base_layer_output::BaseLayerOutput;
pub use committee::Committee;
pub use error::ModelError;
pub use hot_stuff_message::HotStuffMessage;
pub use hot_stuff_tree_node::HotStuffTreeNode;
pub use leaf_node::LeafNode;
pub use node::Node;
pub use payload::Payload;
pub use quorum_certificate::{QuorumCertificate, QuorumDecision};
pub use sidechain_metadata::SidechainMetadata;
use tari_dan_common_types::{serde_with, PayloadId, ShardId, SubstateState};
pub use tari_dan_payload::{CheckpointData, TariDanPayload};
pub use tree_node_hash::TreeNodeHash;
pub use validator_node::ValidatorNode;
pub use view::View;
pub use view_id::ViewId;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct NodeHeight(pub u64);

impl NodeHeight {
    pub fn as_u64(self) -> u64 {
        self.0
    }

    fn to_le_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }
}

impl Add for NodeHeight {
    type Output = NodeHeight;

    fn add(self, rhs: Self) -> Self::Output {
        NodeHeight(self.0 + rhs.0)
    }
}

impl PartialOrd for NodeHeight {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl From<u64> for NodeHeight {
    fn from(height: u64) -> Self {
        NodeHeight(height)
    }
}

impl Display for NodeHeight {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeHeight({})", self.0)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ObjectPledge {
    pub shard_id: ShardId,
    pub current_state: SubstateState,
    // pub current_state_hash: FixedHash,
    pub pledged_to_payload: PayloadId,
    pub pledged_until: NodeHeight,
}

// TODO: encapsulate
pub struct InstructionCaller {
    pub owner_token_id: TokenId,
}

impl InstructionCaller {
    pub fn _owner_token_id(&self) -> &TokenId {
        &self.owner_token_id
    }
}

#[derive(Clone, Debug, Hash)]
pub struct TokenId(pub Vec<u8>);

impl TokenId {
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl AsRef<[u8]> for TokenId {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

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
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(HotStuffMessageType::NewView),
            1 => Ok(HotStuffMessageType::Proposal),
            _ => Err("Not a value message type".to_string()),
        }
    }
}

impl TryFrom<i32> for HotStuffMessageType {
    type Error = anyhow::Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(HotStuffMessageType::NewView),
            1 => Ok(HotStuffMessageType::Proposal),
            _ => Err(anyhow!("Not a value message type")),
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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ValidatorSignature {
    #[serde(with = "serde_with::hex")]
    pub public_key: Vec<u8>,
    #[serde(with = "serde_with::hex")]
    pub signature: Vec<u8>,
}

impl ValidatorSignature {
    // TODO: implement from bytes with correct error
    pub fn from_bytes(public_key_bytes: &[u8], signature_bytes: &[u8]) -> Result<Self, Infallible> {
        Ok(Self {
            public_key: Vec::from(public_key_bytes),
            signature: Vec::from(signature_bytes),
        })
    }

    pub fn combine(&self, other: &ValidatorSignature) -> ValidatorSignature {
        other.clone()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        [self.public_key.clone(), self.signature.clone()].concat()
    }
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ShardVote {
    pub shard_id: ShardId,
    pub node_hash: TreeNodeHash,
    pub pledge: Option<ObjectPledge>,
}

#[derive(Debug, Serialize)]
pub struct RecentTransaction {
    pub payload_id: Vec<u8>,
    pub shard: Vec<u8>,
    pub height: i64,
    pub payload_height: i64,
    pub total_votes: i64,
    pub total_leader_proposals: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateShardData {
    shard: ShardId,
    substate: SubstateState,
    height: NodeHeight,
    tree_node_hash: Option<TreeNodeHash>,
    payload_id: PayloadId,
    certificate: Option<QuorumCertificate>,
}

impl SubstateShardData {
    pub fn new(
        shard: ShardId,
        substate: SubstateState,
        height: NodeHeight,
        tree_node_hash: Option<TreeNodeHash>,
        payload_id: PayloadId,
        certificate: Option<QuorumCertificate>,
    ) -> Self {
        Self {
            shard,
            substate,
            height,
            tree_node_hash,
            payload_id,
            certificate,
        }
    }

    pub fn shard(&self) -> ShardId {
        self.shard
    }

    pub fn substate(&self) -> &SubstateState {
        &self.substate
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn tree_node_hash(&self) -> Option<TreeNodeHash> {
        self.tree_node_hash
    }

    pub fn payload_id(&self) -> PayloadId {
        self.payload_id
    }

    pub fn certificate(&self) -> &Option<QuorumCertificate> {
        &self.certificate
    }
}
