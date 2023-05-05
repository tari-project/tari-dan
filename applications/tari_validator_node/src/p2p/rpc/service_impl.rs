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
use tari_dan_common_types::{optional::Optional, NodeAddressable, PayloadId, ShardId};
use tari_dan_core::{
    services::PeerProvider,
    storage::shard_store::{ShardStore, ShardStoreReadTransaction},
};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::encode;
use tari_transaction::Transaction;
use tari_validator_node_rpc::{
    proto,
    proto::rpc::{
        GetSubstateRequest,
        GetSubstateResponse,
        GetTransactionResultRequest,
        GetTransactionResultResponse,
        PayloadResultStatus,
        SubstateStatus,
        VnStateSyncRequest,
        VnStateSyncResponse,
    },
    rpc_service::ValidatorNodeRpcService,
};
use tokio::{sync::mpsc, task};

const LOG_TARGET: &str = "tari::dan::p2p::rpc";

use crate::p2p::services::mempool::MempoolHandle;

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
        let request = request.into_message();
        let transaction: Transaction = request
            .transaction
            .ok_or_else(|| RpcStatus::bad_request("Missing transaction"))?
            .try_into()
            .map_err(|e| RpcStatus::bad_request(&format!("Malformed transaction: {}", e)))?;

        let transaction_hash = *transaction.hash();

        self.mempool
            .submit_transaction(transaction)
            .await
            .map_err(|e| RpcStatus::bad_request(&format!("Invalid transaction: {}", e)))?;

        debug!(target: LOG_TARGET, "Accepted instruction into mempool");

        Ok(Response::new(proto::rpc::SubmitTransactionResponse {
            transaction_hash: transaction_hash.to_vec(),
        }))
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
                if tx
                    .send(Ok(proto::rpc::GetPeersResponse {
                        identity: peer.identity.as_bytes().to_vec(),
                        addresses: peer.addresses.iter().map(|(a, _)| a.to_vec()).collect(),
                        claims: peer.addresses.into_iter().map(|(_, c)| c.into()).collect(),
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
            .map(|s| ShardId::try_from(s.bytes.as_slice()).map_err(|_| RpcStatus::bad_request("invalid shard_id")))
            .collect::<Result<Vec<_>, RpcStatus>>()?;

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

    async fn get_substate(&self, req: Request<GetSubstateRequest>) -> Result<Response<GetSubstateResponse>, RpcStatus> {
        let req = req.into_message();
        let mut tx = self
            .shard_state_store
            .create_read_tx()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let address = SubstateAddress::from_bytes(&req.address)
            .map_err(|e| RpcStatus::bad_request(&format!("Invalid encoded substate address: {}", e)))?;
        let shard_id = ShardId::from_address(&address, req.version);

        let substates = tx
            .get_substate_states(&[shard_id])
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let Some(substate) = substates.first() else {
            return Ok(Response::new(GetSubstateResponse {
                status: SubstateStatus::DoesNotExist as i32,
                ..Default::default()
            }));
        };

        let resp = if let Some(tx_hash) = substate.destroyed_payload_id() {
            let qcs = tx
                .get_high_qcs(tx_hash)
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            GetSubstateResponse {
                status: SubstateStatus::Down as i32,
                version: substate.version(),
                transaction_hash: tx_hash.into_array().to_vec(),
                quorum_certificates: qcs.into_iter().map(Into::into).collect(),
                ..Default::default()
            }
        } else {
            let tx_hash = substate.created_payload_id();
            let qcs = tx
                .get_high_qcs(tx_hash)
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            GetSubstateResponse {
                status: SubstateStatus::Up as i32,
                version: substate.version(),
                substate: substate.substate().to_bytes(),
                transaction_hash: substate.created_payload_id().into_array().to_vec(),
                quorum_certificates: qcs.into_iter().map(Into::into).collect(),
            }
        };

        Ok(Response::new(resp))
    }

    async fn get_transaction_result(
        &self,
        req: Request<GetTransactionResultRequest>,
    ) -> Result<Response<GetTransactionResultResponse>, RpcStatus> {
        let req = req.into_message();
        let mut tx = self
            .shard_state_store
            .create_read_tx()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let payload_id =
            PayloadId::try_from(req.payload_id).map_err(|_| RpcStatus::bad_request("Invalid payload id"))?;
        let payload = tx
            .get_payload(&payload_id)
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .ok_or_else(|| RpcStatus::not_found("Transaction not found"))?;

        if let Some(result) = payload.result() {
            Ok(Response::new(GetTransactionResultResponse {
                status: PayloadResultStatus::Finalized.into(),
                // For simplicity, we simply encode the whole result as a CBOR blob.
                execution_result: encode(result).map_err(RpcStatus::log_internal_error(LOG_TARGET))?,
            }))
        } else {
            Ok(Response::new(GetTransactionResultResponse {
                status: PayloadResultStatus::Pending.into(),
                ..Default::default()
            }))
        }
    }
}
