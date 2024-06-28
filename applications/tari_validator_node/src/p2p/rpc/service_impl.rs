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
use tari_bor::{decode_exact, encode};
use tari_dan_common_types::{optional::Optional, shard::Shard, Epoch, PeerAddress, SubstateAddress};
use tari_dan_p2p::{
    proto,
    proto::rpc::{
        GetCheckpointRequest,
        GetCheckpointResponse,
        GetHighQcRequest,
        GetHighQcResponse,
        GetSubstateRequest,
        GetSubstateResponse,
        GetTransactionResultRequest,
        GetTransactionResultResponse,
        PayloadResultStatus,
        SubstateStatus,
        SyncBlocksRequest,
        SyncBlocksResponse,
        SyncStateRequest,
        SyncStateResponse,
    },
};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        EpochCheckpoint,
        HighQc,
        LockedBlock,
        QuorumCertificate,
        StateTransitionId,
        SubstateRecord,
        TransactionRecord,
    },
    StateStore,
    StorageError,
};
use tari_engine_types::virtual_substate::VirtualSubstateId;
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_rpc_framework::{Request, Response, RpcStatus, Streaming};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tari_validator_node_rpc::rpc_service::ValidatorNodeRpcService;
use tokio::{sync::mpsc, task};

use crate::{
    p2p::{
        rpc::{block_sync_task::BlockSyncTask, state_sync_task::StateSyncTask},
        services::mempool::MempoolHandle,
    },
    virtual_substate::VirtualSubstateManager,
};

const LOG_TARGET: &str = "tari::dan::p2p::rpc";

pub struct ValidatorNodeRpcServiceImpl {
    epoch_manager: EpochManagerHandle<PeerAddress>,
    shard_state_store: SqliteStateStore<PeerAddress>,
    mempool: MempoolHandle,
    virtual_substate_manager: VirtualSubstateManager<SqliteStateStore<PeerAddress>, EpochManagerHandle<PeerAddress>>,
}

impl ValidatorNodeRpcServiceImpl {
    pub fn new(
        epoch_manager: EpochManagerHandle<PeerAddress>,
        shard_state_store: SqliteStateStore<PeerAddress>,
        mempool: MempoolHandle,
        virtual_substate_manager: VirtualSubstateManager<
            SqliteStateStore<PeerAddress>,
            EpochManagerHandle<PeerAddress>,
        >,
    ) -> Self {
        Self {
            epoch_manager,
            shard_state_store,
            mempool,
            virtual_substate_manager,
        }
    }
}

#[async_trait::async_trait]
impl ValidatorNodeRpcService for ValidatorNodeRpcServiceImpl {
    async fn submit_transaction(
        &self,
        request: Request<proto::rpc::SubmitTransactionRequest>,
    ) -> Result<Response<proto::rpc::SubmitTransactionResponse>, RpcStatus> {
        let request = request.into_message();
        let transaction: Transaction = request
            .transaction
            .ok_or_else(|| RpcStatus::bad_request("Missing transaction"))?
            .try_into()
            .map_err(|e| RpcStatus::bad_request(format!("Malformed transaction: {}", e)))?;

        let transaction_id = *transaction.id();

        self.mempool
            .submit_transaction(transaction)
            .await
            .map_err(|e| RpcStatus::bad_request(format!("Invalid transaction: {}", e)))?;

        debug!(target: LOG_TARGET, "Accepted instruction into mempool");

        Ok(Response::new(proto::rpc::SubmitTransactionResponse {
            transaction_id: transaction_id.as_bytes().to_vec(),
        }))
    }

    async fn get_substate(&self, req: Request<GetSubstateRequest>) -> Result<Response<GetSubstateResponse>, RpcStatus> {
        let req = req.into_message();

        let address = SubstateAddress::from_bytes(&req.address)
            .map_err(|e| RpcStatus::bad_request(format!("Invalid encoded substate id: {}", e)))?;

        let tx = self
            .shard_state_store
            .create_read_tx()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let maybe_substate = SubstateRecord::get(&tx, &address)
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let Some(substate) = maybe_substate else {
            return Ok(Response::new(GetSubstateResponse {
                status: SubstateStatus::DoesNotExist as i32,
                ..Default::default()
            }));
        };

        let created_qc = substate
            .get_created_quorum_certificate(&tx)
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let resp = if substate.is_destroyed() {
            let destroyed_qc = substate
                .get_destroyed_quorum_certificate(&tx)
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            GetSubstateResponse {
                status: SubstateStatus::Down as i32,
                address: substate.substate_id().to_bytes(),
                version: substate.version(),
                created_transaction_hash: substate.created_by_transaction().into_array().to_vec(),
                destroyed_transaction_hash: substate
                    .destroyed()
                    .map(|destroyed| destroyed.by_transaction.as_bytes().to_vec())
                    .unwrap_or_default(),
                quorum_certificates: Some(created_qc)
                    .into_iter()
                    .chain(destroyed_qc)
                    .map(|qc| (&qc).into())
                    .collect(),
                ..Default::default()
            }
        } else {
            GetSubstateResponse {
                status: SubstateStatus::Up as i32,
                address: substate.substate_id().to_bytes(),
                version: substate.version(),
                substate: substate.substate_value().to_bytes(),
                created_transaction_hash: substate.created_by_transaction().into_array().to_vec(),
                destroyed_transaction_hash: vec![],
                quorum_certificates: vec![(&created_qc).into()],
            }
        };

        Ok(Response::new(resp))
    }

