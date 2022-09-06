use async_trait::async_trait;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{Committee, Epoch},
    services::epoch_manager::EpochManager,
};
use tokio::sync::{mpsc::Sender, oneshot, oneshot::channel};

use crate::p2p::services::epoch_manager::epoch_manager_service::{EpochManagerRequest, EpochManagerResponse};

#[derive(Clone)]
pub struct EpochManagerHandle {
    tx_request: Sender<(
        EpochManagerRequest,
        oneshot::Sender<Result<EpochManagerResponse, String>>,
    )>,
}

impl EpochManagerHandle {
    pub fn new(
        tx_request: Sender<(
            EpochManagerRequest,
            oneshot::Sender<Result<EpochManagerResponse, String>>,
        )>,
    ) -> Self {
        Self { tx_request }
    }
}
#[async_trait]
impl EpochManager<CommsPublicKey> for EpochManagerHandle {
    async fn current_epoch(&self) -> Epoch {
        let (tx, mut rx) = channel();
        let _ = self.tx_request.send((EpochManagerRequest::CurrentEpoch, tx)).await;
        let res = rx.await.expect("Error receiving");
        match res {
            Ok(EpochManagerResponse::CurrentEpoch { epoch }) => epoch,
            Err(e) => {
                panic!("erro: {}", e)
            },
            _ => {
                panic!("Wrong output type")
            },
        }
    }

    async fn is_epoch_valid(&self, epoch: Epoch) -> bool {
        todo!()
    }

    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<CommsPublicKey>>)>, String> {
        todo!()
    }

    async fn get_committee(&self, epoch: Epoch, shard: ShardId) -> Result<Committee<CommsPublicKey>, String> {
        todo!()
    }

    async fn get_shards(
        &self,
        epoch: Epoch,
        addr: &CommsPublicKey,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String> {
        todo!()
    }
}
