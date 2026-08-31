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
    collections::BTreeSet,
    convert::{TryFrom, TryInto},
};

use anyhow::{Context, anyhow};
use tari_consensus::messages::{
    CatchUpRequestMessage,
    ForeignProposalMessage,
    ForeignProposalNotificationMessage,
    ForeignProposalRequestMessage,
    HotstuffMessage,
    MissingTransactionsRequest,
    MissingTransactionsResponse,
    NewViewMessage,
    ProposalMessage,
    VoteMessage,
};
use tari_consensus_types::{
    BlockId,
    Decision,
    PcId,
    ProposalCertificate,
    ProposalVote,
    ShardGroupAccumulatedData,
    TimeoutCertificate,
    TimeoutVote,
};
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::{
    commit_result::AbortReason,
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    NodeHeight,
    ProtocolVersion,
    ShardGroup,
    ShardStateVersions,
    StateVersion,
    ValidatorMetadata,
    shard::Shard,
};
use tari_ootle_storage::{
    consensus_models,
    consensus_models::{
        Command,
        EndEpochAtom,
        EvictNodeAtom,
        Evidence,
        ForeignProposal,
        ForeignProposalAtom,
        LeaderFee,
        SubstateCreated,
        SubstateDestroyed,
        SubstateRecord,
        TransactionAtom,
    },
};
use tari_ootle_transaction::TransactionId;
use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

use crate::{
    encoding::{decode_from_slice, encode_to_vec},
    proto::{self},
};
// -------------------------------- HotstuffMessage -------------------------------- //

impl From<&HotstuffMessage> for proto::consensus::HotStuffMessage {
    fn from(source: &HotstuffMessage) -> Self {
        let message = match source {
            HotstuffMessage::NewView(msg) => proto::consensus::hot_stuff_message::Message::NewView((&**msg).into()),
            HotstuffMessage::Proposal(msg) => proto::consensus::hot_stuff_message::Message::Proposal((&**msg).into()),
            HotstuffMessage::ForeignProposal(msg) => {
                proto::consensus::hot_stuff_message::Message::ForeignProposal(msg.into())
            },
            HotstuffMessage::ForeignProposalNotification(msg) => {
                proto::consensus::hot_stuff_message::Message::ForeignProposalNotification(msg.into())
            },
            HotstuffMessage::ForeignProposalRequest(msg) => {
                proto::consensus::hot_stuff_message::Message::ForeignProposalRequest(msg.into())
            },
            HotstuffMessage::Vote(msg) => proto::consensus::hot_stuff_message::Message::Vote(msg.into()),
            HotstuffMessage::MissingTransactionsRequest(msg) => {
                proto::consensus::hot_stuff_message::Message::RequestMissingTransactions(msg.into())
            },
            HotstuffMessage::MissingTransactionsResponse(msg) => {
                proto::consensus::hot_stuff_message::Message::RequestedTransaction(msg.into())
            },
            HotstuffMessage::CatchUpSyncRequest(msg) => {
                proto::consensus::hot_stuff_message::Message::SyncRequest(msg.into())
            },
            HotstuffMessage::CatchUpSyncResponse(msg) => {
                proto::consensus::hot_stuff_message::Message::CatchUpSyncResponse((&**msg).into())
            },
        };
        Self { message: Some(message) }
    }
}

