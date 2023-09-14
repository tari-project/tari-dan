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

use log::*;
use tari_comms::types::CommsPublicKey;
use tari_dan_p2p::{DanMessage, Message};
use tokio::task;

use crate::p2p::services::messaging::{DanMessageSenders, InboundMessaging};

const LOG_TARGET: &str = "tari::validator_node::p2p::services::message_dispatcher";

pub struct MessageDispatcher {
    inbound: InboundMessaging<CommsPublicKey>,
    message_senders: DanMessageSenders,
}

impl MessageDispatcher {
    pub fn new(inbound: InboundMessaging<CommsPublicKey>, message_senders: DanMessageSenders) -> Self {
        Self {
            inbound,
            message_senders,
        }
    }

    pub fn spawn(self) -> task::JoinHandle<anyhow::Result<()>> {
        task::spawn(self.run())
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        while let Some((from, msg)) = self.inbound.next_message().await {
            match msg {
                Message::Consensus(msg) => self.message_senders.tx_consensus_message.send((from, msg)).await?,
                Message::Dan(DanMessage::NewTransaction(msg)) => {
                    self.message_senders
                        .tx_new_transaction_message
                        .send((from, *msg))
                        .await?
                },
                Message::Dan(DanMessage::NetworkAnnounce(announce)) => {
                    self.message_senders.tx_network_announce.send((from, *announce)).await?
                },
            }
        }

        info!(target: LOG_TARGET, "Message dispatcher shutting down");
        Ok(())
    }
}
