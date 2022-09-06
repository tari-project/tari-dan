use async_trait::async_trait;
use tari_service_framework::{ServiceInitializationError, ServiceInitializer, ServiceInitializerContext};
use tokio::sync::mpsc::channel;

use crate::p2p::services::epoch_manager::{epoch_manager_service::EpochManagerService, handle::EpochManagerHandle};

pub struct EpochManagerInitializer {}

#[async_trait]
impl ServiceInitializer for EpochManagerInitializer {
    async fn initialize(&mut self, context: ServiceInitializerContext) -> Result<(), ServiceInitializationError> {
        let (tx_request, rx_request) = channel(10);
        let handle = EpochManagerHandle::new(tx_request);
        context.register_handle(handle);
        let shutdown = context.get_shutdown_signal();
        context.spawn_when_ready(|_handles| EpochManagerService::spawn(rx_request, shutdown));

        Ok(())
    }
}
