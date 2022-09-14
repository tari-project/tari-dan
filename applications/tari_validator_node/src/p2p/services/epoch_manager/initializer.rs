//  Copyright 2021. The Tari Project
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

use async_trait::async_trait;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_core::{chain_storage::UtxoMinedInfo, transactions::transaction_components::OutputType};
use tari_service_framework::{ServiceInitializationError, ServiceInitializer, ServiceInitializerContext};
use tokio::sync::mpsc::channel;

use crate::{
    grpc::services::base_node_client::GrpcBaseNodeClient,
    p2p::services::epoch_manager::{epoch_manager_service::EpochManagerService, handle::EpochManagerHandle},
};

pub struct EpochManagerInitializer {
    pub base_node_client: GrpcBaseNodeClient,
}
#[async_trait]
pub trait WalletClient: Send + Sync {}

#[async_trait]
impl ServiceInitializer for EpochManagerInitializer {
    async fn initialize(&mut self, context: ServiceInitializerContext) -> Result<(), ServiceInitializationError> {
        let (tx_request, rx_request) = channel(10);
        let handle = EpochManagerHandle::new(tx_request);
        context.register_handle(handle);
        let shutdown = context.get_shutdown_signal();
        let base_node_client = self.base_node_client.clone();
        context.spawn_when_ready(|_handles| EpochManagerService::spawn(rx_request, shutdown, base_node_client));

        Ok(())
    }
}
