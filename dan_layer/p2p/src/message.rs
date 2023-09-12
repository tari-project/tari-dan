//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_comms::peer_manager::PeerIdentityClaim;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::{NodeAddressable, ShardId};
use tari_transaction::Transaction;

#[derive(Debug, Clone, Serialize)]
pub enum DanMessage<TAddr> {
    // Consensus
    HotStuffMessage(Box<HotstuffMessage<TAddr>>),
    // Mempool
    NewTransaction(Box<NewTransactionMessage>),
    // Network
    NetworkAnnounce(Box<NetworkAnnounce<TAddr>>),
}

impl<TAddr: NodeAddressable> DanMessage<TAddr> {
    pub fn as_type_str(&self) -> &'static str {
        match self {
            Self::HotStuffMessage(_) => "HotStuffMessage",
            Self::NewTransaction(_) => "NewTransaction",
            Self::NetworkAnnounce(_) => "NetworkAnnounce",
        }
    }

    pub fn get_message_tag(&self) -> String {
        match self {
            Self::HotStuffMessage(msg) => format!("hotstuff_{}", msg.block_id()),
            Self::NewTransaction(msg) => format!("tx_{}", msg.transaction.id()),
            Self::NetworkAnnounce(msg) => format!("pk_{}", msg.identity),
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
