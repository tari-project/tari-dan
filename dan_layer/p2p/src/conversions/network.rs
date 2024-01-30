//   Copyright 2023. The Tari Project
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

use std::convert::{TryFrom, TryInto};

use anyhow::anyhow;

use crate::{proto, DanMessage, Message};

// -------------------------------- Message -------------------------------- //
impl From<&Message> for proto::network::Message {
    fn from(msg: &Message) -> Self {
        match msg {
            Message::Dan(msg) => Self {
                message: Some(proto::network::message::Message::DanMessage(msg.into())),
            },
            Message::Consensus(msg) => Self {
                message: Some(proto::network::message::Message::Consensus(msg.into())),
            },
        }
    }
}

impl TryFrom<proto::network::Message> for Message {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::Message) -> Result<Self, Self::Error> {
        let msg_type = value.message.ok_or_else(|| anyhow!("Message type not provided"))?;
        match msg_type {
            proto::network::message::Message::DanMessage(msg) => Ok(Message::Dan(msg.try_into()?)),
            proto::network::message::Message::Consensus(msg) => Ok(Message::Consensus(msg.try_into()?)),
        }
    }
}

// -------------------------------- DanMessage -------------------------------- //

impl From<&DanMessage> for proto::network::DanMessage {
    fn from(msg: &DanMessage) -> Self {
        let message_tag = msg.get_message_tag();
        match msg {
            DanMessage::NewTransaction(msg) => Self {
                message: Some(proto::network::dan_message::Message::NewTransaction(
                    (**msg).clone().into(),
                )),
                message_tag,
            },
        }
    }
}

impl TryFrom<proto::network::DanMessage> for DanMessage {
    type Error = anyhow::Error;

    fn try_from(value: proto::network::DanMessage) -> Result<Self, Self::Error> {
        let msg_type = value.message.ok_or_else(|| anyhow!("Message type not provided"))?;
        match msg_type {
            proto::network::dan_message::Message::NewTransaction(msg) => {
                Ok(DanMessage::NewTransaction(Box::new(msg.try_into()?)))
            },
        }
    }
}
