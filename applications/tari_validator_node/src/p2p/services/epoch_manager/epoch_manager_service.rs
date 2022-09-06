use tari_dan_core::{models::Epoch, services::epoch_manager::EpochManager};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::p2p::services::epoch_manager::base_layer_epoch_manager::BaseLayerEpochManager;
// const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

pub struct EpochManagerService {
    rx_request: Receiver<(
        EpochManagerRequest,
        oneshot::Sender<Result<EpochManagerResponse, String>>,
    )>,
    inner: BaseLayerEpochManager,
}

#[derive(Debug, Clone)]
pub enum EpochManagerRequest {
    CurrentEpoch,
}

pub enum EpochManagerResponse {
    CurrentEpoch { epoch: Epoch },
}

impl EpochManagerService {
    pub fn spawn(
        rx_request: Receiver<(
            EpochManagerRequest,
            oneshot::Sender<Result<EpochManagerResponse, String>>,
        )>,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            EpochManagerService {
                rx_request,
                inner: BaseLayerEpochManager {},
            }
            .run(shutdown)
            .await
        })
    }

    pub async fn run(&mut self, mut shutdown: ShutdownSignal) -> Result<(), String> {
        loop {
            tokio::select! {
                Some((req, reply)) = self.rx_request.recv() => {
                    let _ignore = reply.send(self.handle_request(req).await);
                },
                _ = shutdown.wait() => {
                    dbg!("Shutting down epoch manager");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, req: EpochManagerRequest) -> Result<EpochManagerResponse, String> {
        match req {
            EpochManagerRequest::CurrentEpoch => Ok(EpochManagerResponse::CurrentEpoch {
                epoch: self.inner.current_epoch().await,
            }),
        }
    }
}
