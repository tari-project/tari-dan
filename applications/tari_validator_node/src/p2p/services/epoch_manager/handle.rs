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
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{BaseLayerMetadata, Committee, Epoch},
    services::epoch_manager::EpochManager,
};
use tokio::sync::{mpsc::Sender, oneshot, oneshot::channel};

use crate::p2p::services::epoch_manager::{
    epoch_manager_service::{EpochManagerRequest, EpochManagerResponse},
    EpochManagerError,
};

#[derive(Clone)]
pub struct EpochManagerHandle {
    tx_request: Sender<(
        EpochManagerRequest,
        oneshot::Sender<Result<EpochManagerResponse, EpochManagerError>>,
    )>,
}

impl EpochManagerHandle {
    pub fn new(
        tx_request: Sender<(
            EpochManagerRequest,
            oneshot::Sender<Result<EpochManagerResponse, EpochManagerError>>,
        )>,
    ) -> Self {
        Self { tx_request }
    }

    pub async fn update_epoch(&self, tip: BaseLayerMetadata) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send((EpochManagerRequest::UpdateEpoch { tip }, tx))
            .await
            .map_err(|e| EpochManagerError::SendError)?;
        Ok(())
    }
}
impl EpochManager<CommsPublicKey> for EpochManagerHandle {
    fn current_epoch(&self) -> Epoch {
        // let (tx, rx) = channel();
        // let _ignore = self.tx_request.send((EpochManagerRequest::CurrentEpoch, tx));
        // let res = rx.await.expect("Error receiving");
        // match res {
        //     Ok(EpochManagerResponse::CurrentEpoch { epoch }) => epoch,
        //     Err(e) => {
        //         panic!("erro: {}", e)
        //     },
        // }
        todo!()
    }

    fn is_epoch_valid(&self, _epoch: Epoch) -> bool {
        todo!()
    }

    fn get_committees(
        &self,
        _epoch: Epoch,
        _shards: &[ShardId],
    ) -> Result<Vec<(ShardId, Option<Committee<CommsPublicKey>>)>, String> {
        todo!()
    }

    fn get_committee(&self, _epoch: Epoch, _shard: ShardId) -> Result<Committee<CommsPublicKey>, String> {
        todo!()
    }

    fn get_shards(
        &self,
        _epoch: Epoch,
        _addr: &CommsPublicKey,
        _available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, String> {
        todo!()
    }
}
