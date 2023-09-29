//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use serde::Serialize;
use tari_comms::peer_manager::PeerIdentityClaim;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::{NodeAddressable, ShardId};
use tari_transaction::Transaction;

#[derive(Debug, Clone)]
pub enum Message<TAddr> {
    Consensus(HotstuffMessage<TAddr>),
    Dan(DanMessage<TAddr>),
}

impl<TAddr> From<HotstuffMessage<TAddr>> for Message<TAddr> {
    fn from(msg: HotstuffMessage<TAddr>) -> Self {
        Self::Consensus(msg)
    }
}

impl<TAddr> From<DanMessage<TAddr>> for Message<TAddr> {
    fn from(msg: DanMessage<TAddr>) -> Self {
        Self::Dan(msg)
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum DanMessage<TAddr> {
    // Mempool
    NewTransaction(Box<NewTransactionMessage>),
    // Network
    NetworkAnnounce(Box<NetworkAnnounce<TAddr>>),
}

impl<TAddr: NodeAddressable> DanMessage<TAddr> {
    pub fn as_type_str(&self) -> &'static str {
        match self {
            Self::NewTransaction(_) => "NewTransaction",
            Self::NetworkAnnounce(_) => "NetworkAnnounce",
        }
    }

    pub fn get_message_tag(&self) -> String {
        match self {
            Self::NewTransaction(msg) => format!("tx_{}", msg.transaction.id()),
            Self::NetworkAnnounce(msg) => format!("pk_{}", msg.identity),
        }
    }
}

impl<TAddr> From<NewTransactionMessage> for DanMessage<TAddr> {
    fn from(value: NewTransactionMessage) -> Self {
        Self::NewTransaction(Box::new(value))
    }
}

impl<TAddr: Display> Display for DanMessage<TAddr> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NewTransaction(msg) => write!(f, "NewTransaction({})", msg.transaction.id()),
            Self::NetworkAnnounce(_) => write!(f, "NetworkAnnounce"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkAnnounce<TAddr> {
    pub identity: TAddr,
    pub claim: PeerIdentityClaim,
}

#[derive(Debug, Clone, Serialize)]
pub struct NewTransactionMessage {
    pub transaction: Transaction,
    /// Output shards that a validator has determined by executing the transaction
    // TODO: The only way to verify this is to execute the transaction again.
    pub output_shards: Vec<ShardId>,
}
