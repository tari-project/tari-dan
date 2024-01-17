//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_p2p::proto::rpc as proto;
use tari_rpc_framework::{Request, Response, RpcStatus, Streaming};
use tari_rpc_macros::tari_rpc;

#[tari_rpc(protocol_name = "/tari/validator/1.0.0", server_struct = ValidatorNodeRpcServer, client_struct = ValidatorNodeRpcClient)]
pub trait ValidatorNodeRpcService: Send + Sync + 'static {
    #[rpc(method = 1)]
    async fn submit_transaction(
        &self,
        request: Request<proto::SubmitTransactionRequest>,
    ) -> Result<Response<proto::SubmitTransactionResponse>, RpcStatus>;

    #[rpc(method = 2)]
    async fn get_substate(
        &self,
        req: Request<proto::GetSubstateRequest>,
    ) -> Result<Response<proto::GetSubstateResponse>, RpcStatus>;

    #[rpc(method = 3)]
    async fn get_transaction_result(
        &self,
        req: Request<proto::GetTransactionResultRequest>,
    ) -> Result<Response<proto::GetTransactionResultResponse>, RpcStatus>;

    #[rpc(method = 4)]
    async fn get_virtual_substate(
        &self,
        req: Request<proto::GetVirtualSubstateRequest>,
    ) -> Result<Response<proto::GetVirtualSubstateResponse>, RpcStatus>;

    #[rpc(method = 5)]
    async fn sync_blocks(
        &self,
        request: Request<proto::SyncBlocksRequest>,
    ) -> Result<Streaming<proto::SyncBlocksResponse>, RpcStatus>;

    #[rpc(method = 6)]
    async fn get_high_qc(
        &self,
        request: Request<proto::GetHighQcRequest>,
    ) -> Result<Response<proto::GetHighQcResponse>, RpcStatus>;
}
