//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_dan_common_types::Epoch;
use tari_dan_storage::consensus_models::BlockId;

use super::{NewViewMessage, ProposalMessage, RequestedTransactionMessage, VoteMessage};
use crate::messages::RequestMissingTransactionsMessage;

#[derive(Debug, Clone, Serialize)]
pub enum HotstuffMessage {
    NewView(NewViewMessage),
    Proposal(ProposalMessage),
    Vote(VoteMessage),
    RequestMissingTransactions(RequestMissingTransactionsMessage),
    RequestedTransaction(RequestedTransactionMessage),
}

impl HotstuffMessage {
    pub fn epoch(&self) -> Epoch {
        match self {
            Self::NewView(msg) => msg.high_qc.epoch(),
            Self::Proposal(msg) => msg.block.epoch(),
            Self::Vote(msg) => msg.epoch,
            Self::RequestMissingTransactions(msg) => msg.epoch,
            Self::RequestedTransaction(msg) => msg.epoch,
        }
    }

    pub fn block_id(&self) -> &BlockId {
        match self {
            Self::NewView(msg) => msg.high_qc.block_id(),
            Self::Proposal(msg) => msg.block.id(),
            Self::Vote(msg) => &msg.block_id,
            Self::RequestMissingTransactions(msg) => &msg.block_id,
            Self::RequestedTransaction(msg) => &msg.block_id,
        }
    }
}
