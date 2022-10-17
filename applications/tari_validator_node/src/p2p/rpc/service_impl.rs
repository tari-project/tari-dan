//  Copyright 2021, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that
// the  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the
// following  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED
// WARRANTIES,  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
// PARTICULAR PURPOSE ARE  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL,  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
// PROCUREMENT OF SUBSTITUTE GOODS OR  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
// CAUSED AND ON ANY THEORY OF LIABILITY,  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
// OTHERWISE) ARISING IN ANY WAY OUT OF THE  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH
// DAMAGE.
use std::convert::TryInto;

use log::*;
use tari_comms::protocol::rpc::{Request, Response, RpcStatus, Streaming};
use tari_dan_core::services::{infrastructure_services::NodeAddressable, PeerProvider};
use tari_dan_engine::transaction::Transaction;
use tokio::{sync::mpsc, task};

const LOG_TARGET: &str = "vn::p2p::rpc";

use crate::p2p::{proto, rpc::ValidatorNodeRpcService, services::messaging::DanMessageSenders};

pub struct ValidatorNodeRpcServiceImpl<TPeerProvider> {
    message_senders: DanMessageSenders,
    peer_provider: TPeerProvider,
}

impl<TPeerProvider: PeerProvider> ValidatorNodeRpcServiceImpl<TPeerProvider> {
    pub fn new(message_senders: DanMessageSenders, peer_provider: TPeerProvider) -> Self {
        Self {
            message_senders,
            peer_provider,
        }
    }
}

#[tari_comms::async_trait]
impl<TPeerProvider> ValidatorNodeRpcService for ValidatorNodeRpcServiceImpl<TPeerProvider>
where TPeerProvider: PeerProvider + Clone + Send + Sync + 'static
{
    async fn submit_transaction(
        &self,
        request: Request<proto::validator_node::SubmitTransactionRequest>,
    ) -> Result<Response<proto::validator_node::SubmitTransactionResponse>, RpcStatus> {
        // let peer = request.context().fetch_peer().await?;
        let request = request.into_message();
        let transaction: Transaction = match request
            .transaction
            .ok_or_else(|| RpcStatus::bad_request("Missing transaction"))?
            .try_into()
        {
            Ok(value) => value,
            Err(e) => {
                return Err(RpcStatus::not_found(&format!("Could not convert transaaction: {}", e)));
            },
        };

        // TODO: Implement a mempool handle that returns if the transaction was accepted or not
        match self.message_senders.tx_new_transaction_message.send(transaction).await {
            Ok(_) => {
                debug!(target: LOG_TARGET, "Accepted instruction into mempool");
                return Ok(Response::new(proto::validator_node::SubmitTransactionResponse {
                    result: vec![],
                    status: "Accepted".to_string(),
                }));
            },
            Err(_err) => {
                // debug!(target: LOG_TARGET, "Mempool rejected instruction: {}", err);
                return Ok(Response::new(proto::validator_node::SubmitTransactionResponse {
                    result: vec![],
                    status: "Mempool has shut down".to_string(),
                }));
            },
        }
    }

    async fn get_peers(
        &self,
        _request: Request<proto::network::GetPeersRequest>,
    ) -> Result<Streaming<proto::network::GetPeersResponse>, RpcStatus> {
        let (tx, rx) = mpsc::channel(100);
        let peer_provider = self.peer_provider.clone();

        task::spawn(async move {
            let mut peer_iter = peer_provider.peers_for_current_epoch_iter().await;
            while let Some(Ok(peer)) = peer_iter.next() {
                if peer.identity_signature.is_none() {
                    continue;
                }
                if tx
                    .send(Ok(proto::network::GetPeersResponse {
                        identity: peer.identity.as_bytes().to_vec(),
                        identity_signature: peer.identity_signature.map(Into::into),
                        addresses: peer.addresses.into_iter().map(|a| a.to_vec()).collect(),
                    }))
                    .await
                    .is_err()
                {
                    debug!(
                        target: LOG_TARGET,
                        "Peer stream closed by client before completing. Aborting"
                    );
                    break;
                }
            }
        });

        Ok(Streaming::new(rx))
    }
}
