//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use serde::Serialize;
use tari_dan_common_types::Epoch;

use super::{NewViewMessage, ProposalMessage, RequestedTransactionMessage, VoteMessage};
use crate::messages::{RequestMissingTransactionsMessage, SyncRequestMessage, SyncResponseMessage};

// Serialize is implemented for the message logger
#[derive(Debug, Clone, Serialize)]
pub enum HotstuffMessage<TAddr> {
    NewView(NewViewMessage<TAddr>),
    Proposal(ProposalMessage<TAddr>),
    ForeignProposal(ProposalMessage<TAddr>),
    Vote(VoteMessage<TAddr>),
    RequestMissingTransactions(RequestMissingTransactionsMessage),
    RequestedTransaction(RequestedTransactionMessage),
    SyncRequest(SyncRequestMessage),
    SyncResponse(SyncResponseMessage<TAddr>),
}

impl<TAddr> HotstuffMessage<TAddr> {
    pub fn as_type_str(&self) -> &'static str {
        match self {
            HotstuffMessage::NewView(_) => "NewView",
            HotstuffMessage::Proposal(_) => "Proposal",
            HotstuffMessage::ForeignProposal(_) => "ForeignProposal",
            HotstuffMessage::Vote(_) => "Vote",
            HotstuffMessage::RequestMissingTransactions(_) => "RequestMissingTransactions",
            HotstuffMessage::RequestedTransaction(_) => "RequestedTransaction",
            HotstuffMessage::SyncRequest(_) => "SyncRequest",
            HotstuffMessage::SyncResponse(_) => "SyncResponse",
        }
    }

    pub fn epoch(&self) -> Epoch {
        match self {
            Self::NewView(msg) => msg.epoch,
            Self::Proposal(msg) => msg.block.epoch(),
            Self::ForeignProposal(msg) => msg.block.epoch(),
            Self::Vote(msg) => msg.epoch,
            Self::RequestMissingTransactions(msg) => msg.epoch,
            Self::RequestedTransaction(msg) => msg.epoch,
            Self::SyncRequest(msg) => msg.epoch,
            Self::SyncResponse(msg) => msg.epoch,
        }
    }

    pub fn proposal(&self) -> Option<&ProposalMessage<TAddr>> {
        match self {
            Self::Proposal(msg) => Some(msg),
            _ => None,
        }
    }
}

impl<TAddr> Display for HotstuffMessage<TAddr> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HotstuffMessage::NewView(msg) => write!(f, "NewView({})", msg.new_height),
            HotstuffMessage::Proposal(msg) => write!(f, "Proposal({})", msg.block.height()),
            HotstuffMessage::ForeignProposal(msg) => write!(f, "ForeignProposal({})", msg.block.height()),
            HotstuffMessage::Vote(msg) => write!(f, "Vote({}, {}, {})", msg.block_height, msg.block_id, msg.decision),
            HotstuffMessage::RequestMissingTransactions(msg) => {
                write!(f, "RequestMissingTransactions({})", msg.transactions.len())
            },
            HotstuffMessage::RequestedTransaction(msg) => write!(f, "RequestedTransaction({})", msg.transactions.len()),
            HotstuffMessage::SyncRequest(msg) => write!(f, "SyncRequest({})", msg.high_qc),
            HotstuffMessage::SyncResponse(msg) => write!(f, "SyncResponse({} block(s))", msg.blocks.len()),
        }
    }
}
