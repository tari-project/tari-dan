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
use tari_common_types::types::FixedHash;
use tari_dan_common_types::ShardId;

use crate::{
    models::{
        HotStuffMessageType,
        HotStuffTreeNode,
        Payload,
        QuorumCertificate,
        TreeNodeHash,
        ValidatorSignature,
        ViewId,
    },
    services::infrastructure_services::NodeAddressable,
};

// TODO: convert to enum
#[derive(Debug, Clone, Serialize)]
pub struct HotStuffMessage<TPayload, TAddr> {
    message_type: HotStuffMessageType,
    // The high qc: used for new view messages
    high_qc: Option<QuorumCertificate>,
    node: Option<HotStuffTreeNode<TAddr, TPayload>>,
    shard: Option<ShardId>,
    // Used for broadcasting the payload in new view
    new_view_payload: Option<TPayload>,
}

impl<TPayload: Payload, TAddr: NodeAddressable> Default for HotStuffMessage<TPayload, TAddr> {
    fn default() -> Self {
        Self {
            message_type: Default::default(),
            high_qc: Default::default(),
            node: Default::default(),
            shard: Default::default(),
            new_view_payload: None,
        }
    }
}

impl<TPayload: Payload, TAddr: NodeAddressable> HotStuffMessage<TPayload, TAddr> {
    pub fn new(
        message_type: HotStuffMessageType,
        high_qc: Option<QuorumCertificate>,
        node: Option<HotStuffTreeNode<TAddr, TPayload>>,
        shard: Option<ShardId>,
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

    pub fn new_view(high_qc: QuorumCertificate, shard: ShardId, payload: Option<TPayload>) -> Self {
        Self {
            message_type: HotStuffMessageType::NewView,
            high_qc: Some(high_qc),
            shard: Some(shard),
            node: None,
            // Traditional hotstuff does not include broadcasting a payload at the same time,
            // but if this is a view for a specific payload, then it can be sent to the leader as
            // an attachment
            new_view_payload: payload,
        }
    }

    pub fn generic(node: HotStuffTreeNode<TAddr, TPayload>, shard: ShardId) -> Self {
        Self {
            message_type: HotStuffMessageType::Generic,
            shard: Some(shard),
            node: Some(node),
            ..Default::default()
        }
    }

    pub fn create_signature_challenge(&self) -> Vec<u8> {
        todo!()
        // let mut b = dan_layer_models_hasher::<Blake256>(HOT_STUFF_MESSAGE_LABEL)
        //     .chain(&[self.message_type.as_u8()])
        //     .chain(self.view_number.as_u64().to_le_bytes());
        // if let Some(ref node) = self.node {
        //     b = b.chain(node.calculate_hash().as_bytes());
        // } else if let Some(ref node_hash) = self.node_hash {
        //     b = b.chain(node_hash.as_bytes());
        // } else {
        // }
        // b.finalize().as_ref().to_vec()
    }

    pub fn view_number(&self) -> ViewId {
        todo!()
    }

    pub fn high_qc(&self) -> Option<QuorumCertificate> {
        self.high_qc.clone()
    }

    pub fn contract_id(&self) -> &FixedHash {
        todo!()
    }

    pub fn new_view_payload(&self) -> Option<&TPayload> {
        self.new_view_payload.as_ref()
    }

    pub fn shard(&self) -> ShardId {
        // TODO: remove unwrap, every message should have a shard
        self.shard.unwrap()
    }

    pub fn node(&self) -> Option<&HotStuffTreeNode<TAddr, TPayload>> {
        self.node.as_ref()
    }

    pub fn node_hash(&self) -> Option<&TreeNodeHash> {
        todo!()
    }

    pub fn message_type(&self) -> HotStuffMessageType {
        self.message_type
    }

    pub fn matches(&self, message_type: HotStuffMessageType, view_id: ViewId) -> bool {
        // from hotstuf spec
        self.message_type() == message_type && view_id == self.view_number()
    }

    pub fn add_partial_sig(&mut self, _signature: ValidatorSignature) {
        todo!()
    }

    pub fn partial_sig(&self) -> Option<&ValidatorSignature> {
        todo!()
    }
}

impl<TPayload: Payload, TAddr: NodeAddressable> Display for HotStuffMessage<TPayload, TAddr> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "HSMessage {{ message_type: {:?}, node height: {:?}, payload height: {:?}, shard: {:?} }}",
            self.message_type,
            self.node.as_ref().map(|n| n.height()),
            self.node.as_ref().map(|n| n.payload_height()),
            self.shard.as_ref().map(|s| s.to_string())
        )
    }
}
