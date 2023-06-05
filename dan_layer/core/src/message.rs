//   Copyright 2022. The Tari Project
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

use serde::Serialize;
use tari_comms::{multiaddr::Multiaddr, peer_manager::IdentitySignature};
use tari_dan_common_types::NodeAddressable;
use tari_dan_storage::models::{HotStuffMessage, VoteMessage};
use tari_transaction::Transaction;

use crate::workers::hotstuff_waiter::RecoveryMessage;

#[derive(Debug, Clone, Serialize)]
pub enum DanMessage<TPayload, TAddr> {
    // Consensus
    HotStuffMessage(Box<HotStuffMessage<TPayload, TAddr>>),
    VoteMessage(VoteMessage),
    // Mempool
    NewTransaction(Box<Transaction>),
    // Network
    NetworkAnnounce(Box<NetworkAnnounce<TAddr>>),
    // Recovery
    RecoveryMessage(Box<RecoveryMessage>),
}

impl<TPayload, TAddr: NodeAddressable> DanMessage<TPayload, TAddr> {
    pub fn as_type_str(&self) -> &'static str {
        match self {
            Self::HotStuffMessage(_) => "HotStuffMessage",
            Self::VoteMessage(_) => "VoteMessage",
            Self::NewTransaction(_) => "NewTransaction",
            Self::NetworkAnnounce(_) => "NetworkAnnounce",
            Self::RecoveryMessage(_) => "RecoveryMessage",
        }
    }

    pub fn get_message_tag(&self) -> String {
        match self {
            Self::HotStuffMessage(msg) => format!("shard_{}", msg.shard()),
            Self::VoteMessage(msg) => format!("node_{}", msg.local_node_hash()),
            Self::NewTransaction(tx) => format!("hash_{}", tx.hash()),
            Self::NetworkAnnounce(msg) => format!("pk_{}", msg.identity),
            Self::RecoveryMessage(msg) => format!("recovery_{:?}", msg),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkAnnounce<TAddr> {
    pub identity: TAddr,
    pub addresses: Vec<Multiaddr>,
    pub identity_signature: IdentitySignature,
}
