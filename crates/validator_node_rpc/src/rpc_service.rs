//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_p2p::proto::rpc as proto;
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

    // Method 4 is retired and must not be reused.

    #[rpc(method = 5)]
    async fn get_checkpoints(
        &self,
        request: Request<proto::GetCheckpointsRequest>,
    ) -> Result<Response<proto::GetCheckpointsResponse>, RpcStatus>;

    #[rpc(method = 6)]
    async fn sync_state(
        &self,
        request: Request<proto::SyncStateRequest>,
    ) -> Result<Streaming<proto::SyncStateResponse>, RpcStatus>;

    #[rpc(method = 7)]
    async fn get_substate_batch(
        &self,
        req: Request<proto::GetSubstatesBatchRequest>,
    ) -> Result<Streaming<proto::GetSubstatesBatchResponse>, RpcStatus>;

    #[rpc(method = 8)]
    async fn get_consensus_state(
        &self,
        req: Request<proto::GetConsensusStateRequest>,
    ) -> Result<Response<proto::GetConsensusStateResponse>, RpcStatus>;

    #[rpc(method = 9)]
    async fn get_high_qc(
        &self,
        req: Request<proto::GetHighQcRequest>,
    ) -> Result<Response<proto::GetHighQcResponse>, RpcStatus>;

    #[rpc(method = 10)]
    async fn get_committed_block_proof(
        &self,
        req: Request<proto::GetCommittedBlockProofRequest>,
    ) -> Result<Response<proto::GetCommittedBlockProofResponse>, RpcStatus>;
}
