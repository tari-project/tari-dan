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

use std::fmt::{Display, Formatter};

use serde::Serialize;
use tari_dan_common_types::{quorum_certificate::QuorumCertificate, NodeAddressable, ShardId, ValidatorMetadata};

use crate::models::{HotStuffMessageType, HotStuffTreeNode};

// TODO: convert to enum
#[derive(Debug, Clone, Serialize)]
pub struct HotStuffMessage<TPayload, TAddr> {
    message_type: HotStuffMessageType,
    // The high qc: used for new view messages
    high_qc: Option<QuorumCertificate<TAddr>>,
    node: Option<HotStuffTreeNode<TAddr, TPayload>>,
    shard: ShardId,
    // Used for broadcasting the payload in new view
    new_view_payload: Option<TPayload>,
}

impl<TPayload, TAddr> Default for HotStuffMessage<TPayload, TAddr> {
    fn default() -> Self {
        Self {
            message_type: Default::default(),
            high_qc: Default::default(),
            node: Default::default(),
            shard: ShardId::zero(),
            new_view_payload: None,
        }
    }
}

impl<TPayload, TAddr: Clone> HotStuffMessage<TPayload, TAddr> {
    pub fn new(
        message_type: HotStuffMessageType,
        high_qc: Option<QuorumCertificate<TAddr>>,
        node: Option<HotStuffTreeNode<TAddr, TPayload>>,
        shard: ShardId,
        new_view_payload: Option<TPayload>,
    ) -> Self {
        Self {
            message_type,
            high_qc,
            node,
            shard,
            new_view_payload,
        }
    }

    pub fn new_view(high_qc: QuorumCertificate<TAddr>, shard: ShardId, payload: TPayload) -> Self {
        Self {
            message_type: HotStuffMessageType::NewView,
            high_qc: Some(high_qc),
            shard,
            node: None,
            // Traditional hotstuff does not include broadcasting a payload at the same time,
            // but if this is a view for a specific payload, then it can be sent to the leader as
            // an attachment
            new_view_payload: Some(payload),
        }
    }

    pub fn new_proposal(node: HotStuffTreeNode<TAddr, TPayload>, shard: ShardId) -> Self {
        Self {
            message_type: HotStuffMessageType::Proposal,
            shard,
            node: Some(node),
            ..Default::default()
        }
    }

    pub fn high_qc(&self) -> Option<QuorumCertificate<TAddr>> {
        self.high_qc.clone()
    }

    pub fn new_view_payload(&self) -> Option<&TPayload> {
        self.new_view_payload.as_ref()
    }

    pub fn shard(&self) -> ShardId {
        self.shard
    }

    pub fn node(&self) -> Option<&HotStuffTreeNode<TAddr, TPayload>> {
        self.node.as_ref()
    }

    pub fn message_type(&self) -> HotStuffMessageType {
        self.message_type
    }

    pub fn add_partial_sig(&mut self, _validator_metadata: ValidatorMetadata) {
        todo!()
    }

    pub fn partial_sig(&self) -> Option<&ValidatorMetadata> {
        todo!()
    }
}

impl<TPayload, TAddr: NodeAddressable> Display for HotStuffMessage<TPayload, TAddr> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HSMessage {{ message_type: {:?}, node height: {:?}, payload height: {:?}, shard: {} }}",
            self.message_type,
            self.node.as_ref().map(|n| n.height()),
            self.node.as_ref().map(|n| n.payload_height()),
            self.shard
        )
    }
}
