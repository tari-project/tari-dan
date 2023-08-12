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
use tari_common_types::types::PublicKey;
use tari_comms::protocol::rpc::{Request, Response, RpcStatus, Streaming};
use tari_dan_common_types::{optional::Optional, NodeAddressable, ShardId};
use tari_dan_p2p::PeerProvider;
use tari_dan_storage::{
    consensus_models::{Block, BlockId, SubstateRecord, TransactionRecord},
    StateStore,
};
use tari_engine_types::virtual_substate::VirtualSubstateAddress;
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_state_store_sqlite::SqliteStateStore;
use tari_template_lib::{encode, prelude::tari_bor::decode_exact};
use tari_transaction::{Transaction, TransactionId};
use tari_validator_node_rpc::{
    proto,
    proto::rpc::{
        BlockSyncRequest,
        BlockSyncResponse,
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

use crate::{p2p::services::mempool::MempoolHandle, virtual_substate::VirtualSubstateManager};

const LOG_TARGET: &str = "tari::dan::p2p::rpc";

pub struct ValidatorNodeRpcServiceImpl<TPeerProvider> {
    peer_provider: TPeerProvider,
    shard_state_store: SqliteStateStore<PublicKey>,
    mempool: MempoolHandle,
    virtual_substate_manager: VirtualSubstateManager<SqliteStateStore<PublicKey>, EpochManagerHandle>,
}

impl<TPeerProvider: PeerProvider> ValidatorNodeRpcServiceImpl<TPeerProvider> {
    pub fn new(
        peer_provider: TPeerProvider,
        shard_state_store: SqliteStateStore<PublicKey>,
        mempool: MempoolHandle,
        virtual_substate_manager: VirtualSubstateManager<SqliteStateStore<PublicKey>, EpochManagerHandle>,
    ) -> Self {
        Self {
            peer_provider,
            shard_state_store,
            mempool,
            virtual_substate_manager,
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

        let transaction_id = *transaction.id();

        self.mempool
            .submit_transaction(transaction)
            .await
            .map_err(|e| RpcStatus::bad_request(&format!("Invalid transaction: {}", e)))?;

        debug!(target: LOG_TARGET, "Accepted instruction into mempool");

        Ok(Response::new(proto::rpc::SubmitTransactionResponse {
            transaction_id: transaction_id.as_bytes().to_vec(),
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
                        claims: peer.claims.into_iter().map(Into::into).collect(),
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
            .ok_or_else(|| RpcStatus::bad_request("start_shard_id malformed or not provided"))?;
        let end_shard_id = msg
            .end_shard_id
            .and_then(|s| ShardId::try_from(s).ok())
            .ok_or_else(|| RpcStatus::bad_request("end_shard_id malformed or not provided"))?;

        let excluded_shards = msg
            .inventory
            .iter()
            .map(|s| ShardId::try_from(s.bytes.as_slice()).map_err(|_| RpcStatus::bad_request("invalid shard_id")))
            .collect::<Result<Vec<_>, RpcStatus>>()?;

        let shard_db = self.shard_state_store.clone();

        task::spawn(async move {
            let shards_substates_data = shard_db.with_read_tx(|tx| {
                SubstateRecord::get_many_within_range(tx, start_shard_id..=end_shard_id, excluded_shards.as_slice())
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

        let shard_id = ShardId::from_bytes(&req.shard)
            .map_err(|e| RpcStatus::bad_request(&format!("Invalid encoded substate address: {}", e)))?;

        let mut tx = self
            .shard_state_store
            .create_read_tx()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let maybe_substate = SubstateRecord::get(&mut tx, &shard_id)
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let Some(substate) = maybe_substate else {
            return Ok(Response::new(GetSubstateResponse {
                status: SubstateStatus::DoesNotExist as i32,
                ..Default::default()
            }));
        };

        let created_qc = substate
            .get_created_quorum_certificate(&mut tx)
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let resp = if substate.is_destroyed() {
            let destroyed_qc = substate
                .get_destroyed_quorum_certificate(&mut tx)
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            GetSubstateResponse {
                status: SubstateStatus::Down as i32,
                address: substate.substate_address().to_bytes(),
                version: substate.version(),
                created_transaction_hash: substate.created_by_transaction().into_array().to_vec(),
                destroyed_transaction_hash: substate
                    .destroyed_by_transaction()
                    .map(|id| id.into_array().to_vec())
                    .unwrap_or_default(),
                quorum_certificates: Some(created_qc)
                    .into_iter()
                    .chain(destroyed_qc)
                    .map(Into::into)
                    .collect(),
                ..Default::default()
            }
        } else {
            GetSubstateResponse {
                status: SubstateStatus::Up as i32,
                address: substate.substate_address().to_bytes(),
                version: substate.version(),
                substate: substate.substate_value().to_bytes(),
                created_transaction_hash: substate.created_by_transaction().into_array().to_vec(),
                destroyed_transaction_hash: vec![],
                quorum_certificates: vec![created_qc.into()],
            }
        };

        Ok(Response::new(resp))
    }

    async fn get_virtual_substate(
        &self,
        req: Request<proto::rpc::GetVirtualSubstateRequest>,
    ) -> Result<Response<proto::rpc::GetVirtualSubstateResponse>, RpcStatus> {
        let req = req.into_message();

        let address = decode_exact::<VirtualSubstateAddress>(&req.address)
            .map_err(|e| RpcStatus::bad_request(&format!("Invalid encoded substate address: {}", e)))?;

        let substate = self
            .virtual_substate_manager
            .generate_for_address(&address)
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let resp = proto::rpc::GetVirtualSubstateResponse {
            substate: encode(&substate)
                .map_err(|e| RpcStatus::general(&format!("Unable to encode substate address: {}", e)))?,
            // TODO: evidence for the correctness of the substate
            quorum_certificates: vec![],
        };

        Ok(Response::new(resp))
    }

    async fn sync_blocks(&self, req: Request<BlockSyncRequest>) -> Result<Streaming<BlockSyncResponse>, RpcStatus> {
        let (sender, receiver) = mpsc::channel(100);
        let req = req.into_message();

        let block_id = BlockId::try_from(req.block_id)
            .map_err(|e| RpcStatus::bad_request(&format!("Invalid encoded block id: {}", e)))?;
        let shard_db = self.shard_state_store.clone();
        // Check if we have such block
        shard_db
            .with_read_tx(|tx| Block::get(tx, &block_id))
            .map_err(|e| RpcStatus::general(&e))?;
        let epoch = req.epoch;
        task::spawn(async move {
            let block = shard_db.with_read_tx(|tx| Block::get_tip(tx, tari_dan_common_types::Epoch(epoch)));
            match block {
                Ok(block) => {
                    let mut block = block;
                    while *block.id() != block_id {
                        if sender
                            .send(Ok(BlockSyncResponse {
                                block: Some(block.clone().into()),
                            }))
                            .await
                            .is_err()
                        {
                            debug!(
                                target: LOG_TARGET,
                                "Peer stream closed by client before completing. Aborting"
                            );
                        }
                        let x = shard_db.with_read_tx(|tx| block.get_parent(tx));
                        block = match x {
                            Ok(block) => block,
                            Err(e) => {
                                let _ignore = sender.send(Err(RpcStatus::general(&e))).await;
                                return;
                            },
                        };
                    }
                },
                Err(e) => {
                    let _ignore = sender.send(Err(RpcStatus::general(&e))).await;
                },
            }
        });
        Ok(Streaming::new(receiver))
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
        let tx_id = TransactionId::try_from(req.transaction_id)
            .map_err(|_| RpcStatus::bad_request("Invalid transaction id"))?;
        let transaction = TransactionRecord::get(&mut tx, &tx_id)
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .ok_or_else(|| RpcStatus::not_found("Transaction not found"))?;

        let Some(final_decision) = transaction.final_decision() else {
            return Ok(Response::new(GetTransactionResultResponse {
                status: PayloadResultStatus::Pending.into(),
                ..Default::default()
            }));
        };

        let abort_details = transaction.abort_details().cloned().unwrap_or_default();

        Ok(Response::new(GetTransactionResultResponse {
            status: PayloadResultStatus::Finalized.into(),
            // For simplicity, we simply encode the whole result as a CBOR blob.
            execution_result: transaction
                .into_final_result()
                .as_ref()
                .map(encode)
                .transpose()
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
                .unwrap_or_default(),
            final_decision: proto::consensus::Decision::from(final_decision) as i32,
            abort_details,
        }))
    }
}
