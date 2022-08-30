//  Copyright 2022. The Tari Project
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

use std::{convert::TryInto, sync::Arc};

use futures::{pin_mut, Stream, StreamExt};
use log::{error, warn};
use tari_dan_core::services::mempool::service::{MempoolService, MempoolServiceHandle};
use tari_dan_engine::instruction::Transaction;
use tari_p2p::{
    comms_connector::{PeerMessage, SubscriptionFactory},
    domain_message::DomainMessage,
    tari_message::TariMessageType,
};
use tari_validator_node_grpc::rpc::SubmitTransactionRequest;

const LOG_TARGET: &str = "tari::validator_node::p2p::services::mempool::inbound";

const SUBSCRIPTION_LABEL: &str = "MempoolInbound";

#[derive(Clone)]
pub struct TariCommsMempoolInboundHandle {
    inbound_message_subscription_factory: Arc<SubscriptionFactory>,
    mempool: MempoolServiceHandle,
}

impl TariCommsMempoolInboundHandle {
    pub fn new(inbound_message_subscription_factory: Arc<SubscriptionFactory>, mempool: MempoolServiceHandle) -> Self {
        Self {
            inbound_message_subscription_factory,
            mempool,
        }
    }

    fn inbound_transaction_stream(&self) -> impl Stream<Item = DomainMessage<Transaction>> {
        self.inbound_message_subscription_factory
            .get_subscription(TariMessageType::DanConsensusMessage, SUBSCRIPTION_LABEL)
            .filter_map(extract_transaction)
    }

    pub async fn run(&mut self) {
        let inbound_transaction_stream = self.inbound_transaction_stream().fuse();
        pin_mut!(inbound_transaction_stream);

        loop {
            let mempool_service = self.mempool.clone();
            tokio::select! {
                Some(domain_msg) = inbound_transaction_stream.next() => {
                    handle_incoming_transaction(mempool_service, domain_msg).await;
                },
            }
        }
    }
}

async fn handle_incoming_transaction(
    mut mempool: MempoolServiceHandle,
    domain_request_msg: DomainMessage<Transaction>,
) {
    let (_, transaction) = domain_request_msg.into_origin_and_inner();

    let result = mempool.submit_transaction(&transaction).await;

    if let Err(e) = result {
        error!(
            target: LOG_TARGET,
            "Error handling incoming mempool transaction. {}",
            e.to_string()
        );
    }
}

async fn extract_transaction(msg: Arc<PeerMessage>) -> Option<DomainMessage<Transaction>> {
    match msg.decode_message::<SubmitTransactionRequest>() {
        Err(e) => {
            warn!(
                target: LOG_TARGET,
                "Could not decode inbound transaction message. {}",
                e.to_string()
            );
            None
        },
        Ok(request) => {
            let transaction: Transaction = match request.transaction.unwrap().try_into() {
                Ok(value) => value,
                Err(e) => {
                    warn!(
                        target: LOG_TARGET,
                        "Could not convert inbound transaction message. {}", e
                    );
                    return None;
                },
            };

            Some(DomainMessage {
                source_peer: msg.source_peer.clone(),
                dht_header: msg.dht_header.clone(),
                authenticated_origin: msg.authenticated_origin.clone(),
                inner: transaction,
            })
        },
    }
}
