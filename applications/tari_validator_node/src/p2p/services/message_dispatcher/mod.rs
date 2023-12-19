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

mod dispatcher;

pub use dispatcher::MessageDispatcher;
use libp2p::PeerId;

mod error;
mod inbound;
mod outbound;

pub use error::MessagingError;
pub use inbound::InboundMessaging;
pub use outbound::OutboundMessaging;
// -----------------------
// Messaging impl
// -----------------------
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::{NodeAddressable, PeerAddress};
use tari_dan_p2p::NewTransactionMessage;
use tari_networking::{NetworkingHandle, NetworkingService};
use tari_validator_node_rpc::proto;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::p2p::logging::MessageLogger;

pub fn spawn<TMsgLogger>(
    network: NetworkingHandle<proto::network::Message>,
    rx_inbound_msg: mpsc::Receiver<(PeerId, proto::network::Message)>,
    message_senders: DanMessageSenders<PeerAddress>,
    msg_logger: TMsgLogger,
) -> (
    OutboundMessaging<PeerAddress, TMsgLogger>,
    JoinHandle<anyhow::Result<()>>,
)
where
    TMsgLogger: MessageLogger + Clone + Send + 'static,
{
    let (loopback_sender, loopback_receiver) = mpsc::channel(100);
    let inbound = InboundMessaging::new(network.local_peer_id().into(), rx_inbound_msg, loopback_receiver);
    let outbound = OutboundMessaging::new(loopback_sender, network, msg_logger.clone());
    let dispatcher = MessageDispatcher::new(inbound, message_senders, msg_logger);
    let handle = dispatcher.spawn();
    (outbound, handle)
}

#[derive(Debug, Clone)]
pub struct DanMessageSenders<TAddr> {
    pub tx_consensus_message: mpsc::Sender<(TAddr, HotstuffMessage)>,
    pub tx_new_transaction_message: mpsc::Sender<(TAddr, NewTransactionMessage)>,
}

#[derive(Debug)]
pub struct DanMessageReceivers<TAddr> {
    pub rx_consensus_message: mpsc::Receiver<(TAddr, HotstuffMessage)>,
    pub rx_new_transaction_message: mpsc::Receiver<(TAddr, NewTransactionMessage)>,
}

pub fn new_messaging_channel<TAddr: NodeAddressable>(
    size: usize,
) -> (DanMessageSenders<TAddr>, DanMessageReceivers<TAddr>) {
    let (tx_consensus_message, rx_consensus_message) = mpsc::channel(size);
    let (tx_new_transaction_message, rx_new_transaction_message) = mpsc::channel(size);
    let senders = DanMessageSenders {
        tx_consensus_message,
        tx_new_transaction_message,
    };
    let receivers = DanMessageReceivers {
        rx_consensus_message,
        rx_new_transaction_message,
    };

    (senders, receivers)
}
