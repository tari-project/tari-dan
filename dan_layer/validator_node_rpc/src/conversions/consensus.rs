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
use serde::Serialize;
use tari_bor::{decode_exact, encode};
use tari_consensus::messages::{
    FullBlock,
    HotstuffMessage,
    NewViewMessage,
    ProposalMessage,
    RequestMissingForeignBlocksMessage,
    RequestMissingTransactionsMessage,
    RequestedTransactionMessage,
    SyncRequestMessage,
    SyncResponseMessage,
    VoteMessage,
};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{shard_bucket::ShardBucket, Epoch, NodeAddressable, NodeHeight, ValidatorMetadata};
use tari_dan_storage::consensus_models::{
    BlockId,
    Command,
    Decision,
    Evidence,
    ForeignProposal,
    ForeignProposalState,
    HighQc,
    QcId,
    QuorumCertificate,
    QuorumDecision,
    SubstateDestroyed,
    SubstateRecord,
    TransactionAtom,
};
use tari_engine_types::substate::{SubstateAddress, SubstateValue};
use tari_transaction::TransactionId;

use crate::proto::{self};
// -------------------------------- HotstuffMessage -------------------------------- //

impl<TAddr: NodeAddressable> From<&HotstuffMessage<TAddr>> for proto::consensus::HotStuffMessage {
    fn from(source: &HotstuffMessage<TAddr>) -> Self {
        let message = match source {
            HotstuffMessage::NewView(msg) => proto::consensus::hot_stuff_message::Message::NewView(msg.into()),
            HotstuffMessage::Proposal(msg) => proto::consensus::hot_stuff_message::Message::Proposal(msg.into()),
            HotstuffMessage::ForeignProposal(msg) => {
                proto::consensus::hot_stuff_message::Message::ForeignProposal(msg.into())
            },
            HotstuffMessage::Vote(msg) => proto::consensus::hot_stuff_message::Message::Vote(msg.into()),
            HotstuffMessage::RequestMissingForeignBlocks(msg) => {
                proto::consensus::hot_stuff_message::Message::RequestMissingForeignBlocks(msg.into())
            },
            HotstuffMessage::RequestMissingTransactions(msg) => {
                proto::consensus::hot_stuff_message::Message::RequestMissingTransactions(msg.into())
            },
            HotstuffMessage::RequestedTransaction(msg) => {
                proto::consensus::hot_stuff_message::Message::RequestedTransaction(msg.into())
            },
            HotstuffMessage::SyncRequest(msg) => proto::consensus::hot_stuff_message::Message::SyncRequest(msg.into()),
            HotstuffMessage::SyncResponse(msg) => {
                proto::consensus::hot_stuff_message::Message::SyncResponse(msg.into())
            },
        };
        Self { message: Some(message) }
    }
}

