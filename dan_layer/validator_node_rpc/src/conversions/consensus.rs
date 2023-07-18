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

use std::{
    borrow::Borrow,
    convert::{TryFrom, TryInto},
};

use anyhow::anyhow;
use tari_bor::{decode_exact, encode};
use tari_common_types::types::PublicKey;
use tari_consensus::messages::{HotstuffMessage, NewViewMessage, ProposalMessage, VoteMessage};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{Epoch, NodeHeight, ShardId, ValidatorMetadata};
use tari_dan_storage::consensus_models::{
    BlockId,
    Command,
    Decision,
    Evidence,
    QuorumCertificate,
    QuorumDecision,
    TransactionAtom,
};
use tari_transaction::TransactionId;

use crate::proto;
// -------------------------------- HotstuffMessage -------------------------------- //

impl From<HotstuffMessage> for proto::consensus::HotStuffMessage {
    fn from(source: HotstuffMessage) -> Self {
        let message = match source {
            HotstuffMessage::NewView(msg) => proto::consensus::hot_stuff_message::Message::NewView(msg.into()),
            HotstuffMessage::Proposal(msg) => proto::consensus::hot_stuff_message::Message::Proposal(msg.into()),
            HotstuffMessage::Vote(msg) => proto::consensus::hot_stuff_message::Message::Vote(msg.into()),
        };
        Self { message: Some(message) }
    }
}

impl TryFrom<proto::consensus::HotStuffMessage> for HotstuffMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::HotStuffMessage) -> Result<Self, Self::Error> {
        let message = value.message.ok_or_else(|| anyhow!("Message is missing"))?;
        Ok(match message {
            proto::consensus::hot_stuff_message::Message::NewView(msg) => HotstuffMessage::NewView(msg.try_into()?),
            proto::consensus::hot_stuff_message::Message::Proposal(msg) => HotstuffMessage::Proposal(msg.try_into()?),
            proto::consensus::hot_stuff_message::Message::Vote(msg) => HotstuffMessage::Vote(msg.try_into()?),
        })
    }
}

//---------------------------------- NewView --------------------------------------------//

impl From<NewViewMessage> for proto::consensus::NewViewMessage {
    fn from(value: NewViewMessage) -> Self {
        Self {
            high_qc: Some(value.high_qc.into()),
        }
    }
}

impl TryFrom<proto::consensus::NewViewMessage> for NewViewMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::NewViewMessage) -> Result<Self, Self::Error> {
        Ok(NewViewMessage {
            high_qc: value.high_qc.ok_or_else(|| anyhow!("High QC is missing"))?.try_into()?,
        })
    }
}

//---------------------------------- ProposalMessage --------------------------------------------//

impl From<ProposalMessage> for proto::consensus::ProposalMessage {
    fn from(value: ProposalMessage) -> Self {
        Self {
            block: Some(value.block.into()),
        }
    }
}

impl TryFrom<proto::consensus::ProposalMessage> for ProposalMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ProposalMessage) -> Result<Self, Self::Error> {
        Ok(ProposalMessage {
            block: value.block.ok_or_else(|| anyhow!("Block is missing"))?.try_into()?,
        })
    }
}

// -------------------------------- VoteMessage -------------------------------- //

impl From<VoteMessage> for proto::consensus::VoteMessage {
    fn from(msg: VoteMessage) -> Self {
        Self {
            epoch: msg.epoch.as_u64(),
            block_id: msg.block_id.as_bytes().to_vec(),
            decision: i32::from(msg.decision.as_u8()),
            signature: Some(msg.signature.into()),
            merkle_proof: encode(&msg.merkle_proof).unwrap(),
        }
    }
}

impl TryFrom<proto::consensus::VoteMessage> for VoteMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::VoteMessage) -> Result<Self, Self::Error> {
        Ok(VoteMessage {
            epoch: Epoch(value.epoch),
            block_id: BlockId::try_from(value.block_id)?,
            decision: QuorumDecision::from_u8(u8::try_from(value.decision)?)
                .ok_or_else(|| anyhow!("Invalid decision byte {}", value.decision))?,
            signature: value
                .signature
                .ok_or_else(|| anyhow!("Signature is missing"))?
                .try_into()?,
            merkle_proof: decode_exact(&value.merkle_proof)?,
        })
    }
}

//---------------------------------- Block --------------------------------------------//

impl From<tari_dan_storage::consensus_models::Block> for proto::consensus::Block {
    fn from(value: tari_dan_storage::consensus_models::Block) -> Self {
        Self {
            height: value.height().as_u64(),
            epoch: value.epoch().as_u64(),
            parent_id: value.parent().as_bytes().to_vec(),
            proposed_by: value.proposed_by().as_bytes().to_vec(),
            merkle_root: value.merkle_root().as_slice().to_vec(),
            justify: Some(value.justify().into()),
            round: value.round(),
            commands: value.into_commands().into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<proto::consensus::Block> for tari_dan_storage::consensus_models::Block {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Block) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.parent_id.try_into()?,
            value
                .justify
                .ok_or_else(|| anyhow!("Block conversion: QC not provided"))?
                .try_into()?,
            NodeHeight(value.height),
            Epoch(value.epoch),
            value.round,
            value.proposed_by.try_into()?,
            value
                .commands
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        ))
    }
}

