//  Copyright 2021. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_consensus::messages::HotstuffMessage;
use tari_dan_p2p::{DanMessage, Message};
use tokio::sync::mpsc;

const _LOG_TARGET: &str = "tari::validator_node::p2p::services::messaging::inbound";

pub struct InboundMessaging<TAddr> {
    our_node_addr: TAddr,
    inbound_messages: mpsc::Receiver<(TAddr, DanMessage<TAddr>)>,
    inbound_consensus_messages: mpsc::Receiver<(TAddr, HotstuffMessage<TAddr>)>,
    loopback_receiver: mpsc::Receiver<Message<TAddr>>,
}

impl<TAddr: Clone> InboundMessaging<TAddr> {
    pub fn new(
        our_node_addr: TAddr,
        inbound_messages: mpsc::Receiver<(TAddr, DanMessage<TAddr>)>,
        inbound_consensus_messages: mpsc::Receiver<(TAddr, HotstuffMessage<TAddr>)>,
        loopback_receiver: mpsc::Receiver<Message<TAddr>>,
    ) -> Self {
        Self {
            our_node_addr,
            inbound_messages,
            inbound_consensus_messages,
            loopback_receiver,
        }
    }

    pub async fn next_message(&mut self) -> Option<(TAddr, Message<TAddr>)> {
        tokio::select! {
           // BIASED: messaging priority is loopback, consensus, then other
           biased;
           maybe_msg = self.loopback_receiver.recv() => maybe_msg.map(|msg| (self.our_node_addr.clone(), msg)),
           maybe_msg = self.inbound_consensus_messages.recv() => maybe_msg.map(|(from, msg)| (from, Message::Consensus(msg))),
           maybe_msg = self.inbound_messages.recv() => maybe_msg.map(|(from, msg)| (from, Message::Dan(msg))),
        }
    }
}
