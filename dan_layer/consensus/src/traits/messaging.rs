// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use async_trait::async_trait;
use tari_dan_common_types::{NodeAddressable, ShardGroup};

use crate::messages::HotstuffMessage;

#[async_trait]
pub trait OutboundMessaging {
    type Addr: NodeAddressable + Send;

    async fn send_self<T: Into<HotstuffMessage> + Send>(&mut self, message: T) -> Result<(), OutboundMessagingError>;

    async fn send<T: Into<HotstuffMessage> + Send>(
        &mut self,
        to: Self::Addr,
        message: T,
    ) -> Result<(), OutboundMessagingError>;

    async fn multicast<'a, T>(&mut self, shard_group: ShardGroup, message: T) -> Result<(), OutboundMessagingError>
    where
        Self::Addr: 'a,
        T: Into<HotstuffMessage> + Send;
}

#[async_trait]
pub trait InboundMessaging {
    type Addr: NodeAddressable + Send;

    async fn next_message(&mut self) -> Option<Result<(Self::Addr, HotstuffMessage), InboundMessagingError>>;
}

#[derive(Debug, thiserror::Error)]
pub enum InboundMessagingError {
    #[error("Invalid message: {reason}")]
    InvalidMessage { reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum OutboundMessagingError {
    #[error("Failed to enqueue message: {reason}")]
    FailedToEnqueueMessage { reason: String },
    #[error(transparent)]
    UpstreamError(anyhow::Error),
}

impl OutboundMessagingError {
    pub fn from_error<E>(err: E) -> Self
    where E: Into<anyhow::Error> {
        Self::UpstreamError(err.into())
    }
}
