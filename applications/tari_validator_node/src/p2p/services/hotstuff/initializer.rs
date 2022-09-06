use std::sync::Arc;

use async_trait::async_trait;
use tari_comms::NodeIdentity;
use tari_dan_core::services::mempool::service::MempoolServiceHandle;
use tari_service_framework::{ServiceInitializationError, ServiceInitializer, ServiceInitializerContext};

use crate::p2p::services::{epoch_manager::handle::EpochManagerHandle, hotstuff::hotstuff_service::HotstuffService};

pub struct HotstuffServiceInitializer {
    pub node_identity: Arc<NodeIdentity>,
}

#[async_trait]
impl ServiceInitializer for HotstuffServiceInitializer {
    async fn initialize(&mut self, context: ServiceInitializerContext) -> Result<(), ServiceInitializationError> {
        // let mut mempool_service = self.mempool.clone();
        // let mut mempool_inbound = TariCommsMempoolInboundHandle::new(
        //     self.inbound_message_subscription_factory.clone(),
        //     mempool_service.clone(),
        // );
        // context.register_handle(mempool_inbound.clone());
        //
        // context.spawn_until_shutdown(move |handles| async move {
        //     let dht = handles.expect_handle::<Dht>();
        //     let outbound_requester = dht.outbound_requester();
        //     let mempool_outbound = TariCommsMempoolOutboundService::new(outbound_requester);
        //     mempool_service.set_outbound_service(Box::new(mempool_outbound)).await;
        //
        //     mempool_inbound.run().await;
        // });

        let shutdown = context.get_shutdown_signal();
        let node_identity = self.node_identity.as_ref().clone();
        context.spawn_when_ready(|handles| async move {
            let epoch_manager = handles.expect_handle::<EpochManagerHandle>();
            let mempool = handles.expect_handle::<MempoolServiceHandle>();
            HotstuffService::spawn(node_identity.public_key().clone(), epoch_manager, mempool, shutdown).await
        });
        Ok(())
    }
}