//---------------------------------- Command --------------------------------------------//

impl From<Command> for proto::consensus::Command {
    fn from(value: Command) -> Self {
        let command = match value {
            Command::Prepare(tx) => proto::consensus::command::Command::Prepare(tx.into()),
            Command::LocalPrepared(tx) => proto::consensus::command::Command::LocalPrepared(tx.into()),
            Command::Accept(tx) => proto::consensus::command::Command::Accept(tx.into()),
        };

        Self { command: Some(command) }
    }
}

impl TryFrom<proto::consensus::Command> for Command {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Command) -> Result<Self, Self::Error> {
        let command = value.command.ok_or_else(|| anyhow!("Command is missing"))?;
        Ok(match command {
            proto::consensus::command::Command::Prepare(tx) => Command::Prepare(tx.try_into()?),
            proto::consensus::command::Command::LocalPrepared(tx) => Command::LocalPrepared(tx.try_into()?),
            proto::consensus::command::Command::Accept(tx) => Command::Accept(tx.try_into()?),
        })
    }
}

//---------------------------------- TranactionAtom --------------------------------------------//

impl From<TransactionAtom> for proto::consensus::TransactionAtom {
    fn from(value: TransactionAtom) -> Self {
        Self {
            id: value.id.as_bytes().to_vec(),
            involved_shards: value
                .involved_shards
                .into_iter()
                .map(|s| s.as_bytes().to_vec())
                .collect(),
            decision: i32::from(value.decision.as_u8()),
            evidence: Some(value.evidence.into()),
            fee: value.fee,
        }
    }
}

impl TryFrom<proto::consensus::TransactionAtom> for TransactionAtom {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::TransactionAtom) -> Result<Self, Self::Error> {
        Ok(TransactionAtom {
            id: TransactionId::try_from(value.id)?,
            involved_shards: value
                .involved_shards
                .into_iter()
                .map(|s| ShardId::try_from(s.as_slice()))
                .collect::<Result<_, _>>()?,
            decision: Decision::from_u8(u8::try_from(value.decision)?)
                .ok_or_else(|| anyhow!("Invalid Decision byte {}", value.decision))?,
            evidence: value
                .evidence
                .ok_or_else(|| anyhow!("evidence not provided"))?
                .try_into()?,
            fee: value.fee,
        })
    }
}

//---------------------------------- Evidence --------------------------------------------//

impl From<Evidence> for proto::consensus::Evidence {
    fn from(value: Evidence) -> Self {
        // TODO: we may want to write out the protobuf here
        Self {
            encoded_evidence: encode(&value).unwrap(),
        }
    }
}

impl TryFrom<proto::consensus::Evidence> for Evidence {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Evidence) -> Result<Self, Self::Error> {
        Ok(decode_exact(&value.encoded_evidence)?)
    }
}

// -------------------------------- QuorumCertificate -------------------------------- //

impl<T: Borrow<QuorumCertificate>> From<T> for proto::consensus::QuorumCertificate {
    fn from(value: T) -> Self {
        let source = value.borrow();
        // TODO: unwrap
        let merged_merkle_proof = encode(&source.merged_proof()).unwrap();
        Self {
            block_id: source.block_id().as_bytes().to_vec(),
            block_height: source.block_height().as_u64(),
            epoch: source.epoch().as_u64(),
            signatures: source.signatures().iter().cloned().map(Into::into).collect(),
            merged_proof: merged_merkle_proof,
            leaf_hashes: source.leaf_hashes().iter().map(|h| h.to_vec()).collect(),
            decision: i32::from(source.decision().as_u8()),
        }
    }
}

impl TryFrom<proto::consensus::QuorumCertificate> for QuorumCertificate {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::QuorumCertificate) -> Result<Self, Self::Error> {
        let merged_proof = decode_exact(&value.merged_proof)?;
        Ok(Self::new(
            value.block_id.try_into()?,
            NodeHeight(value.block_height),
            Epoch(value.epoch),
            value
                .signatures
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            merged_proof,
            value
                .leaf_hashes
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            QuorumDecision::from_u8(u8::try_from(value.decision)?)
                .ok_or_else(|| anyhow!("Invalid Decision byte {}", value.decision))?,
        ))
    }
}

// -------------------------------- ValidatorMetadata -------------------------------- //

impl From<ValidatorMetadata> for proto::consensus::ValidatorMetadata {
    fn from(msg: ValidatorMetadata) -> Self {
        Self {
            public_key: msg.public_key.to_vec(),
            vn_shard_key: msg.vn_shard_key.as_bytes().to_vec(),
            signature: Some(msg.signature.into()),
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
        })
    }
}
