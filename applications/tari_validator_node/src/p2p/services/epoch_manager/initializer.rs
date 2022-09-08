use async_trait::async_trait;
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
impl ServiceInitializer for EpochManagerInitializer {
    async fn initialize(&mut self, context: ServiceInitializerContext) -> Result<(), ServiceInitializationError> {
        let (tx_request, rx_request) = channel(10);
        let handle = EpochManagerHandle::new(tx_request);
        context.register_handle(handle);
        let shutdown = context.get_shutdown_signal();
        let base_node_client = self.base_node_client.clone();
        context.spawn_when_ready(|handles| EpochManagerService::spawn(rx_request, shutdown, base_node_client));

        Ok(())
    }
}
