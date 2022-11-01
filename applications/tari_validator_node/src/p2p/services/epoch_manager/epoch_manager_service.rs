//  Copyright 2022. The Tari Project
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

use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_core::{
    models::Committee,
    services::epoch_manager::{EpochManagerError, ShardCommitteeAllocation},
};
use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{broadcast, mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::{
    grpc::services::base_node_client::GrpcBaseNodeClient,
    p2p::services::epoch_manager::base_layer_epoch_manager::BaseLayerEpochManager,
};
// const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

pub struct EpochManagerService {
    rx_request: Receiver<(
        EpochManagerRequest,
        oneshot::Sender<Result<EpochManagerResponse, EpochManagerError>>,
    )>,
    inner: BaseLayerEpochManager,
    events: (
        broadcast::Sender<EpochManagerEvent>,
        broadcast::Receiver<EpochManagerEvent>,
    ),
}

#[derive(Debug, Clone)]
pub enum EpochManagerRequest {
    CurrentEpoch,
    UpdateEpoch {
        height: u64,
    },
    LastRegistrationEpoch,
    UpdateLastRegistrationEpoch {
        epoch: Epoch,
    },
    IsEpochValid {
        epoch: Epoch,
    },
    GetCommittees {
        epoch: Epoch,
        shards: Vec<ShardId>,
    },
    GetCommittee {
        epoch: Epoch,
        shard: ShardId,
    },
    FilterToLocalShards {
        epoch: Epoch,
        for_addr: CommsPublicKey,
        available_shards: Vec<ShardId>,
    },
    Subscribe,
}

pub enum EpochManagerResponse {
    CurrentEpoch {
        epoch: Epoch,
    },
    UpdateEpoch,
    LastRegistrationEpoch {
        epoch: Option<Epoch>,
    },
    UpdateLastRegistrationEpoch,
    IsEpochValid {
        is_valid: bool,
    },
    GetCommittees {
        committees: Vec<ShardCommitteeAllocation<CommsPublicKey>>,
    },
    GetCommittee {
        committee: Committee<CommsPublicKey>,
    },
    FilterToLocalShards {
        shards: Vec<ShardId>,
    },
    Subscribe {
        rx: broadcast::Receiver<EpochManagerEvent>,
    },
}

#[derive(Debug, Clone)]
pub enum EpochManagerEvent {
    EpochChanged(Epoch),
}

impl EpochManagerService {
    pub fn spawn(
        id: CommsPublicKey,
        rx_request: Receiver<(
            EpochManagerRequest,
            oneshot::Sender<Result<EpochManagerResponse, EpochManagerError>>,
        )>,
        shutdown: ShutdownSignal,
        db_factory: SqliteDbFactory,
        base_node_client: GrpcBaseNodeClient,
    ) -> JoinHandle<Result<(), EpochManagerError>> {
        tokio::spawn(async move {
            let (tx, rx) = broadcast::channel(10);
            EpochManagerService {
                rx_request,
                inner: BaseLayerEpochManager::new(db_factory, base_node_client, id, tx.clone()),
                events: (tx, rx),
            }
            .run(shutdown)
            .await
        })
    }

    pub async fn run(&mut self, mut shutdown: ShutdownSignal) -> Result<(), EpochManagerError> {
        // first, load initial state
        self.inner.load_initial_state().await?;

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

    async fn handle_request(&mut self, req: EpochManagerRequest) -> Result<EpochManagerResponse, EpochManagerError> {
        match req {
            EpochManagerRequest::CurrentEpoch => Ok(EpochManagerResponse::CurrentEpoch {
                epoch: self.inner.current_epoch(),
            }),
            EpochManagerRequest::UpdateEpoch { height } => {
                self.inner.update_epoch(height).await?;
                Ok(EpochManagerResponse::UpdateEpoch)
            },
            EpochManagerRequest::LastRegistrationEpoch => Ok(EpochManagerResponse::LastRegistrationEpoch {
                epoch: self.inner.last_registration_epoch().await?,
            }),
            EpochManagerRequest::UpdateLastRegistrationEpoch { epoch } => {
                self.inner.update_last_registration_epoch(epoch).await?;
                Ok(EpochManagerResponse::UpdateLastRegistrationEpoch)
            },
            EpochManagerRequest::IsEpochValid { epoch } => {
                let is_valid = self.inner.is_epoch_valid(epoch);
                Ok(EpochManagerResponse::IsEpochValid { is_valid })
            },
            EpochManagerRequest::GetCommittees { epoch, shards } => {
                let committees = self.inner.get_committees(epoch, &shards)?;
                Ok(EpochManagerResponse::GetCommittees { committees })
            },
            EpochManagerRequest::GetCommittee { epoch, shard } => {
                let committee = self.inner.get_committee(epoch, shard)?;
                Ok(EpochManagerResponse::GetCommittee { committee })
            },
            EpochManagerRequest::FilterToLocalShards {
                epoch,
                for_addr,
                available_shards,
            } => {
                let shards = self.inner.filter_to_local_shards(epoch, &for_addr, &available_shards)?;
                Ok(EpochManagerResponse::FilterToLocalShards { shards })
            },
            EpochManagerRequest::Subscribe => Ok(EpochManagerResponse::Subscribe {
                rx: self.events.1.resubscribe(),
            }),
        }
    }
}
