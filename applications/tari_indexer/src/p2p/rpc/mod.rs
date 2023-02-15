//  Copyright 2023, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

#![allow(dead_code)]

mod service_impl;

pub use service_impl::ValidatorNodeRpcServiceImpl;
use tari_comms::protocol::rpc::{Request, Response, RpcStatus, Streaming};
use tari_comms_rpc_macros::tari_rpc;
use tari_dan_app_grpc::proto;
use tari_dan_core::services::PeerProvider;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;

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

pub fn create_validator_node_rpc_service<TPeerProvider>(
    peer_provider: TPeerProvider,
    shard_store_store: SqliteShardStore,
) -> ValidatorNodeRpcServer<ValidatorNodeRpcServiceImpl<TPeerProvider>>
where
    TPeerProvider: PeerProvider + Clone + Send + Sync + 'static,
{
    ValidatorNodeRpcServer::new(ValidatorNodeRpcServiceImpl::new(peer_provider, shard_store_store))
}