impl TryFrom<proto::consensus::HotStuffMessage> for HotstuffMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::HotStuffMessage) -> Result<Self, Self::Error> {
        let message = value.message.ok_or_else(|| anyhow!("Message is missing"))?;
        Ok(match message {
            proto::consensus::hot_stuff_message::Message::NewView(msg) => HotstuffMessage::new_newview(msg.try_into()?),
            proto::consensus::hot_stuff_message::Message::Proposal(msg) => {
                HotstuffMessage::new_proposal(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::ForeignProposal(msg) => {
                HotstuffMessage::ForeignProposal(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::ForeignProposalNotification(msg) => {
                HotstuffMessage::ForeignProposalNotification(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::ForeignProposalRequest(msg) => {
                HotstuffMessage::ForeignProposalRequest(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::Vote(msg) => HotstuffMessage::Vote(msg.try_into()?),
            proto::consensus::hot_stuff_message::Message::RequestMissingTransactions(msg) => {
                HotstuffMessage::MissingTransactionsRequest(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::RequestedTransaction(msg) => {
                HotstuffMessage::MissingTransactionsResponse(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::SyncRequest(msg) => {
                HotstuffMessage::CatchUpSyncRequest(msg.try_into()?)
            },
            proto::consensus::hot_stuff_message::Message::CatchUpSyncResponse(msg) => {
                HotstuffMessage::new_catch_up_sync_response(msg.try_into()?)
            },
        })
    }
}

//---------------------------------- NewView --------------------------------------------//

impl From<&NewViewMessage> for proto::consensus::NewViewMessage {
    fn from(value: &NewViewMessage) -> Self {
        Self {
            high_qc: Some((&value.high_pc).into()),
            last_vote: value.last_vote.as_ref().map(|a| a.into()),
            timeout: Some((&value.timeout).into()),
        }
    }
}

impl TryFrom<proto::consensus::NewViewMessage> for NewViewMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::NewViewMessage) -> Result<Self, Self::Error> {
        Ok(NewViewMessage {
            high_pc: value.high_qc.ok_or_else(|| anyhow!("High QC is missing"))?.try_into()?,
            last_vote: value
                .last_vote
                .map(|a: proto::consensus::VoteMessage| a.try_into())
                .transpose()?,
            timeout: value.timeout.ok_or_else(|| anyhow!("Timeout is missing"))?.try_into()?,
        })
    }
}

// -------------------------------- TimeoutVote -------------------------------- //

impl From<&TimeoutVote> for proto::consensus::TimeoutVote {
    fn from(value: &TimeoutVote) -> Self {
        Self {
            epoch: value.epoch.as_u64(),
            height: value.height.as_u64(),
            signature: Some((&value.signature).into()),
        }
    }
}

impl TryFrom<proto::consensus::TimeoutVote> for TimeoutVote {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::TimeoutVote) -> Result<Self, Self::Error> {
        Ok(TimeoutVote {
            epoch: Epoch(value.epoch),
            height: NodeHeight(value.height),
            signature: value
                .signature
                .ok_or_else(|| anyhow!("Signature is missing"))?
                .try_into()?,
        })
    }
}

//---------------------------------- ProposalMessage --------------------------------------------//

impl From<&ProposalMessage> for proto::consensus::ProposalMessage {
    fn from(value: &ProposalMessage) -> Self {
        Self {
            block: Some((&value.block).into()),
            foreign_proposals: value.foreign_proposals.iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<proto::consensus::ProposalMessage> for ProposalMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ProposalMessage) -> Result<Self, Self::Error> {
        Ok(ProposalMessage {
            block: value.block.ok_or_else(|| anyhow!("Block is missing"))?.try_into()?,
            foreign_proposals: value
                .foreign_proposals
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

// -------------------------------- ForeignProposalMessage -------------------------------- //

impl From<&ForeignProposalMessage> for proto::consensus::ForeignProposalMessage {
    fn from(value: &ForeignProposalMessage) -> Self {
        Self {
            proposal: Some(proto::consensus::ForeignProposal::from(&*value.proposal)),
        }
    }
}

impl TryFrom<proto::consensus::ForeignProposalMessage> for ForeignProposalMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ForeignProposalMessage) -> Result<Self, Self::Error> {
        let proposal = value.proposal.ok_or_else(|| anyhow!("Proposal is missing"))?;
        Ok(ForeignProposalMessage {
            proposal: Box::new(proposal.try_into()?),
        })
    }
}

impl From<&ForeignProposal> for proto::consensus::ForeignProposal {
    fn from(value: &ForeignProposal) -> Self {
        Self {
            // TODO: remove panics
            encoded_commit_proof: encode_to_vec(value.commit_proof()).expect("Failed to encode commit proof"),
            encoded_block_pledge: encode_to_vec(value.block_pledge()).expect("Failed to encode block pledge"),
        }
    }
}

impl TryFrom<proto::consensus::ForeignProposal> for ForeignProposal {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ForeignProposal) -> Result<Self, Self::Error> {
        Ok(Self::new(
            decode_from_slice(&value.encoded_commit_proof).context("Failed to decode commit proof")?,
            decode_from_slice(&value.encoded_block_pledge).context("Failed to decode block pledge")?,
        ))
    }
}

// -------------------------------- ForeignProposalNotification -------------------------------- //

impl From<&ForeignProposalNotificationMessage> for proto::consensus::ForeignProposalNotification {
    fn from(value: &ForeignProposalNotificationMessage) -> Self {
        Self {
            block_id: value.block_id.as_bytes().to_vec(),
            epoch: value.epoch.as_u64(),
            shard_groups: value.shard_groups.iter().map(|sg| sg.encode_as_u32()).collect(),
        }
    }
}

impl TryFrom<proto::consensus::ForeignProposalNotification> for ForeignProposalNotificationMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ForeignProposalNotification) -> Result<Self, Self::Error> {
        Ok(Self {
            block_id: BlockId::try_from(value.block_id)?,
            epoch: Epoch(value.epoch),
            shard_groups: value
                .shard_groups
                .into_iter()
                .map(|sg| {
                    ShardGroup::decode_from_u32(sg)
                        .ok_or_else(|| anyhow!("Invalid shard group in foreign proposal notification: {sg}"))
                })
                .collect::<Result<_, _>>()?,
        })
    }
}

impl From<&ForeignProposalRequestMessage> for proto::consensus::ForeignProposalRequest {
    fn from(value: &ForeignProposalRequestMessage) -> Self {
        match value {
            ForeignProposalRequestMessage::ByBlockId {
                block_id,
                for_shard_group,
                epoch,
            } => Self {
                request: Some(proto::consensus::foreign_proposal_request::Request::ByBlockId(
                    proto::consensus::ForeignProposalRequestByBlockId {
                        block_id: block_id.as_bytes().to_vec(),
                        for_shard_group: for_shard_group.encode_as_u32(),
                        epoch: epoch.as_u64(),
                    },
                )),
            },
        }
    }
}

impl TryFrom<proto::consensus::ForeignProposalRequest> for ForeignProposalRequestMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ForeignProposalRequest) -> Result<Self, Self::Error> {
        let request = value.request.ok_or_else(|| anyhow!("Request is missing"))?;
        Ok(match request {
            proto::consensus::foreign_proposal_request::Request::ByBlockId(by_block_id) => {
                ForeignProposalRequestMessage::ByBlockId {
                    block_id: BlockId::try_from(by_block_id.block_id)?,
                    for_shard_group: ShardGroup::decode_from_u32(by_block_id.for_shard_group)
                        .ok_or_else(|| anyhow!("Invalid ShardGroup"))?,
                    epoch: Epoch(by_block_id.epoch),
                }
            },
        })
    }
}

// -------------------------------- VoteMessage -------------------------------- //

impl From<&VoteMessage> for proto::consensus::VoteMessage {
    fn from(msg: &VoteMessage) -> Self {
        let vote = &msg.vote;
        Self {
            epoch: vote.epoch.as_u64(),
            block_id: vote.block_id.as_bytes().to_vec(),
            block_height: vote.block_height.as_u64(),
            decision: i32::from(vote.decision.as_u8()),
            signature: Some((&vote.signature).into()),
        }
    }
}

impl TryFrom<proto::consensus::VoteMessage> for VoteMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::VoteMessage) -> Result<Self, Self::Error> {
        Ok(VoteMessage {
            vote: value.try_into()?,
        })
    }
}

// -------------------------------- ProposalVote -------------------------------- //

impl From<&ProposalVote> for proto::consensus::VoteMessage {
    fn from(vote: &ProposalVote) -> Self {
        Self {
            epoch: vote.epoch.as_u64(),
            block_id: vote.block_id.as_bytes().to_vec(),
            block_height: vote.block_height.as_u64(),
            decision: i32::from(vote.decision.as_u8()),
            signature: Some((&vote.signature).into()),
        }
    }
}

impl TryFrom<proto::consensus::VoteMessage> for ProposalVote {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::VoteMessage) -> Result<Self, Self::Error> {
        Ok(ProposalVote {
            epoch: Epoch(value.epoch),
            block_id: BlockId::try_from(value.block_id)?,
            block_height: NodeHeight(value.block_height),
            decision: u8::try_from(value.decision)?.try_into()?,
            signature: value
                .signature
                .ok_or_else(|| anyhow!("Signature is missing"))?
                .try_into()?,
        })
    }
}

//---------------------------------- MissingTransactionsRequest --------------------------------------------//
impl From<&MissingTransactionsRequest> for proto::consensus::MissingTransactionsRequest {
    fn from(msg: &MissingTransactionsRequest) -> Self {
        Self {
            request_id: msg.request_id,
            epoch: msg.epoch.as_u64(),
            block_id: msg.block_id.as_bytes().to_vec(),
            transaction_ids: msg.transactions.iter().map(|tx_id| tx_id.as_bytes().to_vec()).collect(),
        }
    }
}

impl TryFrom<proto::consensus::MissingTransactionsRequest> for MissingTransactionsRequest {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::MissingTransactionsRequest) -> Result<Self, Self::Error> {
        Ok(MissingTransactionsRequest {
            request_id: value.request_id,
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
//---------------------------------- MissingTransactionsResponse --------------------------------------------//

impl From<&MissingTransactionsResponse> for proto::consensus::MissingTransactionsResponse {
    fn from(msg: &MissingTransactionsResponse) -> Self {
        Self {
            request_id: msg.request_id,
            epoch: msg.epoch.as_u64(),
            block_id: msg.block_id.as_bytes().to_vec(),
            transactions: msg.transactions.iter().map(|tx| tx.into()).collect(),
        }
    }
}

impl TryFrom<proto::consensus::MissingTransactionsResponse> for MissingTransactionsResponse {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::MissingTransactionsResponse) -> Result<Self, Self::Error> {
        Ok(MissingTransactionsResponse {
            request_id: value.request_id,
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

impl From<&consensus_models::BlockHeader> for proto::consensus::BlockHeader {
    fn from(value: &consensus_models::BlockHeader) -> Self {
        Self {
            network: value.network().as_byte().into(),
            height: value.height().as_u64(),
            epoch: value.epoch().as_u64(),
            shard_group: value.shard_group().encode_as_u32(),
            parent_id: value.parent().as_bytes().to_vec(),
            proposed_by: value.proposed_by().as_bytes().to_vec(),
            state_merkle_root: value.state_merkle_root().as_slice().to_vec(),
            total_leader_fee: value.total_leader_fee(),
            signature: value.signature().map(Into::into),
            timestamp: value.timestamp(),
            epoch_hash: value.epoch_hash().as_bytes().to_vec(),
            extra_data: Some(value.extra_data().into()),
            accumulated_data: Some(value.accumulated_data().into()),
            protocol_version: value.protocol_version().as_u32(),
        }
    }
}

fn try_convert_proto_block_header(
    value: proto::consensus::BlockHeader,
    justify_id: PcId,
    commands: &BTreeSet<Command>,
) -> Result<consensus_models::BlockHeader, anyhow::Error> {
    let network = u8::try_from(value.network)
        .map_err(|_| anyhow!("Block conversion: Invalid network byte {}", value.network))?
        .try_into()?;

    let protocol_version = ProtocolVersion::try_from(value.protocol_version)?;

    let shard_group = ShardGroup::decode_from_u32(value.shard_group)
        .ok_or_else(|| anyhow!("Block shard_group ({}) is not a valid", value.shard_group))?;

    let proposed_by = RistrettoPublicKeyBytes::from_bytes(&value.proposed_by)
        .map_err(|_| anyhow!("Block conversion: Invalid proposed_by"))?;

    let extra_data = value
        .extra_data
        .ok_or_else(|| anyhow!("ExtraData not provided"))?
        .try_into()?;

    // TODO: foreign nodes should never be able to send a block without a signature - currently used to force a view
    // change in catch up sync
    // Dummy has no signature
    if value.signature.is_none() {
        Ok(consensus_models::BlockHeader::dummy_block(
            network,
            protocol_version,
            value.parent_id.try_into()?,
            proposed_by,
            NodeHeight(value.height),
            justify_id,
            Epoch(value.epoch),
            shard_group,
            value.state_merkle_root.try_into()?,
            value.timestamp,
            value.epoch_hash.try_into()?,
            value
                .accumulated_data
                .ok_or_else(|| anyhow!("AccumulatedData not provided"))?
                .try_into()?,
        ))
    } else {
        // We calculate the BlockId and command MR locally from remote data. This means that they will
        // always be valid, therefore do not need to be explicitly validated.
        // If there were a mismatch (perhaps due modified data over the wire) the signature verification will fail.
        let block = consensus_models::BlockHeader::create(
            network,
            protocol_version,
            value.parent_id.try_into()?,
            justify_id,
            NodeHeight(value.height),
            Epoch(value.epoch),
            shard_group,
            proposed_by,
            value.state_merkle_root.try_into()?,
            commands,
            value.total_leader_fee,
            value
                .signature
                .map(TryInto::try_into)
                .transpose()?
                .ok_or_else(|| anyhow!("Block conversion: Block signature is missing"))?,
            value.timestamp,
            value.epoch_hash.try_into()?,
            value
                .accumulated_data
                .ok_or_else(|| anyhow!("AccumulatedData not provided"))?
                .try_into()?,
            extra_data,
        )?;

        Ok(block)
    }
}

//---------------------------------- Block --------------------------------------------//

impl From<&consensus_models::Block> for proto::consensus::Block {
    fn from(value: &consensus_models::Block) -> Self {
        Self {
            header: Some(value.header().into()),
            justify: Some(value.justify().into()),
            commands: value.commands().iter().map(Into::into).collect(),
            timeout_certificate: value.timeout_certificate().map(|a| a.into()),
        }
    }
}

impl TryFrom<proto::consensus::Block> for consensus_models::Block {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Block) -> Result<Self, Self::Error> {
        let commands = value
            .commands
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?;

        let justify = value
            .justify
            .ok_or_else(|| anyhow!("Block conversion: QC not provided"))?;
        let justify = ProposalCertificate::try_from(justify)?;

        let high_tc = value.timeout_certificate.map(TryInto::try_into).transpose()?;

        let header = value.header.ok_or_else(|| anyhow!("BlockHeader not provided"))?;
        let header = try_convert_proto_block_header(header, justify.calculate_id(), &commands)?;

        Ok(Self::new(header, justify, commands, high_tc))
    }
}

// -------------------------------- TimeoutCertificate -------------------------------- //

impl From<&TimeoutCertificate> for proto::consensus::TimeoutCertificate {
    fn from(value: &TimeoutCertificate) -> Self {
        Self {
            epoch: value.epoch().as_u64(),
            block_height: value.height().as_u64(),
            signatures: value.signatures().iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<proto::consensus::TimeoutCertificate> for TimeoutCertificate {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::TimeoutCertificate) -> Result<Self, Self::Error> {
        Ok(Self::new(
            Epoch(value.epoch),
            NodeHeight(value.block_height),
            value
                .signatures
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()
                .context("invalid encoding of signatures")?,
        ))
    }
}

//---------------------------------- Evidence --------------------------------------------//

impl From<&ExtraData> for proto::consensus::ExtraData {
    fn from(value: &ExtraData) -> Self {
        Self {
            // TODO: remove panics
            encoded_extra_data: encode_to_vec(value).unwrap(),
        }
    }
}

impl TryFrom<proto::consensus::ExtraData> for ExtraData {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ExtraData) -> Result<Self, Self::Error> {
        decode_from_slice(&value.encoded_extra_data)
    }
}

//---------------------------------- Command --------------------------------------------//

impl From<&Command> for proto::consensus::Command {
    fn from(value: &Command) -> Self {
        let command = match value {
            Command::LocalOnly(tx) => proto::consensus::command::Command::LocalOnly(tx.into()),
            Command::LocalPrepare(tx) => proto::consensus::command::Command::LocalPrepare(tx.into()),
            Command::LocalAccept(tx) => proto::consensus::command::Command::LocalAccept(tx.into()),
            Command::AllAccept(tx) => proto::consensus::command::Command::AllAccept(tx.into()),
            Command::SomeAccept(tx) => proto::consensus::command::Command::SomeAccept(tx.into()),
            Command::ForeignProposal(foreign_proposal) => {
                proto::consensus::command::Command::ForeignProposal(foreign_proposal.into())
            },
            Command::EvictNode(atom) => proto::consensus::command::Command::EvictNode(atom.into()),
            Command::EndEpoch(atom) => proto::consensus::command::Command::EndEpoch(atom.into()),
        };

        Self { command: Some(command) }
    }
}

impl TryFrom<proto::consensus::Command> for Command {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Command) -> Result<Self, Self::Error> {
        let command = value.command.ok_or_else(|| anyhow!("Command is missing"))?;
        Ok(match command {
            proto::consensus::command::Command::LocalOnly(tx) => Command::LocalOnly(tx.try_into()?),
            proto::consensus::command::Command::LocalPrepare(tx) => Command::LocalPrepare(tx.try_into()?),
            proto::consensus::command::Command::LocalAccept(tx) => Command::LocalAccept(tx.try_into()?),
            proto::consensus::command::Command::AllAccept(tx) => Command::AllAccept(tx.try_into()?),
            proto::consensus::command::Command::SomeAccept(tx) => Command::SomeAccept(tx.try_into()?),
            proto::consensus::command::Command::ForeignProposal(foreign_proposal) => {
                Command::ForeignProposal(foreign_proposal.try_into()?)
            },
            proto::consensus::command::Command::EvictNode(atom) => Command::EvictNode(atom.try_into()?),
            proto::consensus::command::Command::EndEpoch(atom) => Command::EndEpoch(atom.try_into()?),
        })
    }
}

//---------------------------------- TransactionAtom --------------------------------------------//

impl From<&TransactionAtom> for proto::consensus::TransactionAtom {
    fn from(value: &TransactionAtom) -> Self {
        Self {
            id: value.id.as_bytes().to_vec(),
            decision: Some(proto::consensus::Decision::from(value.decision)),
            evidence: Some((&value.evidence).into()),
            fee: value.transaction_fee,
            leader_fee: value.leader_fee.as_ref().map(|a| a.into()),
        }
    }
}

impl TryFrom<proto::consensus::TransactionAtom> for TransactionAtom {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::TransactionAtom) -> Result<Self, Self::Error> {
        let proto_decision = value.decision.ok_or(anyhow!("Decision is missing!"))?;
        Ok(TransactionAtom {
            id: TransactionId::try_from(value.id)?,
            decision: Decision::try_from(proto_decision)?,
            evidence: value
                .evidence
                .ok_or_else(|| anyhow!("evidence not provided"))?
                .try_into()?,
            transaction_fee: value.fee,
            leader_fee: value.leader_fee.map(TryInto::try_into).transpose()?,
        })
    }
}

// -------------------------------- BlockFee -------------------------------- //

impl From<&LeaderFee> for proto::consensus::LeaderFee {
    fn from(value: &LeaderFee) -> Self {
        Self {
            leader_fee: value.fee,
            exhaust_burn: value.exhaust_burn,
        }
    }
}

impl TryFrom<proto::consensus::LeaderFee> for LeaderFee {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::LeaderFee) -> Result<Self, Self::Error> {
        Ok(Self {
            fee: value.leader_fee,
            exhaust_burn: value.exhaust_burn,
        })
    }
}

// -------------------------------- ForeignProposalAtom -------------------------------- //

impl From<&ForeignProposalAtom> for proto::consensus::ForeignProposalAtom {
    fn from(value: &ForeignProposalAtom) -> Self {
        Self {
            block_id: value.block_id.as_bytes().to_vec(),
            shard_group: value.shard_group.encode_as_u32(),
        }
    }
}

impl TryFrom<proto::consensus::ForeignProposalAtom> for ForeignProposalAtom {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ForeignProposalAtom) -> Result<Self, Self::Error> {
        Ok(ForeignProposalAtom {
            block_id: BlockId::try_from(value.block_id)?,
            shard_group: ShardGroup::decode_from_u32(value.shard_group)
                .ok_or_else(|| anyhow!("Block shard_group ({}) is not a valid", value.shard_group))?,
        })
    }
}

// -------------------------------- EvictNodeAtom -------------------------------- //

impl From<&EvictNodeAtom> for proto::consensus::EvictNodeAtom {
    fn from(value: &EvictNodeAtom) -> Self {
        Self {
            public_key: value.public_key.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<proto::consensus::EvictNodeAtom> for EvictNodeAtom {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::EvictNodeAtom) -> Result<Self, Self::Error> {
        Ok(Self {
            public_key: value
                .public_key
                .as_slice()
                .try_into()
                .map_err(|e| anyhow!("EvictNodeAtom failed to decode public key: {e}"))?,
        })
    }
}

// -------------------------------- EndEpochAtom -------------------------------- //

impl From<&EndEpochAtom> for proto::consensus::EndEpochAtom {
    fn from(value: &EndEpochAtom) -> Self {
        Self {
            next_epoch_hash: value.next_epoch_hash.as_slice().to_vec(),
        }
    }
}

impl TryFrom<proto::consensus::EndEpochAtom> for EndEpochAtom {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::EndEpochAtom) -> Result<Self, Self::Error> {
        Ok(Self {
            next_epoch_hash: value
                .next_epoch_hash
                .as_slice()
                .try_into()
                .map_err(|e| anyhow!("EndEpochAtom failed to decode next_epoch_hash: {e}"))?,
        })
    }
}

// -------------------------------- Decision -------------------------------- //

impl From<Decision> for proto::consensus::Decision {
    fn from(value: Decision) -> Self {
        proto::consensus::Decision {
            decision: Some(value.into()),
        }
    }
}

impl From<Decision> for proto::consensus::decision::Decision {
    fn from(value: Decision) -> Self {
        match value {
            Decision::Commit => Self::Commit(true),
            Decision::Abort(reason) => Self::Abort(proto::consensus::AbortReason::from(reason) as i32),
        }
    }
}

// -------------------------------- Abort reason -------------------------------- //
impl From<AbortReason> for proto::consensus::AbortReason {
    fn from(value: AbortReason) -> Self {
        match value {
            AbortReason::LockOutputsFailed => Self::LockOutputsFailed,
            AbortReason::LockInputsOutputsFailed => Self::LockInputsOutputsFailed,
            AbortReason::LockInputsFailed => Self::LockInputsFailed,
            AbortReason::ExecutionFailure => Self::ExecutionFailure,
            AbortReason::OneOrMoreInputsNotFound => Self::OneOrMoreInputsNotFound,
            AbortReason::ForeignPledgeInputConflict => Self::ForeignPledgeInputConflict,
            AbortReason::InsufficientFeesPaid => Self::InsufficientFeesPaid,
            AbortReason::FeePaymentInMainIntent => Self::FeePaymentInMainIntent,
            AbortReason::EpochExpired => Self::EpochExpired,
            AbortReason::ValidityWindowTooLong => Self::ValidityWindowTooLong,
        }
    }
}

impl TryFrom<proto::consensus::AbortReason> for AbortReason {
    type Error = anyhow::Error;

    fn try_from(proto_reason: proto::consensus::AbortReason) -> Result<Self, Self::Error> {
        match proto_reason {
            proto::consensus::AbortReason::None => Err(anyhow!("AbortReason not provided/None")),
            proto::consensus::AbortReason::LockInputsFailed => Ok(Self::LockInputsFailed),
            proto::consensus::AbortReason::LockOutputsFailed => Ok(Self::LockOutputsFailed),
            proto::consensus::AbortReason::LockInputsOutputsFailed => Ok(Self::LockInputsOutputsFailed),
            proto::consensus::AbortReason::ExecutionFailure => Ok(Self::ExecutionFailure),
            proto::consensus::AbortReason::OneOrMoreInputsNotFound => Ok(Self::OneOrMoreInputsNotFound),
            proto::consensus::AbortReason::ForeignPledgeInputConflict => Ok(Self::ForeignPledgeInputConflict),
            proto::consensus::AbortReason::InsufficientFeesPaid => Ok(Self::InsufficientFeesPaid),
            proto::consensus::AbortReason::FeePaymentInMainIntent => Ok(Self::FeePaymentInMainIntent),
            proto::consensus::AbortReason::EpochExpired => Ok(Self::EpochExpired),
            proto::consensus::AbortReason::ValidityWindowTooLong => Ok(Self::ValidityWindowTooLong),
        }
    }
}

impl TryFrom<proto::consensus::Decision> for Decision {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Decision) -> Result<Self, Self::Error> {
        match value
            .decision
            .as_ref()
            .ok_or_else(|| anyhow!("Decision not provided"))?
        {
            proto::consensus::decision::Decision::Commit(_) => Ok(Decision::Commit),
            proto::consensus::decision::Decision::Abort(reason) => {
                let reason = proto::consensus::AbortReason::try_from(*reason)?;
                Ok(Decision::Abort(reason.try_into()?))
            },
        }
    }
}

//---------------------------------- Evidence --------------------------------------------//

impl From<&Evidence> for proto::consensus::Evidence {
    fn from(value: &Evidence) -> Self {
        Self {
            // TODO: remove panics
            encoded_evidence: encode_to_vec(value).unwrap(),
        }
    }
}

impl TryFrom<proto::consensus::Evidence> for Evidence {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::Evidence) -> Result<Self, Self::Error> {
        decode_from_slice(&value.encoded_evidence)
    }
}

// -------------------------------- ProposalCertificate -------------------------------- //

impl From<&ProposalCertificate> for proto::consensus::QuorumCertificate {
    fn from(source: &ProposalCertificate) -> Self {
        Self {
            header_hash: source.header_hash().as_bytes().to_vec(),
            parent_id: source.parent_id().as_bytes().to_vec(),
            block_height: source.height().as_u64(),
            epoch: source.epoch().as_u64(),
            shard_group: source.shard_group().encode_as_u32(),
            signatures: source.signatures().iter().map(Into::into).collect(),
            decision: i32::from(source.decision().as_u8()),
        }
    }
}

impl TryFrom<proto::consensus::QuorumCertificate> for ProposalCertificate {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::QuorumCertificate) -> Result<Self, Self::Error> {
        let shard_group = ShardGroup::decode_from_u32(value.shard_group)
            .ok_or_else(|| anyhow!("QC shard_group ({}) is not a valid", value.shard_group))?;
        Ok(Self::new(
            value.header_hash.try_into().context("header_hash")?,
            value.parent_id.try_into().context("parent_id")?,
            NodeHeight(value.block_height),
            Epoch(value.epoch),
            shard_group,
            value
                .signatures
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
            u8::try_from(value.decision)?.try_into()?,
        ))
    }
}

// -------------------------------- ValidatorMetadata -------------------------------- //

impl From<ValidatorMetadata> for proto::consensus::ValidatorMetadata {
    fn from(msg: ValidatorMetadata) -> Self {
        Self {
            public_key: msg.public_key.to_vec(),
            vn_shard_key: msg.vn_shard_key.as_bytes().to_vec(),
            signature: Some((&msg.signature).into()),
        }
    }
}

impl TryFrom<proto::consensus::ValidatorMetadata> for ValidatorMetadata {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ValidatorMetadata) -> Result<Self, Self::Error> {
        Ok(ValidatorMetadata {
            public_key: value
                .public_key
                .as_slice()
                .try_into()
                .context("Invalid public key TryFrom<ValidatorMetadata>")?,
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
            substate_id: SubstateId::from_bytes(&value.substate_id)?,
            version: value.version,
            substate_value: Some(value.substate.as_slice())
                .filter(|d| !d.is_empty())
                .map(SubstateValue::from_bytes)
                .transpose()?,
            // TODO: Should we add this to the proto?
            state_hash: Default::default(),

            created: value
                .created
                .ok_or_else(|| anyhow!("Substate created metadata not provided"))?
                .try_into()?,
            destroyed: value.destroyed.map(TryInto::try_into).transpose()?,
        })
    }
}

impl From<SubstateRecord> for proto::consensus::Substate {
    fn from(value: SubstateRecord) -> Self {
        Self {
            substate_id: value.substate_id.to_bytes(),
            version: value.version,
            substate: value.substate_value.as_ref().map(|s| s.to_bytes()).unwrap_or_default(),

            created: Some(value.created().into()),
            destroyed: value.destroyed().map(Into::into),
        }
    }
}

// -------------------------------- SubstateCreatedMetadata -------------------------------- //
impl TryFrom<proto::consensus::SubstateCreatedMetadata> for SubstateCreated {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::SubstateCreatedMetadata) -> Result<Self, Self::Error> {
        Ok(Self {
            at_epoch: value
                .at_epoch
                .map(Into::into)
                .ok_or_else(|| anyhow!("Epoch not provided"))?,
            in_shard: Shard::from(value.in_shard),
            at_state_version: value.at_state_version,
        })
    }
}

impl From<&SubstateCreated> for proto::consensus::SubstateCreatedMetadata {
    fn from(value: &SubstateCreated) -> Self {
        Self {
            at_epoch: Some(value.at_epoch.into()),
            in_shard: value.in_shard.as_u32(),
            at_state_version: value.at_state_version,
        }
    }
}

// -------------------------------- SubstateDestroyedMetadata -------------------------------- //
impl TryFrom<proto::consensus::SubstateDestroyedMetadata> for SubstateDestroyed {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::SubstateDestroyedMetadata) -> Result<Self, Self::Error> {
        Ok(Self {
            at_epoch: value
                .at_epoch
                .map(Into::into)
                .ok_or_else(|| anyhow!("Epoch not provided"))?,
            at_state_version: value.at_state_version,
        })
    }
}

