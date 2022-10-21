//  Copyright 2021, The Tari Project
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

mod service_impl;

pub use service_impl::ValidatorNodeRpcServiceImpl;
use tari_comms::protocol::rpc::{Request, Response, RpcStatus, Streaming};
use tari_comms_rpc_macros::tari_rpc;
use tari_dan_core::services::PeerProvider;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStoreFactory;

use crate::p2p::{proto, services::messaging::DanMessageSenders};

#[tari_rpc(protocol_name = b"t/vn/1", server_struct = ValidatorNodeRpcServer, client_struct = ValidatorNodeRpcClient)]
pub trait ValidatorNodeRpcService: Send + Sync + 'static {
    #[rpc(method = 1)]
    async fn submit_transaction(
        &self,
        request: Request<proto::validator_node::SubmitTransactionRequest>,
    ) -> Result<Response<proto::validator_node::SubmitTransactionResponse>, RpcStatus>;

    #[rpc(method = 2)]
    async fn get_peers(
        &self,
        request: Request<proto::network::GetPeersRequest>,
    ) -> Result<Streaming<proto::network::GetPeersResponse>, RpcStatus>;

    #[rpc(method = 3)]
    async fn get_vn_state_inventory(
        &self,
        request: Request<proto::network::GetVnStateInventoryRequest>,
    ) -> Result<Response<proto::network::GetVnStateInventoryResponse>, RpcStatus>;

    #[rpc(method = 4)]
    async fn vn_state_sync(
        &self,
        request: Request<proto::network::VnStateSyncRequest>,
    ) -> Result<Streaming<proto::network::VnStateSyncResponse>, RpcStatus>;
}

pub fn create_validator_node_rpc_service<TPeerProvider>(
    message_senders: DanMessageSenders,
    peer_provider: TPeerProvider,
    shard_store_store: SqliteShardStoreFactory,
) -> ValidatorNodeRpcServer<ValidatorNodeRpcServiceImpl<TPeerProvider>>
where
    TPeerProvider: PeerProvider + Clone + Send + Sync + 'static,
{
    ValidatorNodeRpcServer::new(ValidatorNodeRpcServiceImpl::new(
        message_senders,
        peer_provider,
        shard_store_store,
    ))
}
