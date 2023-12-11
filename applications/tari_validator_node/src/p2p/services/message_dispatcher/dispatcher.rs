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
use tari_dan_common_types::PeerAddress;
use tari_dan_p2p::{DanMessage, Message};
use tokio::task;

use super::{DanMessageSenders, InboundMessaging};
use crate::p2p::logging::MessageLogger;

const LOG_TARGET: &str = "tari::validator_node::p2p::services::message_dispatcher";

pub struct MessageDispatcher<TMsgLogger> {
    inbound: InboundMessaging<PeerAddress>,
    message_senders: DanMessageSenders<PeerAddress>,
    msg_logger: TMsgLogger,
}

impl<TMsgLogger: MessageLogger + Send + 'static> MessageDispatcher<TMsgLogger> {
    pub fn new(
        inbound: InboundMessaging<PeerAddress>,
        message_senders: DanMessageSenders<PeerAddress>,
        msg_logger: TMsgLogger,
    ) -> Self {
        Self {
            inbound,
            message_senders,
            msg_logger,
        }
    }

    pub fn spawn(self) -> task::JoinHandle<anyhow::Result<()>> {
        task::spawn(self.run())
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        while let Some(result) = self.inbound.next_message().await {
            let (from, msg) = match result {
                Ok(from_and_msg) => from_and_msg,
                Err(err) => {
                    warn!(target: LOG_TARGET, "Inbound message error: {}", err);
                    continue;
                },
            };

            self.msg_logger
                .log_inbound_message(&from.to_string(), &msg.to_type_str(), &msg.get_message_tag(), &msg);

            match msg {
                Message::Consensus(msg) => self.message_senders.tx_consensus_message.send((from, msg)).await?,
                Message::Dan(DanMessage::NewTransaction(msg)) => {
                    self.message_senders
                        .tx_new_transaction_message
                        .send((from, *msg))
                        .await?
                },
            }
        }

        info!(target: LOG_TARGET, "Message dispatcher shutting down");
        Ok(())
    }
}
