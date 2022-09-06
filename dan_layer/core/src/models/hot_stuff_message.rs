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

use tari_common_types::types::FixedHash;
use tari_dan_common_types::ShardId;

use crate::{
    models::{
        dan_layer_models_hasher,
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
#[derive(Debug, Clone)]
pub struct HotStuffMessage<TPayload: Payload, TAddr: NodeAddressable> {
    message_type: HotStuffMessageType,
    justify: Option<QuorumCertificate>,
    // The high qc: used for new view messages
    high_qc: Option<QuorumCertificate>,
    node: Option<HotStuffTreeNode<TAddr>>,
    // node_hash: Option<TreeNodeHash>,
    // partial_sig: Option<ValidatorSignature>,
    // checkpoint_signature: Option<SignerSignature>,
    // contract_id: Option<FixedHash>,
    shard: Option<ShardId>,
    epoch: Option<u32>,
    // Used for broadcasting the payload in new view
    new_view_payload: Option<TPayload>,
}

impl<TPayload: Payload, TAddr: NodeAddressable> Default for HotStuffMessage<TPayload, TAddr> {
    fn default() -> Self {
        Self {
            message_type: Default::default(),
            justify: Default::default(),
            high_qc: Default::default(),
            node: Default::default(),
            shard: Default::default(),
            epoch: None,
            new_view_payload: None,
        }
    }
}

impl<TPayload: Payload, TAddr: NodeAddressable> HotStuffMessage<TPayload, TAddr> {
    pub fn new(
        message_type: HotStuffMessageType,
        justify: Option<QuorumCertificate>,
        node: Option<HotStuffTreeNode<TAddr>>,
        node_hash: Option<TreeNodeHash>,
        partial_sig: Option<ValidatorSignature>,
        contract_id: FixedHash,
    ) -> Self {
        todo!();
        // Self {
        //     message_type,
        //     justify,
        //     node,
        //     high_qc: None,
        //     shard: None,
        //     new_view_payload: None,
        // }
    }

    pub fn new_view(high_qc: QuorumCertificate, shard: ShardId, payload: Option<TPayload>) -> Self {
        Self {
            message_type: HotStuffMessageType::NewView,
            high_qc: Some(high_qc),
            shard: Some(shard),
            justify: None,
            node: None,
            epoch: None,
            // Traditional hotstuff does not include broadcasting a payload at the same time,
            // but if this is a view for a specific payload, then it can be sent to the leader as
            // an attachment
            new_view_payload: payload,
        }
    }

    pub fn generic(node: HotStuffTreeNode<TAddr>, shard: ShardId) -> Self {
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

    pub fn node(&self) -> Option<&HotStuffTreeNode<TAddr>> {
        self.node.as_ref()
    }

    pub fn node_hash(&self) -> Option<&TreeNodeHash> {
        todo!()
    }

    pub fn message_type(&self) -> HotStuffMessageType {
        self.message_type
    }

    pub fn justify(&self) -> Option<&QuorumCertificate> {
        self.justify.as_ref()
    }

    pub fn matches(&self, message_type: HotStuffMessageType, view_id: ViewId) -> bool {
        // from hotstuf spec
        self.message_type() == message_type && view_id == self.view_number()
    }

    pub fn add_partial_sig(&mut self, signature: ValidatorSignature) {
        todo!()
    }

    pub fn partial_sig(&self) -> Option<&ValidatorSignature> {
        todo!()
    }
}