impl<TAddr: NodeAddressable + Serialize> TryFrom<proto::consensus::HotStuffMessage> for HotstuffMessage<TAddr> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::HotStuffMessage) -> Result<Self, Self::Error> {
        let message = value.message.ok_or_else(|| anyhow!("Message is missing"))?;
        Ok(match message {
            proto::consensus::hot_stuff_message::Message::NewView(msg) => HotstuffMessage::NewView(msg.try_into()?),
            proto::consensus::hot_stuff_message::Message::Proposal(msg) => HotstuffMessage::Proposal(msg.try_into()?),
            proto::consensus::hot_stuff_message::Message::ForeignProposal(msg) => {
                HotstuffMessage::ForeignProposal(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::Vote(msg) => HotstuffMessage::Vote(msg.try_into()?),
            proto::consensus::hot_stuff_message::Message::RequestMissingTransactions(msg) => {
                HotstuffMessage::RequestMissingTransactions(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::RequestMissingForeignBlocks(msg) => {
                HotstuffMessage::RequestMissingForeignBlocks(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::RequestedTransaction(msg) => {
                HotstuffMessage::RequestedTransaction(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::SyncRequest(msg) => {
                HotstuffMessage::SyncRequest(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::SyncResponse(msg) => {
                HotstuffMessage::SyncResponse(msg.try_into()?)
            },
        })
    }
}

//---------------------------------- NewView --------------------------------------------//

impl<TAddr: NodeAddressable> From<&NewViewMessage<TAddr>> for proto::consensus::NewViewMessage {
    fn from(value: &NewViewMessage<TAddr>) -> Self {
        Self {
            high_qc: Some((&value.high_qc).into()),
            new_height: value.new_height.0,
            epoch: value.epoch.as_u64(),
            last_vote: value.last_vote.as_ref().map(|a| a.into()),
        }
    }
}

impl<TAddr: NodeAddressable> TryFrom<proto::consensus::NewViewMessage> for NewViewMessage<TAddr> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::NewViewMessage) -> Result<Self, Self::Error> {
        Ok(NewViewMessage {
            high_qc: value.high_qc.ok_or_else(|| anyhow!("High QC is missing"))?.try_into()?,
            new_height: value.new_height.into(),
            epoch: Epoch(value.epoch),
            last_vote: value
                .last_vote
                .map(|a: proto::consensus::VoteMessage| a.try_into())
                .transpose()?,
        })
    }
}

//---------------------------------- ProposalMessage --------------------------------------------//

impl<TAddr: NodeAddressable> From<&ProposalMessage<TAddr>> for proto::consensus::ProposalMessage {
    fn from(value: &ProposalMessage<TAddr>) -> Self {
        Self {
            block: Some((&value.block).into()),
        }
    }
}

impl<TAddr: NodeAddressable + Serialize> TryFrom<proto::consensus::ProposalMessage> for ProposalMessage<TAddr> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ProposalMessage) -> Result<Self, Self::Error> {
        Ok(ProposalMessage {
            block: value.block.ok_or_else(|| anyhow!("Block is missing"))?.try_into()?,
        })
    }
}

// -------------------------------- VoteMessage -------------------------------- //

impl<TAddr: NodeAddressable> From<&VoteMessage<TAddr>> for proto::consensus::VoteMessage {
    fn from(msg: &VoteMessage<TAddr>) -> Self {
        Self {
            epoch: msg.epoch.as_u64(),
            block_id: msg.block_id.as_bytes().to_vec(),
            block_height: msg.block_height.as_u64(),
            decision: i32::from(msg.decision.as_u8()),
            signature: Some((&msg.signature).into()),
        }
    }
}

impl<TAddr: NodeAddressable> TryFrom<proto::consensus::VoteMessage> for VoteMessage<TAddr> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::VoteMessage) -> Result<Self, Self::Error> {
        Ok(VoteMessage {
            epoch: Epoch(value.epoch),
            block_id: BlockId::try_from(value.block_id)?,
            block_height: NodeHeight(value.block_height),
            decision: QuorumDecision::from_u8(u8::try_from(value.decision)?)
                .ok_or_else(|| anyhow!("Invalid decision byte {}", value.decision))?,
            signature: value
                .signature
                .ok_or_else(|| anyhow!("Signature is missing"))?
                .try_into()?,
        })
    }
}

//---------------------------------- RequestMissingForeignBlocksMessage --------------------------------------------//
impl From<&RequestMissingForeignBlocksMessage> for proto::consensus::RequestMissingForeignBlocksMessage {
    fn from(msg: &RequestMissingForeignBlocksMessage) -> Self {
        Self {
            epoch: msg.epoch.as_u64(),
            from: msg.from,
            to: msg.to,
        }
    }
}

impl TryFrom<proto::consensus::RequestMissingForeignBlocksMessage> for RequestMissingForeignBlocksMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::RequestMissingForeignBlocksMessage) -> Result<Self, Self::Error> {
        Ok(RequestMissingForeignBlocksMessage {
            epoch: Epoch(value.epoch),
            from: value.from,
            to: value.to,
        })
    }
}

//---------------------------------- RequestMissingTransactionsMessage --------------------------------------------//
impl From<&RequestMissingTransactionsMessage> for proto::consensus::RequestMissingTransactionsMessage {
    fn from(msg: &RequestMissingTransactionsMessage) -> Self {
        Self {
            epoch: msg.epoch.as_u64(),
            block_id: msg.block_id.as_bytes().to_vec(),
            transaction_ids: msg.transactions.iter().map(|tx_id| tx_id.as_bytes().to_vec()).collect(),
        }
    }
}

impl TryFrom<proto::consensus::RequestMissingTransactionsMessage> for RequestMissingTransactionsMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::RequestMissingTransactionsMessage) -> Result<Self, Self::Error> {
        Ok(RequestMissingTransactionsMessage {
            epoch: Epoch(value.epoch),
            block_id: BlockId::try_from(value.block_id)?,
            transactions: value
                .transaction_ids
                .into_iter()
                .map(|tx_id| tx_id.try_into())
                .collect::<Result<_, _>>()?,
        })
    }
}
//---------------------------------- RequestedTransactionMessage --------------------------------------------//

