//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::convert::{TryFrom, TryInto};

use anyhow::anyhow;
use tari_common_types::types::PublicKey;
use tari_comms::types::CommsPublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{ShardId, SubstateState};
use tari_dan_core::models::{
    vote_message::VoteMessage,
    HotStuffMessage,
    HotStuffTreeNode,
    Node,
    ObjectPledge,
    QuorumCertificate,
    QuorumDecision,
    ShardVote,
    TariDanPayload,
    TreeNodeHash,
    ValidatorSignature,
};

use crate::p2p::proto;

// -------------------------------- VoteMessage -------------------------------- //

impl From<VoteMessage> for proto::consensus::VoteMessage {
    fn from(msg: VoteMessage) -> Self {
        Self {
            local_node_hash: msg.local_node_hash().as_bytes().to_vec(),
            shard_id: msg.shard().as_bytes().to_vec(),
            decision: i32::from(msg.decision().as_u8()),
            all_shard_nodes: msg.all_shard_nodes().iter().map(|n| n.clone().into()).collect(),
            signature: msg.signature().to_bytes(),
        }
    }
}

impl TryFrom<proto::consensus::VoteMessage> for VoteMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::VoteMessage) -> Result<Self, Self::Error> {
        Ok(VoteMessage::with_signature(
            TreeNodeHash::try_from(value.local_node_hash)?,
            ShardId::from_bytes(&value.shard_id)?,
            QuorumDecision::from_u8(u8::try_from(value.decision)?)?,
            value
                .all_shard_nodes
                .into_iter()
                .map(|n| n.try_into())
                .collect::<Result<Vec<_>, _>>()?,
            ValidatorSignature::from_bytes(&value.signature)?,
        ))
    }
}

// -------------------------------- HotstuffMessage -------------------------------- //

impl From<HotStuffMessage<TariDanPayload, CommsPublicKey>> for proto::consensus::HotStuffMessage {
    fn from(source: HotStuffMessage<TariDanPayload, CommsPublicKey>) -> Self {
        Self {
            message_type: i32::from(source.message_type().as_u8()),
            node: source.node().map(|n| n.clone().into()),
            high_qc: source.high_qc().map(|h| h.into()),
            shard: source.shard().as_bytes().to_vec(),
            new_view_payload: source.new_view_payload().map(|p| p.clone().into()),
        }
    }
}

impl TryFrom<proto::consensus::HotStuffMessage> for HotStuffMessage<TariDanPayload, CommsPublicKey> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::HotStuffMessage) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.message_type.try_into()?,
            value.high_qc.map(|h| h.try_into()).transpose()?,
            value.node.map(|n| n.try_into()).transpose()?,
            Some(value.shard.try_into()?),
            value.new_view_payload.map(|p| p.try_into()).transpose()?,
        ))
    }
}

// -------------------------------- HotStuffTreeNode -------------------------------- //

impl TryFrom<proto::consensus::HotStuffTreeNode> for HotStuffTreeNode<CommsPublicKey> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::HotStuffTreeNode) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.parent.try_into()?,
            value.shard.try_into()?,
            value.height.into(),
            value.payload.try_into()?,
            value.payload_height.into(),
            value
                .local_pledges
                .iter()
                .map(|lp| lp.clone().try_into())
                .collect::<Result<_, _>>()?,
            value.epoch.into(),
            PublicKey::from_bytes(value.proposed_by.as_slice())?,
            value
                .justify
                .map(|j| j.try_into())
                .transpose()?
                .ok_or_else(|| anyhow!("Justify is required"))?,
        ))
    }
}

impl From<HotStuffTreeNode<CommsPublicKey>> for proto::consensus::HotStuffTreeNode {
    fn from(source: HotStuffTreeNode<CommsPublicKey>) -> Self {
        Self {
            parent: Vec::from(source.parent().as_bytes()),
            payload: source.payload().as_bytes().to_vec(),
            height: source.height().as_u64(),
            shard: source.shard().as_bytes().to_vec(),
            payload_height: source.payload_height().as_u64(),
            local_pledges: source.local_pledges().iter().map(|p| p.clone().into()).collect(),
            epoch: source.epoch().as_u64(),
            proposed_by: source.proposed_by().as_bytes().to_vec(),
            justify: Some(source.justify().clone().into()),
        }
    }
}

