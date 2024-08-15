//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use serde::Serialize;
use tari_dan_common_types::Epoch;

use super::{ForeignProposalMessage, MissingTransactionsResponse, NewViewMessage, ProposalMessage, VoteMessage};
use crate::messages::{MissingTransactionsRequest, SyncRequestMessage, SyncResponseMessage};

// Serialize is implemented for the message logger
#[derive(Debug, Clone, Serialize)]
pub enum HotstuffMessage {
    NewView(NewViewMessage),
    Proposal(ProposalMessage),
    ForeignProposal(ForeignProposalMessage),
    Vote(VoteMessage),
    MissingTransactionsRequest(MissingTransactionsRequest),
    MissingTransactionsResponse(MissingTransactionsResponse),
    CatchUpSyncRequest(SyncRequestMessage),
    // TODO: remove unused
    SyncResponse(SyncResponseMessage),
}

impl HotstuffMessage {
    pub fn as_type_str(&self) -> &'static str {
        match self {
            HotstuffMessage::NewView(_) => "NewView",
            HotstuffMessage::Proposal(_) => "Proposal",
            HotstuffMessage::ForeignProposal(_) => "ForeignProposal",
            HotstuffMessage::Vote(_) => "Vote",
            HotstuffMessage::MissingTransactionsRequest(_) => "MissingTransactionsRequest",
            HotstuffMessage::MissingTransactionsResponse(_) => "MissingTransactionsResponse",
            HotstuffMessage::CatchUpSyncRequest(_) => "CatchUpSyncRequest",
            HotstuffMessage::SyncResponse(_) => "SyncResponse",
        }
    }

    pub fn epoch(&self) -> Epoch {
        match self {
            Self::NewView(msg) => msg.epoch,
            Self::Proposal(msg) => msg.block.epoch(),
            Self::ForeignProposal(msg) => msg.block.epoch(),
            Self::Vote(msg) => msg.epoch,
            Self::MissingTransactionsRequest(msg) => msg.epoch,
            Self::MissingTransactionsResponse(msg) => msg.epoch,
            Self::CatchUpSyncRequest(msg) => msg.epoch,
            Self::SyncResponse(msg) => msg.epoch,
        }
    }

    pub fn proposal(&self) -> Option<&ProposalMessage> {
        match self {
            Self::Proposal(msg) => Some(msg),
            _ => None,
        }
    }
}

impl Display for HotstuffMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HotstuffMessage::NewView(msg) => write!(f, "NewView({})", msg.new_height),
            HotstuffMessage::Proposal(msg) => {
                write!(f, "Proposal(Epoch={},Height={})", msg.block.epoch(), msg.block.height(),)
            },
            HotstuffMessage::ForeignProposal(msg) => write!(f, "ForeignProposal({})", msg),
            HotstuffMessage::Vote(msg) => write!(f, "Vote({}, {}, {})", msg.block_height, msg.block_id, msg.decision),
            HotstuffMessage::MissingTransactionsRequest(msg) => {
                write!(
                    f,
                    "RequestMissingTransactions({} transaction(s), block: {}, epoch: {})",
                    msg.transactions.len(),
                    msg.block_id,
                    msg.epoch
                )
            },
            HotstuffMessage::MissingTransactionsResponse(msg) => write!(
                f,
                "RequestedTransaction({} transaction(s), block: {}, epoch: {})",
                msg.transactions.len(),
                msg.block_id,
                msg.epoch
            ),
            HotstuffMessage::CatchUpSyncRequest(msg) => write!(f, "SyncRequest({})", msg.high_qc),
            HotstuffMessage::SyncResponse(msg) => write!(f, "SyncResponse({} block(s))", msg.blocks.len()),
        }
    }
}
