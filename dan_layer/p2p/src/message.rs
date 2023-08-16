//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_comms::peer_manager::PeerIdentityClaim;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::NodeAddressable;
use tari_transaction::Transaction;

#[derive(Debug, Clone, Serialize)]
pub enum DanMessage<TAddr> {
    // Consensus
    HotStuffMessage(Box<HotstuffMessage<TAddr>>),
    // Mempool
    NewTransaction(Box<Transaction>),
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
            Self::NewTransaction(tx) => format!("tx_{}", tx.id()),
            Self::NetworkAnnounce(msg) => format!("pk_{}", msg.identity),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkAnnounce<TAddr> {
    pub identity: TAddr,
    pub claim: PeerIdentityClaim,
}
