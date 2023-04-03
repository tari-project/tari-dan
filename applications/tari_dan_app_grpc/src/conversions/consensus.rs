//   Copyright 2023. The Tari Project
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
use tari_dan_common_types::{
    ObjectPledge,
    QuorumCertificate,
    QuorumDecision,
    ShardPledge,
    SubstateState,
    TreeNodeHash,
    ValidatorMetadata,
};
use tari_dan_core::models::{vote_message::VoteMessage, HotStuffMessage, HotStuffTreeNode, Node, TariDanPayload};
use tari_engine_types::substate::{Substate, SubstateAddress};

use crate::proto;

// -------------------------------- VoteMessage -------------------------------- //

impl From<VoteMessage> for proto::consensus::VoteMessage {
    fn from(msg: VoteMessage) -> Self {
        Self {
            local_node_hash: msg.local_node_hash().as_bytes().to_vec(),
            decision: i32::from(msg.decision().as_u8()),
            all_shard_pledges: msg.all_shard_pledges().iter().map(|n| n.clone().into()).collect(),
            validator_metadata: Some(msg.validator_metadata().clone().into()),
        }
    }
}

impl TryFrom<proto::consensus::VoteMessage> for VoteMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::VoteMessage) -> Result<Self, Self::Error> {
        let metadata = value
            .validator_metadata
            .ok_or_else(|| anyhow!("Validator metadata is missing"))?;
        Ok(VoteMessage::with_validator_metadata(
            TreeNodeHash::try_from(value.local_node_hash)?,
            QuorumDecision::from_u8(u8::try_from(value.decision)?)?,
            value
                .all_shard_pledges
                .into_iter()
                .map(|n| n.try_into())
                .collect::<Result<_, _>>()?,
            metadata.try_into()?,
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
            value.shard.try_into()?,
            value.new_view_payload.map(|p| p.try_into()).transpose()?,
        ))
    }
}

// -------------------------------- HotStuffTreeNode -------------------------------- //

impl TryFrom<proto::consensus::HotStuffTreeNode> for HotStuffTreeNode<CommsPublicKey, TariDanPayload> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::HotStuffTreeNode) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.parent.try_into()?,
            value.shard.try_into()?,
            value.height.into(),
            value.payload_id.try_into()?,
            value.payload.map(|a| a.try_into().unwrap()),
            value.payload_height.into(),
            value.leader_round as u32,
            value.local_pledge.map(|lp| lp.try_into()).transpose()?,
            value.epoch.into(),
            value.proposed_by,
            value
                .justify
                .map(|j| j.try_into())
                .transpose()?
                .ok_or_else(|| anyhow!("Justify is required"))?,
        ))
    }
}

impl From<HotStuffTreeNode<CommsPublicKey, TariDanPayload>> for proto::consensus::HotStuffTreeNode {
    fn from(source: HotStuffTreeNode<CommsPublicKey, TariDanPayload>) -> Self {
        Self {
            parent: Vec::from(source.parent().as_bytes()),
            payload: source.payload().map(|a| a.clone().into()),
            height: source.height().as_u64(),
            shard: source.shard().as_bytes().to_vec(),
            payload_id: source.payload_id().as_bytes().to_vec(),
            payload_height: source.payload_height().as_u64(),
            leader_round: u64::from(source.leader_round()),
            local_pledge: source.local_pledge().map(|p| p.clone().into()),
            epoch: source.epoch().as_u64(),
            proposed_by: source.proposed_by().as_bytes().to_vec(),
            justify: Some(source.justify().clone().into()),
        }
    }
}

// -------------------------------- QuorumCertificate -------------------------------- //

impl From<QuorumCertificate<PublicKey>> for proto::consensus::QuorumCertificate {
    fn from(source: QuorumCertificate<PublicKey>) -> Self {
        Self {
            payload_id: source.payload_id().as_bytes().to_vec(),
            payload_height: source.payload_height().as_u64(),
            local_node_hash: source.node_hash().as_bytes().to_vec(),
            local_node_height: source.node_height().as_u64(),
            shard: source.shard().as_bytes().to_vec(),
            epoch: source.epoch().as_u64(),
            decision: match source.decision() {
                QuorumDecision::Accept => 0,
                QuorumDecision::Reject(ref reason) => reason.as_u8().into(),
            },
            proposed_by: source.proposed_by().as_bytes().to_vec(),
            all_shard_pledges: source.all_shard_pledges().iter().map(|p| p.clone().into()).collect(),
            validators_metadata: source.validators_metadata().iter().map(|p| p.clone().into()).collect(),
        }
    }
}