// -------------------------------- QuorumCertificate -------------------------------- //

impl From<QuorumCertificate> for proto::consensus::QuorumCertificate {
    fn from(source: QuorumCertificate) -> Self {
        Self {
            payload_id: source.payload_id().as_bytes().to_vec(),
            payload_height: source.payload_height().as_u64(),
            local_node_hash: source.local_node_hash().as_bytes().to_vec(),
            local_node_height: source.local_node_height().as_u64(),
            shard: source.shard().as_bytes().to_vec(),
            epoch: source.epoch().as_u64(),
            decision: match source.decision() {
                QuorumDecision::Accept => 1,
                QuorumDecision::Reject => 0,
            },
            all_shard_nodes: source.all_shard_nodes().iter().map(|p| p.clone().into()).collect(),
            signatures: source.signatures().iter().map(|p| p.clone().into()).collect(),
        }
    }
}

impl TryFrom<proto::consensus::QuorumCertificate> for QuorumCertificate {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::QuorumCertificate) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.payload_id.try_into()?,
            value.payload_height.into(),
            value.local_node_hash.try_into()?,
            value.local_node_height.into(),
            value.shard.try_into()?,
            value.epoch.into(),
            match value.decision {
                0 => QuorumDecision::Reject,
                1 => QuorumDecision::Accept,
                _ => return Err(anyhow!("Invalid decision")),
            },
            value
                .all_shard_nodes
                .iter()
                .map(|s| s.clone().try_into())
                .collect::<Result<_, _>>()?,
            value
                .signatures
                .iter()
                .map(|v| v.clone().try_into())
                .collect::<Result<_, _>>()?,
        ))
    }
}

// -------------------------------- ShardVote -------------------------------- //

impl From<ShardVote> for proto::consensus::ShardVote {
    fn from(s: ShardVote) -> Self {
        Self {
            shard_id: s.shard_id.into(),
            node_hash: s.node_hash.into(),
            pledges: s.pledges.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<proto::consensus::ShardVote> for ShardVote {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ShardVote) -> Result<Self, Self::Error> {
        Ok(Self {
            shard_id: value.shard_id.try_into()?,
            node_hash: value.node_hash.try_into()?,
            pledges: value
                .pledges
                .iter()
                .map(|p| p.clone().try_into())
                .collect::<Result<_, _>>()?,
        })
    }
}

// -------------------------------- ObjectPledge -------------------------------- //
impl TryFrom<proto::consensus::ObjectPledge> for ObjectPledge {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ObjectPledge) -> Result<Self, Self::Error> {
        Ok(Self {
            shard_id: value.shard_id.try_into()?,
            current_state: value
                .current_state
                .map(|s| s.try_into())
                .ok_or_else(|| anyhow!("current_state is required"))??,
            pledged_to_payload: value.pledged_to_payload.try_into()?,
            pledged_until: value.pledged_until.into(),
        })
    }
}

impl From<ObjectPledge> for proto::consensus::ObjectPledge {
    fn from(source: ObjectPledge) -> Self {
        Self {
            shard_id: source.shard_id.as_bytes().to_vec(),
            current_state: Some(source.current_state.into()),
            pledged_to_payload: source.pledged_to_payload.as_bytes().to_vec(),
            pledged_until: source.pledged_until.as_u64(),
        }
    }
}

// -------------------------------- SubstateState -------------------------------- //
impl TryFrom<proto::consensus::SubstateState> for SubstateState {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::SubstateState) -> Result<Self, Self::Error> {
        use proto::consensus::substate_state::State;
        match value.state {
            Some(State::DoesNotExist(_)) => Ok(Self::DoesNotExist),
            Some(State::Up(up)) => Ok(Self::Up {
                created_by: up.created_by.try_into()?,
                data: up.data,
            }),
            Some(State::Down(down)) => Ok(Self::Down {
                deleted_by: down.deleted_by.try_into()?,
            }),
            None => Err(anyhow!("SubstateState missing")),
        }
    }
}

impl From<SubstateState> for proto::consensus::SubstateState {
    fn from(source: SubstateState) -> Self {
        use proto::consensus::substate_state::State;
        match source {
            SubstateState::DoesNotExist => Self {
                state: Some(State::DoesNotExist(true)),
            },
            SubstateState::Up { created_by, data } => Self {
                state: Some(State::Up(proto::consensus::UpState {
                    created_by: created_by.as_bytes().to_vec(),
                    data,
                })),
            },
            SubstateState::Down { deleted_by } => Self {
                state: Some(State::Down(proto::consensus::DownState {
                    deleted_by: deleted_by.as_bytes().to_vec(),
                })),
            },
        }
    }
}

