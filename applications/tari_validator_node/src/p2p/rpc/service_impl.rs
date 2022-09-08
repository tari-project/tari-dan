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
use tari_comms::{
    protocol::rpc::{Request, Response, RpcStatus, Streaming},
    utils,
};
use tari_dan_core::{services::mempool::service::MempoolService, storage::DbFactory};
use tari_dan_engine::{instruction::Transaction, state::StateDbUnitOfWorkReader};
use tokio::{sync::mpsc, task};

const LOG_TARGET: &str = "vn::p2p::rpc";

use crate::p2p::{proto::validator_node as proto, rpc::ValidatorNodeRpcService};

pub struct ValidatorNodeRpcServiceImpl<TMempoolService, TDbFactory: DbFactory> {
    mempool_service: TMempoolService,
    db_factory: TDbFactory,
}

impl<TMempoolService: MempoolService + Clone, TDbFactory: DbFactory + Clone>
    ValidatorNodeRpcServiceImpl<TMempoolService, TDbFactory>
{
    pub fn new(mempool_service: TMempoolService, db_factory: TDbFactory) -> Self {
        Self {
            mempool_service,
            db_factory,
        }
    }
}

#[tari_comms::async_trait]
impl<TMempoolService, TDbFactory> ValidatorNodeRpcService for ValidatorNodeRpcServiceImpl<TMempoolService, TDbFactory>
where
    TMempoolService: MempoolService + Clone,
    TDbFactory: DbFactory + Clone,
{
    async fn get_token_data(
        &self,
        _request: Request<proto::GetTokenDataRequest>,
    ) -> Result<Response<proto::GetTokenDataResponse>, RpcStatus> {
        Err(RpcStatus::general("Not implemented"))
    }

    async fn submit_transaction(
        &self,
        request: Request<proto::SubmitTransactionRequest>,
    ) -> Result<Response<proto::SubmitTransactionResponse>, RpcStatus> {
        println!("{:?}", request);
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

        let mut mempool_service = self.mempool_service.clone();
        match mempool_service.submit_transaction(&transaction).await {
            Ok(_) => {
                debug!(target: LOG_TARGET, "Accepted instruction into mempool");
                return Ok(Response::new(proto::SubmitTransactionResponse {
                    result: vec![],
                    status: "Accepted".to_string(),
                }));
            },
            Err(err) => {
                debug!(target: LOG_TARGET, "Mempool rejected instruction: {}", err);
                return Ok(Response::new(proto::SubmitTransactionResponse {
                    result: vec![],
                    status: format!("Errored: {}", err),
                }));
            },
        }
    }

    async fn get_sidechain_state(
        &self,
        request: Request<proto::GetSidechainStateRequest>,
    ) -> Result<Streaming<proto::GetSidechainStateResponse>, RpcStatus> {
        let msg = request.into_message();

        let contract_id = msg
            .contract_id
            .try_into()
            .map_err(|_| RpcStatus::bad_request("Invalid contract_id"))?;

        let db = self
            .db_factory
            .get_state_db(&contract_id)
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .ok_or_else(|| RpcStatus::not_found("Asset not found"))?;

        let uow = db.reader();
        let data = uow.get_all_state().map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let (tx, rx) = mpsc::channel(10);

        task::spawn(async move {
            for state in data {
                let schema = proto::GetSidechainStateResponse {
                    state: Some(proto::get_sidechain_state_response::State::Schema(state.name)),
                };

                if tx.send(Ok(schema)).await.is_err() {
                    return;
                }

                let key_values = state
                    .items
                    .into_iter()
                    .map(|kv| proto::get_sidechain_state_response::State::KeyValue(kv.into()))
                    .map(|state| Ok(proto::GetSidechainStateResponse { state: Some(state) }));

                if utils::mpsc::send_all(&tx, key_values).await.is_err() {
                    return;
                }
            }
        });

        Ok(Streaming::new(rx))
    }

    async fn get_op_logs(
        &self,
        request: Request<proto::GetStateOpLogsRequest>,
    ) -> Result<Response<proto::GetStateOpLogsResponse>, RpcStatus> {
        let msg = request.into_message();

        let contract_id = msg
            .contract_id
            .try_into()
            .map_err(|_| RpcStatus::bad_request("Invalid contract_id"))?;

        let db = self
            .db_factory
            .get_state_db(&contract_id)
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .ok_or_else(|| RpcStatus::not_found("Asset not found"))?;

        let reader = db.reader();
        let op_logs = reader
            .get_op_logs_for_height(msg.height)
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let resp = proto::GetStateOpLogsResponse {
            op_logs: op_logs.into_iter().map(Into::into).collect(),
        };

        Ok(Response::new(resp))
    }

    async fn get_tip_node(
        &self,
        _request: Request<proto::GetTipNodeRequest>,
    ) -> Result<Response<proto::GetTipNodeResponse>, RpcStatus> {
        // let msg = request.into_message();
        //
        // let contract_id = msg
        //     .contract_id
        //     .try_into()
        //     .map_err(|_| RpcStatus::bad_request("Invalid contract_id"))?;
        //
        // let db = self
        //     .db_factory
        //     .get_chain_db(&contract_id)
        //     .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
        //     .ok_or_else(|| RpcStatus::not_found("Asset not found"))?;
        //
        // let tip_node = db.get_tip_node().map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        //
        // let resp = proto::GetTipNodeResponse {
        //     tip_node: tip_node.map(Into::into),
        // };
        //
        // Ok(Response::new(resp))
        todo!()
    }
}
