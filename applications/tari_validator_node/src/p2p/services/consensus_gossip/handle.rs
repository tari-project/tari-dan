//   Copyright 2024. The Tari Project
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

use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::ShardGroup;
use tokio::sync::{mpsc, oneshot};

use super::ConsensusGossipError;

pub enum ConsensusGossipRequest {
    Multicast {
        shard_group: ShardGroup,
        message: HotstuffMessage,
        reply: oneshot::Sender<Result<(), ConsensusGossipError>>,
    },
    GetLocalShardGroup {
        reply: oneshot::Sender<Result<Option<ShardGroup>, ConsensusGossipError>>,
    },
}

#[derive(Debug)]
pub struct ConsensusGossipHandle {
    tx_consensus_request: mpsc::Sender<ConsensusGossipRequest>,
}

impl Clone for ConsensusGossipHandle {
    fn clone(&self) -> Self {
        ConsensusGossipHandle {
            tx_consensus_request: self.tx_consensus_request.clone(),
        }
    }
}

impl ConsensusGossipHandle {
    pub(super) fn new(tx_consensus_request: mpsc::Sender<ConsensusGossipRequest>) -> Self {
        Self { tx_consensus_request }
    }

    pub async fn multicast(
        &self,
        shard_group: ShardGroup,
        message: HotstuffMessage,
    ) -> Result<(), ConsensusGossipError> {
        let (tx, rx) = oneshot::channel();
        self.tx_consensus_request
            .send(ConsensusGossipRequest::Multicast {
                shard_group,
                message,
                reply: tx,
            })
            .await?;

        rx.await?
    }

    pub async fn get_local_shard_group(&self) -> Result<Option<ShardGroup>, ConsensusGossipError> {
        let (tx, rx) = oneshot::channel();
        self.tx_consensus_request
            .send(ConsensusGossipRequest::GetLocalShardGroup { reply: tx })
            .await?;

        rx.await?
    }
}
