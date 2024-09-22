//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use anyhow::bail;
use serde::Serialize;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::SubstateAddress;
use tari_transaction::Transaction;

#[derive(Debug, Clone)]
pub enum Message {
    Consensus(HotstuffMessage),
    Dan(DanMessage),
}

impl From<HotstuffMessage> for Message {
    fn from(msg: HotstuffMessage) -> Self {
        Self::Consensus(msg)
    }
}

impl From<DanMessage> for Message {
    fn from(msg: DanMessage) -> Self {
        Self::Dan(msg)
    }
}

impl Message {
    pub fn to_type_str(&self) -> String {
        match self {
            Self::Consensus(msg) => format!("Consensus({})", msg.as_type_str()),
            Self::Dan(msg) => format!("Dan({})", msg.as_type_str()),
        }
    }

    pub fn get_message_tag(&self) -> String {
        match self {
            Self::Consensus(msg) => msg.as_type_str().to_string(),
            Self::Dan(msg) => msg.get_message_tag(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum DanMessage {
    // Mempool
    NewTransaction(Box<NewTransactionMessage>),
}

impl DanMessage {
    pub fn as_type_str(&self) -> &'static str {
        match self {
            Self::NewTransaction(_) => "NewTransaction",
        }
    }

    pub fn get_message_tag(&self) -> String {
        match self {
            Self::NewTransaction(msg) => format!("tx_{}", msg.transaction.id()),
        }
    }
}

impl TryFrom<Message> for DanMessage {
    type Error = anyhow::Error;

    fn try_from(msg: Message) -> Result<Self, Self::Error> {
        if let Message::Dan(msg) = msg {
            Ok(msg)
        } else {
            bail!("Invalid variant")
        }
    }
}

impl TryFrom<Message> for HotstuffMessage {
    type Error = anyhow::Error;

    fn try_from(msg: Message) -> Result<Self, Self::Error> {
        if let Message::Consensus(msg) = msg {
            Ok(msg)
        } else {
            bail!("Invalid variant")
        }
    }
}

impl From<NewTransactionMessage> for DanMessage {
    fn from(value: NewTransactionMessage) -> Self {
        Self::NewTransaction(Box::new(value))
    }
}

impl Display for DanMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewTransaction(msg) => write!(f, "NewTransaction({})", msg.transaction.id()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NewTransactionMessage {
    pub transaction: Transaction,
    /// Output shards that a validator has determined by executing the transaction
    // TODO: The only way to verify this is to execute the transaction again.
    pub output_shards: Vec<SubstateAddress>,
}
