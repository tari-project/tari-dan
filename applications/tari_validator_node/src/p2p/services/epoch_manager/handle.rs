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
    models::{Committee, Epoch},
    services::epoch_manager::{EpochManager, EpochManagerError, ShardCommitteeAllocation},
};
use tokio::sync::{mpsc::Sender, oneshot};

use crate::p2p::services::epoch_manager::epoch_manager_service::{EpochManagerRequest, EpochManagerResponse};

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

    pub async fn update_epoch(&self, height: u64) -> Result<(), EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send((EpochManagerRequest::UpdateEpoch { height }, tx))
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        let _result = rx.await.map_err(|_| EpochManagerError::ReceiveError)??;
        Ok(())
    }
}
#[async_trait]
impl EpochManager<CommsPublicKey> for EpochManagerHandle {
    async fn current_epoch(&self) -> Result<Epoch, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send((EpochManagerRequest::CurrentEpoch, tx))
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        match rx.await.map_err(|_| EpochManagerError::ReceiveError)?? {
            EpochManagerResponse::CurrentEpoch { epoch } => Ok(epoch),
            _ => Err(EpochManagerError::UnexpectedResponse),
        }
    }

    async fn is_epoch_valid(&self, epoch: Epoch) -> Result<bool, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send((EpochManagerRequest::IsEpochValid { epoch }, tx))
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        match rx.await.map_err(|_| EpochManagerError::ReceiveError)?? {
            EpochManagerResponse::IsEpochValid { is_valid } => Ok(is_valid),
            _ => Err(EpochManagerError::UnexpectedResponse),
        }
    }

    async fn get_committees(
        &self,
        epoch: Epoch,
        shards: &[ShardId],
    ) -> Result<Vec<ShardCommitteeAllocation<CommsPublicKey>>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send((
                EpochManagerRequest::GetCommittees {
                    epoch,
                    shards: shards.to_vec(),
                },
                tx,
            ))
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        match rx.await.map_err(|_| EpochManagerError::ReceiveError)?? {
            EpochManagerResponse::GetCommittees { committees } => Ok(committees),
            _ => Err(EpochManagerError::UnexpectedResponse),
        }
    }

    async fn get_committee(
        &self,
        epoch: Epoch,
        shard: ShardId,
    ) -> Result<Committee<CommsPublicKey>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send((EpochManagerRequest::GetCommittee { epoch, shard }, tx))
            .await
            .map_err(|_| EpochManagerError::SendError)?;
        match rx.await.map_err(|_| EpochManagerError::ReceiveError)?? {
            EpochManagerResponse::GetCommittee { committee } => Ok(committee),
            _ => Err(EpochManagerError::UnexpectedResponse),
        }
    }

    async fn filter_to_local_shards(
        &self,
        epoch: Epoch,
        for_addr: &CommsPublicKey,
        available_shards: &[ShardId],
    ) -> Result<Vec<ShardId>, EpochManagerError> {
        let (tx, rx) = oneshot::channel();
        self.tx_request
            .send((
                EpochManagerRequest::FilterToLocalShards {
                    epoch,
                    for_addr: for_addr.clone(),
                    available_shards: available_shards.to_vec(),
                },
                tx,
            ))
            .await
            .map_err(|_| EpochManagerError::SendError)?;

        match rx.await.map_err(|_| EpochManagerError::ReceiveError)?? {
            EpochManagerResponse::FilterToLocalShards { shards } => Ok(shards),
            _ => Err(EpochManagerError::UnexpectedResponse),
        }
    }
}
