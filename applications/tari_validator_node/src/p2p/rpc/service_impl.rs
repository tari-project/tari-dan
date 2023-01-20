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
use std::convert::{TryFrom, TryInto};

use log::*;
use tari_comms::protocol::rpc::{Request, Response, RpcStatus, Streaming};
use tari_dan_common_types::{NodeAddressable, ShardId};
use tari_dan_core::{
    services::PeerProvider,
    storage::shard_store::{ShardStore, ShardStoreReadTransaction},
};
use tari_dan_engine::transaction::Transaction;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tokio::{sync::mpsc, task};

use crate::p2p::proto::rpc::{VnStateSyncRequest, VnStateSyncResponse};

const LOG_TARGET: &str = "vn::p2p::rpc";

use crate::p2p::{proto, rpc::ValidatorNodeRpcService, services::mempool::MempoolHandle};

pub struct ValidatorNodeRpcServiceImpl<TPeerProvider> {
    peer_provider: TPeerProvider,
    shard_state_store: SqliteShardStore,
    mempool: MempoolHandle,
}

impl<TPeerProvider: PeerProvider> ValidatorNodeRpcServiceImpl<TPeerProvider> {
    pub fn new(peer_provider: TPeerProvider, shard_state_store: SqliteShardStore, mempool: MempoolHandle) -> Self {
        Self {
            peer_provider,
            shard_state_store,
            mempool,
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
        match self.mempool.submit_transaction(transaction).await {
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

    async fn vn_state_sync(
        &self,
        request: Request<VnStateSyncRequest>,
    ) -> Result<Streaming<VnStateSyncResponse>, RpcStatus> {
        let (tx, rx) = mpsc::channel(100);
        let msg = request.into_message();

        let start_shard_id = msg
            .start_shard_id
            .and_then(|s| ShardId::try_from(s).ok())
            .ok_or_else(|| RpcStatus::bad_request("Invalid gRPC request: start_shard_id not provided"))?;
        let end_shard_id = msg
            .end_shard_id
            .and_then(|s| ShardId::try_from(s).ok())
            .ok_or_else(|| RpcStatus::bad_request("Invalid gRPC request: end_shard_id not provided"))?;

        let excluded_shards = msg
            .inventory
            .iter()
            .map(|s| {
                ShardId::try_from(s.bytes.as_slice())
                    .expect("Invalid gRPC request: failed to parse shard id's request data")
            })
            .collect::<Vec<ShardId>>();

        let shard_db = self.shard_state_store.clone();

        task::spawn(async move {
            let shards_substates_data = shard_db.with_read_tx(|tx| {
                tx.get_substate_states_by_range(start_shard_id, end_shard_id, excluded_shards.as_slice())
            });
            let substates = match shards_substates_data {
                Ok(s) => s,
                Err(err) => {
                    error!(target: LOG_TARGET, "{}", err);
                    let _ignore = tx.send(Err(RpcStatus::general(&err))).await;
                    return;
                },
            };

            if substates.is_empty() {
                return;
            }

            // select data from db where shard_id <= end_shard_id and shard_id >= start_shard_id
            for substate in substates {
                match proto::rpc::VnStateSyncResponse::try_from(substate) {
                    Ok(r) => {
                        if tx.send(Ok(r)).await.is_err() {
                            debug!(
                                target: LOG_TARGET,
                                "Peer stream closed by client before completing. Aborting"
                            );
                            break;
                        }
                    },
                    Err(e) => {
                        error!(target: LOG_TARGET, "{}", e);
                        let _ignore = tx.send(Err(RpcStatus::general(&e))).await;
                        return;
                    },
                }
            }
        });
        Ok(Streaming::new(rx))
    }
}