impl TryFrom<proto::consensus::QuorumCertificate> for QuorumCertificate<PublicKey> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::QuorumCertificate) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.payload_id.try_into()?,
            value.payload_height.into(),
            value.local_node_hash.try_into()?,
            value.local_node_height.into(),
            value.shard.try_into()?,
            value.epoch.into(),
            PublicKey::from_vec(&value.proposed_by)?,
            QuorumDecision::from_u8(value.decision.try_into()?)?,
            value
                .all_shard_pledges
                .iter()
                .map(|s| s.clone().try_into())
                .collect::<Result<_, _>>()?,
            value
                .validators_metadata
                .iter()
                .map(|v| v.clone().try_into())
                .collect::<Result<_, _>>()?,
        ))
    }
}

// -------------------------------- ShardPledge -------------------------------- //

impl From<ShardPledge> for proto::consensus::ShardPledge {
    fn from(s: ShardPledge) -> Self {
        Self {
            shard_id: s.shard_id.into(),
            node_hash: s.node_hash.into(),
            pledge: Some(s.pledge.into()),
        }
    }
}

impl TryFrom<proto::consensus::ShardPledge> for ShardPledge {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ShardPledge) -> Result<Self, Self::Error> {
        Ok(Self {
            shard_id: value.shard_id.try_into()?,
            node_hash: value.node_hash.try_into()?,
            pledge: value
                .pledge
                .map(|p| p.try_into())
                .transpose()?
                .ok_or_else(|| anyhow!("Pledge is required"))?,
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
        })
    }
}

impl From<ObjectPledge> for proto::consensus::ObjectPledge {
    fn from(source: ObjectPledge) -> Self {
        Self {
            shard_id: source.shard_id.as_bytes().to_vec(),
            current_state: Some(source.current_state.into()),
            pledged_to_payload: source.pledged_to_payload.as_bytes().to_vec(),
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
                address: SubstateAddress::from_bytes(&up.address)?,
                created_by: up.created_by.try_into()?,
                data: Substate::from_bytes(&up.data)?,
                fees_accrued: up.fees_accrued,
            }),
            Some(State::Down(down)) => Ok(Self::Down {
                deleted_by: down.deleted_by.try_into()?,
                fees_accrued: down.fees_accrued,
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
            SubstateState::Up {
                created_by,
                data,
                address,
                fees_accrued,
            } => Self {
                state: Some(State::Up(proto::consensus::UpState {
                    address: address.to_bytes(),
                    created_by: created_by.as_bytes().to_vec(),
                    data: data.to_bytes(),
                    fees_accrued,
                })),
            },
            SubstateState::Down {
                deleted_by,
                fees_accrued,
            } => Self {
                state: Some(State::Down(proto::consensus::DownState {
                    deleted_by: deleted_by.as_bytes().to_vec(),
                    fees_accrued,
                })),
            },
        }
    }
}

// -------------------------------- ValidatorMetadata -------------------------------- //

impl From<ValidatorMetadata> for proto::consensus::ValidatorMetadata {
    fn from(msg: ValidatorMetadata) -> Self {
        let merkle_proof = msg.encode_merkle_proof();
        Self {
            public_key: msg.public_key.to_vec(),
            vn_shard_key: msg.vn_shard_key.as_bytes().to_vec(),
            signature: Some(msg.signature.into()),
            merkle_proof,
            merkle_leaf_index: msg.merkle_leaf_index,
        }
    }
}

impl TryFrom<proto::consensus::ValidatorMetadata> for ValidatorMetadata {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ValidatorMetadata) -> Result<Self, Self::Error> {
        Ok(ValidatorMetadata {
            public_key: PublicKey::from_bytes(&value.public_key)?,
            vn_shard_key: value.vn_shard_key.try_into()?,
            signature: value
                .signature
                .map(TryFrom::try_from)
                .transpose()?
                .ok_or_else(|| anyhow!("ValidatorMetadata missing signature"))?,
            merkle_proof: ValidatorMetadata::decode_merkle_proof(&value.merkle_proof)?,
            merkle_leaf_index: value.merkle_leaf_index,
        })
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