impl From<&SubstateDestroyed> for proto::consensus::SubstateDestroyedMetadata {
    fn from(value: &SubstateDestroyed) -> Self {
        Self {
            at_epoch: Some(value.at_epoch.into()),
            at_state_version: value.at_state_version,
        }
    }
}

// -------------------------------- SyncRequest -------------------------------- //

impl From<&CatchUpRequestMessage> for proto::consensus::SyncRequest {
    fn from(value: &CatchUpRequestMessage) -> Self {
        Self {
            epoch: value.epoch.as_u64(),
            block_height: value.block_height.as_u64(),
        }
    }
}

impl TryFrom<proto::consensus::SyncRequest> for CatchUpRequestMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::SyncRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: Epoch(value.epoch),
            block_height: NodeHeight(value.block_height),
        })
    }
}

// -------------------------------- ShardStateVersions -------------------------------- //
impl From<&ShardStateVersions> for proto::consensus::ShardStateVersions {
    fn from(value: &ShardStateVersions) -> Self {
        Self {
            versions: value.as_slice().iter().map(|v| v.as_u64()).collect(),
        }
    }
}

impl TryFrom<proto::consensus::ShardStateVersions> for ShardStateVersions {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ShardStateVersions) -> Result<Self, Self::Error> {
        if value.versions.is_empty() {
            return Err(anyhow!("ShardStateVersions cannot be empty"));
        }
        if value.versions.len() > ShardStateVersions::MAX_LEN {
            return Err(anyhow!(
                "ShardStateVersions cannot have more than {} versions, got {}",
                ShardStateVersions::MAX_LEN,
                value.versions.len()
            ));
        }

        ShardStateVersions::from_vec(value.versions.into_iter().map(StateVersion::new).collect())
            .map_err(|e| anyhow!("Failed to convert ShardStateVersions: {}", e))
    }
}

// -------------------------------- AccumulatedData -------------------------------- //

impl From<&ShardGroupAccumulatedData> for proto::consensus::ShardGroupAccumulatedData {
    fn from(value: &ShardGroupAccumulatedData) -> Self {
        // Extract 2 u64s from the total_exhaust_burn u128
        let msb = value.total_exhaust_burn >> 64;
        let lsb = value.total_exhaust_burn & u128::from(u64::MAX);

        Self {
            total_exhaust_burn_msb: msb as u64,
            total_exhaust_burn_lsb: lsb as u64,
        }
    }
}

impl TryFrom<proto::consensus::ShardGroupAccumulatedData> for ShardGroupAccumulatedData {
    type Error = anyhow::Error;

    fn try_from(value: proto::consensus::ShardGroupAccumulatedData) -> Result<Self, anyhow::Error> {
        let total_exhaust_burn =
            (u128::from(value.total_exhaust_burn_msb) << 64) | u128::from(value.total_exhaust_burn_lsb);

        Ok(Self { total_exhaust_burn })
    }
}
