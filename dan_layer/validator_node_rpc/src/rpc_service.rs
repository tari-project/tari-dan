//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_comms::protocol::rpc::{Request, Response, RpcStatus, Streaming};
use tari_comms_rpc_macros::tari_rpc;

use crate::proto::rpc as proto;

#[tari_rpc(protocol_name = b"t/vn/1", server_struct = ValidatorNodeRpcServer, client_struct = ValidatorNodeRpcClient)]
pub trait ValidatorNodeRpcService: Send + Sync + 'static {
    #[rpc(method = 1)]
    async fn submit_transaction(
        &self,
        request: Request<proto::SubmitTransactionRequest>,
    ) -> Result<Response<proto::SubmitTransactionResponse>, RpcStatus>;

    #[rpc(method = 2)]
    async fn get_peers(
        &self,
        request: Request<proto::GetPeersRequest>,
    ) -> Result<Streaming<proto::GetPeersResponse>, RpcStatus>;

    // #[rpc(method = 3)]
    // async fn vn_state_sync(
    //     &self,
    //     request: Request<proto::VnStateSyncRequest>,
    // ) -> Result<Streaming<proto::VnStateSyncResponse>, RpcStatus>;

    #[rpc(method = 4)]
    async fn get_substate(
        &self,
        req: Request<proto::GetSubstateRequest>,
    ) -> Result<Response<proto::GetSubstateResponse>, RpcStatus>;

    #[rpc(method = 5)]
    async fn get_transaction_result(
        &self,
        req: Request<proto::GetTransactionResultRequest>,
    ) -> Result<Response<proto::GetTransactionResultResponse>, RpcStatus>;

    #[rpc(method = 6)]
    async fn get_virtual_substate(
        &self,
        req: Request<proto::GetVirtualSubstateRequest>,
    ) -> Result<Response<proto::GetVirtualSubstateResponse>, RpcStatus>;

    #[rpc(method = 7)]
    async fn sync_blocks(
        &self,
        request: Request<proto::SyncBlocksRequest>,
    ) -> Result<Streaming<proto::SyncBlocksResponse>, RpcStatus>;
    #[rpc(method = 8)]
    async fn get_high_qc(
        &self,
        request: Request<proto::GetHighQcRequest>,
    ) -> Result<Response<proto::GetHighQcResponse>, RpcStatus>;
}