impl From<&RequestedTransactionMessage> for proto::consensus::RequestedTransactionMessage {
    fn from(msg: &RequestedTransactionMessage) -> Self {
        Self {
            epoch: msg.epoch.as_u64(),
            block_id: msg.block_id.as_bytes().to_vec(),
            transactions: msg.transactions.iter().map(|tx| tx.into()).collect(),
        }
    }
}

impl TryFrom<proto::consensus::RequestedTransactionMessage> for RequestedTransactionMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::RequestedTransactionMessage) -> Result<Self, Self::Error> {
        Ok(RequestedTransactionMessage {
            epoch: Epoch(value.epoch),
            block_id: BlockId::try_from(value.block_id)?,
            transactions: value
                .transactions
                .into_iter()
                .map(|tx| tx.try_into())
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}
//---------------------------------- Block --------------------------------------------//

impl<TAddr: NodeAddressable> From<&tari_dan_storage::consensus_models::Block<TAddr>> for proto::consensus::Block {
    fn from(value: &tari_dan_storage::consensus_models::Block<TAddr>) -> Self {
        Self {
            height: value.height().as_u64(),
            epoch: value.epoch().as_u64(),
            parent_id: value.parent().as_bytes().to_vec(),
            proposed_by: value.proposed_by().as_bytes().to_vec(),
            merkle_root: value.merkle_root().as_slice().to_vec(),
            justify: Some(value.justify().into()),
            total_leader_fee: value.total_leader_fee(),
            commands: value.commands().iter().map(Into::into).collect(),
            foreign_indexes: encode(value.get_foreign_indexes()).unwrap(),
        }
    }
}

impl<TAddr: NodeAddressable + Serialize> TryFrom<proto::consensus::Block>
    for tari_dan_storage::consensus_models::Block<TAddr>
{
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
            TAddr::from_bytes(&value.proposed_by).ok_or_else(|| anyhow!("Block conversion: Invalid proposed_by"))?,
            value
                .commands
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            value.total_leader_fee,
            decode_exact(&value.foreign_indexes)?,
        ))
    }
}

//---------------------------------- Command --------------------------------------------//

impl From<&Command> for proto::consensus::Command {
    fn from(value: &Command) -> Self {
        let command = match value {
            Command::Prepare(tx) => proto::consensus::command::Command::Prepare(tx.into()),
            Command::LocalPrepared(tx) => proto::consensus::command::Command::LocalPrepared(tx.into()),
            Command::Accept(tx) => proto::consensus::command::Command::Accept(tx.into()),
            Command::ForeignProposal(foreign_proposal) => {
                proto::consensus::command::Command::ForeignProposal(foreign_proposal.into())
            },
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
            proto::consensus::command::Command::ForeignProposal(foreign_proposal) => {
                Command::ForeignProposal(foreign_proposal.try_into()?)
            },
        })
    }
}

//---------------------------------- TranactionAtom --------------------------------------------//

impl From<&TransactionAtom> for proto::consensus::TransactionAtom {
    fn from(value: &TransactionAtom) -> Self {
        Self {
            id: value.id.as_bytes().to_vec(),
            decision: proto::consensus::Decision::from(value.decision) as i32,
            evidence: Some((&value.evidence).into()),
            fee: value.transaction_fee,
            leader_fee: value.leader_fee,
        }
    }
}

impl TryFrom<proto::consensus::TransactionAtom> for TransactionAtom {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::TransactionAtom) -> Result<Self, Self::Error> {
        Ok(TransactionAtom {
            id: TransactionId::try_from(value.id)?,
            decision: proto::consensus::Decision::from_i32(value.decision)
                .ok_or_else(|| anyhow!("Invalid decision value {}", value.decision))?
                .try_into()?,
            evidence: value
                .evidence
                .ok_or_else(|| anyhow!("evidence not provided"))?
                .try_into()?,
            transaction_fee: value.fee,
            leader_fee: value.leader_fee,
        })
    }
}

// ForeignProposalState
// -------------------------------- Decision -------------------------------- //

impl From<ForeignProposalState> for proto::consensus::ForeignProposalState {
    fn from(value: ForeignProposalState) -> Self {
        match value {
            ForeignProposalState::New => proto::consensus::ForeignProposalState::New,
            ForeignProposalState::Mined => proto::consensus::ForeignProposalState::Mined,
            ForeignProposalState::Deleted => proto::consensus::ForeignProposalState::Deleted,
        }
    }
}

