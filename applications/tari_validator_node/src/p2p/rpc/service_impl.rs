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
use std::{
    convert::{TryFrom, TryInto},
    sync::Arc,
};

use log::*;
use tari_comms::protocol::rpc::{Request, Response, RpcStatus, Streaming};
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    services::{infrastructure_services::NodeAddressable, PeerProvider},
    storage::shard_store::{ShardStoreFactory, ShardStoreTransaction},
};
use tari_dan_engine::transaction::Transaction;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStoreFactory;
use tokio::{
    sync::{mpsc, Mutex},
    task,
};

use crate::p2p::proto::{
    consensus::QuorumCertificate,
    rpc::{GetVnStateInventoryRequest, GetVnStateInventoryResponse, VnStateSyncRequest, VnStateSyncResponse},
};

const LOG_TARGET: &str = "vn::p2p::rpc";

use crate::p2p::{proto, rpc::ValidatorNodeRpcService, services::messaging::DanMessageSenders};

pub struct ValidatorNodeRpcServiceImpl<TPeerProvider> {
    message_senders: DanMessageSenders,
    peer_provider: TPeerProvider,
    shard_state_store: SqliteShardStoreFactory,
}

impl<TPeerProvider: PeerProvider> ValidatorNodeRpcServiceImpl<TPeerProvider> {
    pub fn new(
        message_senders: DanMessageSenders,
        peer_provider: TPeerProvider,
        shard_state_store: SqliteShardStoreFactory,
    ) -> Self {
        Self {
            message_senders,
            peer_provider,
            shard_state_store,
        }
    }
}

#[tari_comms::async_trait]
impl<TPeerProvider> ValidatorNodeRpcService for ValidatorNodeRpcServiceImpl<TPeerProvider>
where TPeerProvider: PeerProvider + Clone + Send + Sync + 'static
{
    async fn submit_transaction(
        &self,
        request: Request<proto::rpc::SubmitTransactionRequest>,
    ) -> Result<Response<proto::rpc::SubmitTransactionResponse>, RpcStatus> {
        // let peer = request.context().fetch_peer().await?;
        let request = request.into_message();
        let transaction: Transaction = match request
            .transaction
            .ok_or_else(|| RpcStatus::bad_request("Missing transaction"))?
            .try_into()
        {
            Ok(value) => value,
            Err(e) => {
                return Err(RpcStatus::not_found(&format!("Could not convert transaction: {}", e)));
            },
        };

        // TODO: Implement a mempool handle that returns if the transaction was accepted or not
        match self.message_senders.tx_new_transaction_message.send(transaction).await {
            Ok(_) => {
                debug!(target: LOG_TARGET, "Accepted instruction into mempool");
                return Ok(Response::new(proto::rpc::SubmitTransactionResponse {
                    result: vec![],
                    status: "Accepted".to_string(),
                }));
            },
            Err(_err) => {
                // debug!(target: LOG_TARGET, "Mempool rejected instruction: {}", err);
                return Ok(Response::new(proto::rpc::SubmitTransactionResponse {
                    result: vec![],
                    status: "Mempool has shut down".to_string(),
                }));
            },
        }
    }

    async fn get_peers(
        &self,
        _request: Request<proto::rpc::GetPeersRequest>,
    ) -> Result<Streaming<proto::rpc::GetPeersResponse>, RpcStatus> {
        let (tx, rx) = mpsc::channel(100);
        let peer_provider = self.peer_provider.clone();

        task::spawn(async move {
            let mut peer_iter = peer_provider.peers_for_current_epoch_iter().await;
            while let Some(Ok(peer)) = peer_iter.next() {
                if peer.identity_signature.is_none() {
                    continue;
                }
                if tx
                    .send(Ok(proto::rpc::GetPeersResponse {
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

    async fn get_vn_state_inventory(
        &self,
        request: Request<GetVnStateInventoryRequest>,
    ) -> Result<Response<GetVnStateInventoryResponse>, RpcStatus> {
        let request = request.into_message();
        let start_shard_id = request
            .start_shard_id
            .and_then(|s| ShardId::try_from(s).ok())
            .ok_or_else(|| RpcStatus::bad_request("Invalid gRPC request: start_shard_id not provided"))?;
        let end_shard_id = request
            .end_shard_id
            .and_then(|s| ShardId::try_from(s).ok())
            .ok_or_else(|| RpcStatus::bad_request("Invalid gRPC request: end_shard_id not provided"))?;

        let store_tx = self
            .shard_state_store
            .create_tx()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let inventory = store_tx
            .get_state_inventory(start_shard_id, end_shard_id)
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        Ok(Response::new(GetVnStateInventoryResponse {
            inventory: inventory.into_iter().map(|item| item.into()).collect(),
        }))
    }

    async fn vn_state_sync(
        &self,
        request: Request<VnStateSyncRequest>,
    ) -> Result<Streaming<VnStateSyncResponse>, RpcStatus> {
        let (tx, rx) = mpsc::channel(100);
        let msg = request.into_message();

        let missing_shard_ids = msg
            .missing_shard_state_ids
            .iter()
            .map(|s| {
                ShardId::try_from(s.bytes.as_slice())
                    .expect("Invalid gRPC request: failed to parse shard id's request data")
            })
            .collect::<Vec<ShardId>>();

        if missing_shard_ids.is_empty() {
            return Err(RpcStatus::bad_request(
                "Invalid gRPC request: request should contain at least one shard id available",
            ));
        }

        let shard_store = self.shard_state_store.clone();

        let store_tx = Arc::new(Mutex::new(
            shard_store
                .create_tx()
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?,
        ));

        task::spawn(async move {
            loop {
                let shards_substates_data = store_tx.lock().await.get_substate_states(missing_shard_ids.as_slice());
                let substates_data = match shards_substates_data {
                    Ok(s) => s,
                    Err(err) => {
                        error!(target: LOG_TARGET, "{}", err);
                        let _ignore = tx.send(Err(RpcStatus::general(&err))).await;
                        return;
                    },
                };

                // select data from db where shard_id <= end_shard_id and shard_id >= start_shard_id
                for substate_data in substates_data {
                    let shard_id = proto::common::ShardId::from(substate_data.shard());
                    let substate_state = proto::consensus::SubstateState::from(substate_data.substate().clone());
                    let node_height = substate_data.height().as_u64();
                    let tree_node_hash = if let Some(h) = substate_data.tree_node_hash() {
                        Vec::from(h.as_bytes())
                    } else {
                        vec![]
                    };
                    let payload_id = Vec::from(substate_data.payload_id().as_bytes());
                    let certificate = substate_data.certificate().clone().map(QuorumCertificate::from);

                    let response = proto::rpc::VnStateSyncResponse {
                        shard_id: Some(shard_id),
                        substate_state: Some(substate_state),
                        node_height,
                        tree_node_hash,
                        payload_id,
                        certificate,
                    };
                    // if send returns error, the client has closed the connection, so we break the loop
                    if tx.send(Ok(response)).await.is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Streaming::new(rx))
    }
}
