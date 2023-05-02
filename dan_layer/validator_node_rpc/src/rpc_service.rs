//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_comms::protocol::rpc::{Request, Response, RpcStatus, Streaming};
use tari_comms_rpc_macros::tari_rpc;

use crate::proto;

#[tari_rpc(protocol_name = b"t/vn/1", server_struct = ValidatorNodeRpcServer, client_struct = ValidatorNodeRpcClient)]
pub trait ValidatorNodeRpcService: Send + Sync + 'static {
    #[rpc(method = 1)]
    async fn submit_transaction(
        &self,
        request: Request<proto::rpc::SubmitTransactionRequest>,
    ) -> Result<Response<proto::rpc::SubmitTransactionResponse>, RpcStatus>;

    #[rpc(method = 2)]
    async fn get_peers(
        &self,
        request: Request<proto::rpc::GetPeersRequest>,
    ) -> Result<Streaming<proto::rpc::GetPeersResponse>, RpcStatus>;

    #[rpc(method = 3)]
    async fn vn_state_sync(
        &self,
        request: Request<proto::rpc::VnStateSyncRequest>,
    ) -> Result<Streaming<proto::rpc::VnStateSyncResponse>, RpcStatus>;
}