impl TryFrom<proto::consensus::ForeignProposalState> for ForeignProposalState {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ForeignProposalState) -> Result<Self, Self::Error> {
        match value {
            proto::consensus::ForeignProposalState::New => Ok(ForeignProposalState::New),
            proto::consensus::ForeignProposalState::Mined => Ok(ForeignProposalState::Mined),
            proto::consensus::ForeignProposalState::Deleted => Ok(ForeignProposalState::Deleted),
            proto::consensus::ForeignProposalState::UnknownState => Err(anyhow!("Foreign proposal state not provided")),
        }
    }
}

// ForeignProposal

impl From<&ForeignProposal> for proto::consensus::ForeignProposal {
    fn from(value: &ForeignProposal) -> Self {
        Self {
            bucket: value.bucket.as_u32(),
            block_id: value.block_id.as_bytes().to_vec(),
            state: proto::consensus::ForeignProposalState::from(value.state).into(),
            mined_at: value.mined_at.map(|a| a.0).unwrap_or(0),
        }
    }
}

impl TryFrom<proto::consensus::ForeignProposal> for ForeignProposal {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ForeignProposal) -> Result<Self, Self::Error> {
        Ok(ForeignProposal {
            bucket: ShardBucket::from(value.bucket),
            block_id: BlockId::try_from(value.block_id)?,
            state: proto::consensus::ForeignProposalState::from_i32(value.state)
                .ok_or_else(|| anyhow!("Invalid foreign proposal state value {}", value.state))?
                .try_into()?,
            mined_at: if value.mined_at == 0 {
                None
            } else {
                Some(NodeHeight(value.mined_at))
            },
        })
    }
}

// -------------------------------- Decision -------------------------------- //

impl From<Decision> for proto::consensus::Decision {
    fn from(value: Decision) -> Self {
        match value {
            Decision::Commit => proto::consensus::Decision::Commit,
            Decision::Abort => proto::consensus::Decision::Abort,
        }
    }
}

impl TryFrom<proto::consensus::Decision> for Decision {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Decision) -> Result<Self, Self::Error> {
        match value {
            proto::consensus::Decision::Commit => Ok(Decision::Commit),
            proto::consensus::Decision::Abort => Ok(Decision::Abort),
            proto::consensus::Decision::Unknown => Err(anyhow!("Decision not provided")),
        }
    }
}

//---------------------------------- Evidence --------------------------------------------//

impl From<&Evidence> for proto::consensus::Evidence {
    fn from(value: &Evidence) -> Self {
        // TODO: we may want to write out the protobuf here
        Self {
            encoded_evidence: encode(value).unwrap(),
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

impl<TAddr: NodeAddressable> From<&QuorumCertificate<TAddr>> for proto::consensus::QuorumCertificate {
    fn from(source: &QuorumCertificate<TAddr>) -> Self {
        // TODO: unwrap
        let merged_merkle_proof = encode(&source.merged_proof()).unwrap();
        Self {
            block_id: source.block_id().as_bytes().to_vec(),
            block_height: source.block_height().as_u64(),
            epoch: source.epoch().as_u64(),
            signatures: source.signatures().iter().map(Into::into).collect(),
            merged_proof: merged_merkle_proof,
            leaf_hashes: source.leaf_hashes().iter().map(|h| h.to_vec()).collect(),
            decision: i32::from(source.decision().as_u8()),
        }
    }
}

impl<TAddr: NodeAddressable + Serialize> TryFrom<proto::consensus::QuorumCertificate> for QuorumCertificate<TAddr> {
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
            public_key: ByteArray::from_canonical_bytes(&value.public_key).map_err(anyhow::Error::msg)?,
            vn_shard_key: value.vn_shard_key.try_into()?,
            signature: value
                .signature
                .map(TryFrom::try_from)
                .transpose()?
                .ok_or_else(|| anyhow!("ValidatorMetadata missing signature"))?,
        })
    }
}

// -------------------------------- Substate -------------------------------- //

impl TryFrom<proto::consensus::Substate> for SubstateRecord {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Substate) -> Result<Self, Self::Error> {
        Ok(Self {
            address: SubstateAddress::from_bytes(&value.address)?,
            version: value.version,
            substate_value: SubstateValue::from_bytes(&value.substate)?,
            state_hash: Default::default(),

            created_at_epoch: Epoch(value.created_epoch),
            created_by_transaction: value.created_transaction.try_into()?,
            created_justify: value.created_justify.try_into()?,
            created_block: value.created_block.try_into()?,
            created_height: NodeHeight(value.created_height),

            destroyed: value.destroyed.map(TryInto::try_into).transpose()?,
        })
    }
}

