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
use tari_dan_common_types::NodeAddressable;

use crate::Message;

#[async_trait]
pub trait OutboundService {
    type Error;
    type Addr: NodeAddressable + Send;

    async fn send_self<T: Into<Message> + Send>(&mut self, message: T) -> Result<(), Self::Error>;

    async fn send<T: Into<Message> + Send>(&mut self, to: Self::Addr, message: T) -> Result<(), Self::Error>;

    async fn broadcast<'a, I, T>(&mut self, committee: I, message: T) -> Result<(), Self::Error>
    where
        Self::Addr: 'a,
        I: IntoIterator<Item = &'a Self::Addr> + Send,
        T: Into<Message> + Send;

    async fn publish_gossip<TTopic: Into<String> + Send, TMsg: Into<Message> + Send>(
        &mut self,
        topic: TTopic,
        message: TMsg,
    ) -> Result<(), Self::Error>;
}