// -------------------------------- ValidatorSignature -------------------------------- //

impl TryFrom<proto::consensus::ValidatorSignature> for ValidatorSignature {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ValidatorSignature) -> Result<Self, Self::Error> {
        Ok(Self { signer: value.signer })
    }
}

impl From<ValidatorSignature> for proto::consensus::ValidatorSignature {
    fn from(value: ValidatorSignature) -> Self {
        Self { signer: value.signer }
    }
}

// -------------------------------- TariDanPayload -------------------------------- //

impl TryFrom<proto::consensus::TariDanPayload> for TariDanPayload {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::TariDanPayload) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value
                .transaction
                .map(|s| s.try_into())
                .transpose()?
                .ok_or_else(|| anyhow!("transaction is missing"))?,
        ))
    }
}

impl From<TariDanPayload> for proto::consensus::TariDanPayload {
    fn from(source: TariDanPayload) -> Self {
        Self {
            transaction: Some(source.transaction().clone().into()),
        }
    }
}

// -------------------------------- Node -------------------------------- //
impl From<Node> for proto::transaction::Node {
    fn from(node: Node) -> Self {
        Self {
            hash: node.hash().as_bytes().to_vec(),
            parent: node.parent().as_bytes().to_vec(),
            height: node.height(),
            is_committed: node.is_committed(),
        }
    }
}

impl TryFrom<proto::transaction::Node> for Node {
    type Error = anyhow::Error;

    fn try_from(node: proto::transaction::Node) -> Result<Self, Self::Error> {
        let hash = TreeNodeHash::try_from(node.hash)?;
        let parent = TreeNodeHash::try_from(node.parent)?;
        let height = node.height;
        let is_committed = node.is_committed;

        Ok(Self::new(hash, parent, height, is_committed))
    }
}

// // -------------------------------- KeyValue -------------------------------- //
//
// impl From<KeyValue> for proto::validator_node::KeyValue {
//     fn from(kv: KeyValue) -> Self {
//         Self {
//             key: kv.key,
//             value: kv.value,
//         }
//     }
// }
//
// impl TryFrom<proto::validator_node::KeyValue> for KeyValue {
//     type Error = anyhow::Error;
//
//     fn try_from(kv: proto::validator_node::KeyValue) -> Result<Self, Self::Error> {
//         if kv.key.is_empty() {
//             return Err(anyhow!("KeyValue: key cannot be empty"));
//         }
//
//         Ok(Self {
//             key: kv.key,
//             value: kv.value,
//         })
//     }
// }

// -------------------------------- SubstateState ------------------------------ //

// impl TryFrom<proto::common::SubstateState> for SubstateState {
//     type Error = anyhow::Error;
//
//     fn try_from(request: proto::common::SubstateState) -> Result<Self, Self::Error> {
//         let result = match request.substate_state_type {
//             0 => SubstateState::DoesNotExist,
//             1 => SubstateState::Up {
//                 created_by: PayloadId::try_from(request.created_by.unwrap())?,
//                 data: request.data,
//             },
//             2 => SubstateState::Down {
//                 deleted_by: PayloadId::try_from(request.deleted_by.unwrap())?,
//             },
//             _ => return Err(anyhow!("bad gRPC substate state parsing")),
//         };
//
//         Ok(result)
//     }
// }
//
// impl From<SubstateState> for proto::common::SubstateState {
//     fn from(value: SubstateState) -> Self {
//         let mut result = proto::common::SubstateState::default();
//         match value {
//             SubstateState::DoesNotExist => {
//                 result.substate_state_type = 0;
//             },
//             SubstateState::Up { data, created_by } => {
//                 result.substate_state_type = 1;
//                 result.data = data;
//                 result.created_by = Some(proto::common::PayloadId::from(created_by));
//             },
//             SubstateState::Down { deleted_by } => {
//                 result.substate_state_type = 2;
//                 result.deleted_by = Some(proto::common::PayloadId::from(deleted_by));
//             },
//         }
//
//         result
//     }
// }