impl From<SubstateRecord> for proto::consensus::Substate {
    fn from(value: SubstateRecord) -> Self {
        Self {
            address: value.address.to_bytes(),
            version: value.version,
            substate: value.substate_value.to_bytes(),

            created_transaction: value.created_by_transaction.as_bytes().to_vec(),
            created_justify: value.created_justify.as_bytes().to_vec(),
            created_block: value.created_block.as_bytes().to_vec(),
            created_height: value.created_height.as_u64(),
            created_epoch: value.created_at_epoch.as_u64(),

            destroyed: value.destroyed.map(Into::into),
        }
    }
}

// -------------------------------- SubstateDestroyed -------------------------------- //
impl TryFrom<proto::consensus::SubstateDestroyed> for SubstateDestroyed {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::SubstateDestroyed) -> Result<Self, Self::Error> {
        Ok(Self {
            by_transaction: value.transaction.try_into()?,
            justify: value.justify.try_into()?,
            by_block: value.block.try_into()?,
            at_epoch: value
                .epoch
                .map(Into::into)
                .ok_or_else(|| anyhow!("Epoch not provided"))?,
        })
    }
}

impl From<SubstateDestroyed> for proto::consensus::SubstateDestroyed {
    fn from(value: SubstateDestroyed) -> Self {
        Self {
            transaction: value.by_transaction.as_bytes().to_vec(),
            justify: value.justify.as_bytes().to_vec(),
            block: value.by_block.as_bytes().to_vec(),
            epoch: Some(value.at_epoch.into()),
        }
    }
}

// -------------------------------- SyncRequest -------------------------------- //

impl From<&SyncRequestMessage> for proto::consensus::SyncRequest {
    fn from(value: &SyncRequestMessage) -> Self {
        Self {
            epoch: value.epoch.as_u64(),
            high_qc: Some(proto::consensus::HighQc {
                block_id: value.high_qc.block_id.as_bytes().to_vec(),
                block_height: value.high_qc.block_height.as_u64(),
                qc_id: value.high_qc.qc_id.as_bytes().to_vec(),
            }),
        }
    }
}

impl TryFrom<proto::consensus::SyncRequest> for SyncRequestMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::SyncRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: Epoch(value.epoch),
            high_qc: value
                .high_qc
                .map(|value| {
                    Ok::<_, anyhow::Error>(HighQc {
                        block_id: BlockId::try_from(value.block_id)?,
                        block_height: NodeHeight(value.block_height),
                        qc_id: QcId::try_from(value.qc_id)?,
                    })
                })
                .transpose()?
                .ok_or_else(|| anyhow!("High QC not provided"))?,
        })
    }
}

// -------------------------------- SyncResponse -------------------------------- //

impl<TAddr: NodeAddressable> From<&SyncResponseMessage<TAddr>> for proto::consensus::SyncResponse {
    fn from(value: &SyncResponseMessage<TAddr>) -> Self {
        Self {
            epoch: value.epoch.as_u64(),
            blocks: value.blocks.iter().map(|block| block.into()).collect::<Vec<_>>(),
        }
    }
}

impl<TAddr: NodeAddressable + Serialize> TryFrom<proto::consensus::SyncResponse> for SyncResponseMessage<TAddr> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::SyncResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: Epoch(value.epoch),
            blocks: value
                .blocks
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

// -------------------------------- FullBlock -------------------------------- //

impl<TAddr: NodeAddressable> From<&FullBlock<TAddr>> for proto::consensus::FullBlock {
    fn from(value: &FullBlock<TAddr>) -> Self {
        Self {
            block: Some((&value.block).into()),
            qcs: value.qcs.iter().map(Into::into).collect(),
            transactions: value.transactions.iter().map(Into::into).collect(),
        }
    }
}

impl<TAddr: NodeAddressable + Serialize> TryFrom<proto::consensus::FullBlock> for FullBlock<TAddr> {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::FullBlock) -> Result<Self, Self::Error> {
        Ok(Self {
            block: value.block.ok_or_else(|| anyhow!("Block is missing"))?.try_into()?,
            qcs: value.qcs.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
            transactions: value
                .transactions
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}