    async fn get_virtual_substate(
        &self,
        req: Request<proto::rpc::GetVirtualSubstateRequest>,
    ) -> Result<Response<proto::rpc::GetVirtualSubstateResponse>, RpcStatus> {
        let req = req.into_message();

        let address = decode_exact::<VirtualSubstateId>(&req.address)
            .map_err(|e| RpcStatus::bad_request(format!("Invalid encoded substate id: {}", e)))?;

        let substate = self
            .virtual_substate_manager
            .generate_for_address(&address)
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let resp = proto::rpc::GetVirtualSubstateResponse {
            substate: encode(&substate).map_err(|e| RpcStatus::general(format!("Unable to encode substate: {}", e)))?,
            // TODO: evidence for the correctness of the substate
            quorum_certificates: vec![],
        };

        Ok(Response::new(resp))
    }

    async fn get_transaction_result(
        &self,
        req: Request<GetTransactionResultRequest>,
    ) -> Result<Response<GetTransactionResultResponse>, RpcStatus> {
        let req = req.into_message();
        let tx = self
            .shard_state_store
            .create_read_tx()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let tx_id = TransactionId::try_from(req.transaction_id)
            .map_err(|_| RpcStatus::bad_request("Invalid transaction id"))?;
        let transaction = TransactionRecord::get(&tx, &tx_id)
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

            final_decision: proto::consensus::Decision::from(final_decision) as i32,
            execution_time_ms: transaction
                .execution_time()
                .map(|t| u64::try_from(t.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or_default(),
            finalized_time_ms: transaction
                .finalized_time()
                .map(|t| u64::try_from(t.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or_default(),
            abort_details,
            // For simplicity, we simply encode the whole result as a CBOR blob.
            execution_result: transaction
                .into_final_result()
                .as_ref()
                .map(encode)
                .transpose()
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
                .unwrap_or_default(),
        }))
    }

    async fn sync_blocks(
        &self,
        request: Request<SyncBlocksRequest>,
    ) -> Result<Streaming<SyncBlocksResponse>, RpcStatus> {
        let req = request.into_message();
        let store = self.shard_state_store.clone();

        let (sender, receiver) = mpsc::channel(10);

        let start_block_id = BlockId::try_from(req.start_block_id)
            .map_err(|e| RpcStatus::bad_request(format!("Invalid encoded block id: {}", e)))?;
        // Check if we have the blocks
        let start_block = store
            .with_read_tx(|tx| Block::get(tx, &start_block_id).optional())
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .ok_or_else(|| RpcStatus::not_found(format!("start_block_id {start_block_id} not found")))?;

        // Check that the start block
        let locked_block = store
            .with_read_tx(|tx| LockedBlock::get(tx).optional())
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .ok_or_else(|| RpcStatus::not_found("No locked block"))?;
        if start_block.height() > locked_block.height() {
            return Err(RpcStatus::not_found(format!(
                "start_block_id {} is after locked block {}",
                start_block_id, locked_block
            )));
        }

        task::spawn(
            BlockSyncTask::new(
                self.shard_state_store.clone(),
                start_block,
                req.up_to_epoch.map(|epoch| epoch.into()),
                sender,
            )
            .run(),
        );

        Ok(Streaming::new(receiver))
    }

    async fn get_high_qc(&self, _request: Request<GetHighQcRequest>) -> Result<Response<GetHighQcResponse>, RpcStatus> {
        let high_qc = self
            .shard_state_store
            .with_read_tx(|tx| {
                HighQc::get(tx)
                    .optional()?
                    .map(|hqc| hqc.get_quorum_certificate(tx))
                    .transpose()
            })
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .unwrap_or_else(QuorumCertificate::genesis);

        Ok(Response::new(GetHighQcResponse {
            high_qc: Some((&high_qc).into()),
        }))
    }

    async fn get_checkpoint(
        &self,
        _request: Request<GetCheckpointRequest>,
    ) -> Result<Response<GetCheckpointResponse>, RpcStatus> {
        let prev_epoch = self
            .epoch_manager
            .current_epoch()
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .saturating_sub(Epoch(1));
        if prev_epoch.is_zero() {
            return Err(RpcStatus::not_found("Cannot generate checkpoint for genesis epoch"));
        }

        if !self
            .epoch_manager
            .is_this_validator_registered_for_epoch(prev_epoch)
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
        {
            return Err(RpcStatus::bad_request(format!(
                "This validator node is not registered for the previous epoch {prev_epoch}"
            )));
        }

        let checkpoint = self
            .shard_state_store
            .with_read_tx(|tx| EpochCheckpoint::generate(tx, prev_epoch))
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        Ok(Response::new(GetCheckpointResponse {
            checkpoint: Some(checkpoint.into()),
        }))
    }

    async fn sync_state(&self, request: Request<SyncStateRequest>) -> Result<Streaming<SyncStateResponse>, RpcStatus> {
        let req = request.into_message();

        let (sender, receiver) = mpsc::channel(10);

        let last_state_transition_for_chain =
            StateTransitionId::from_parts(Epoch(req.start_epoch), Shard::from(req.start_shard), req.start_seq);

        // TODO: validate that we can provide the required sync data
        let current_shard = Shard::from(req.current_shard);
        let current_epoch = Epoch(req.current_epoch);
        info!(target: LOG_TARGET, "🌍peer initiated sync with this node ({current_epoch}, {current_shard})");

        task::spawn(
            StateSyncTask::new(
                self.shard_state_store.clone(),
                sender,
                last_state_transition_for_chain,
                current_shard,
                current_epoch,
            )
            .run(),
        );

        Ok(Streaming::new(receiver))
    }
}
