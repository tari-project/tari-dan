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

use std::sync::Arc;

use log::error;
use tari_comms::{types::CommsPublicKey, NodeIdentity};
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_core::{
    consensus_constants::ConsensusConstants,
    models::{BaseLayerMetadata, Committee},
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
    p2p::services::{
        epoch_manager::base_layer_epoch_manager::BaseLayerEpochManager,
        rpc_client::TariCommsValidatorNodeClientFactory,
    },
    ValidatorNodeConfig,
};

const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

pub struct EpochManagerService {
    rx_request: Receiver<EpochManagerRequest>,
    inner: BaseLayerEpochManager,
    events: (
        broadcast::Sender<EpochManagerEvent>,
        broadcast::Receiver<EpochManagerEvent>,
    ),
}

type Reply<T> = oneshot::Sender<Result<T, EpochManagerError>>;

#[derive(Debug)]
pub enum EpochManagerRequest {
    CurrentEpoch {
        reply: Reply<Epoch>,
    },
    UpdateEpoch {
        tip_info: BaseLayerMetadata,
        reply: Reply<()>,
    },
    LastRegistrationEpoch {
        reply: Reply<Option<Epoch>>,
    },
    UpdateLastRegistrationEpoch {
        epoch: Epoch,
        reply: Reply<()>,
    },
    IsEpochValid {
        epoch: Epoch,
        reply: Reply<bool>,
    },
    GetCommittees {
        epoch: Epoch,
        shards: Vec<ShardId>,
        reply: Reply<Vec<ShardCommitteeAllocation<CommsPublicKey>>>,
    },
    GetCommittee {
        epoch: Epoch,
        shard: ShardId,
        reply: Reply<Committee<CommsPublicKey>>,
    },
    IsValidatorInCommitteeForCurrentEpoch {
        shard: ShardId,
        identity: CommsPublicKey,
        reply: Reply<bool>,
    },
    FilterToLocalShards {
        epoch: Epoch,
        for_addr: CommsPublicKey,
        available_shards: Vec<ShardId>,
        reply: Reply<Vec<ShardId>>,
    },
    Subscribe {
        reply: Reply<broadcast::Receiver<EpochManagerEvent>>,
    },
}

#[derive(Debug, Clone)]
pub enum EpochManagerEvent {
    EpochChanged(Epoch),
}

impl EpochManagerService {
    pub fn spawn(
        id: CommsPublicKey,
        rx_request: Receiver<EpochManagerRequest>,
        shutdown: ShutdownSignal,
        db_factory: SqliteDbFactory,
        base_node_client: GrpcBaseNodeClient,
        consensus_constants: ConsensusConstants,
        node_identity: Arc<NodeIdentity>,
        validator_node_config: ValidatorNodeConfig,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    ) -> JoinHandle<Result<(), EpochManagerError>> {
        tokio::spawn(async move {
            let (tx, rx) = broadcast::channel(10);
            EpochManagerService {
                rx_request,
                inner: BaseLayerEpochManager::new(
                    db_factory,
                    base_node_client,
                    consensus_constants,
                    id,
                    tx.clone(),
                    node_identity,
                    validator_node_config,
                    validator_node_client_factory,
                ),
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
                Some(req) = self.rx_request.recv() => self.handle_request(req).await,
                _ = shutdown.wait() => {
                    dbg!("Shutting down epoch manager");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, req: EpochManagerRequest) {
        match req {
            EpochManagerRequest::CurrentEpoch { reply } => handle(reply, Ok(self.inner.current_epoch())),
            EpochManagerRequest::UpdateEpoch { tip_info, reply } => {
                handle(reply, self.inner.update_epoch(tip_info).await);
            },
            EpochManagerRequest::LastRegistrationEpoch { reply } => {
                handle(reply, self.inner.last_registration_epoch().await)
            },

            EpochManagerRequest::UpdateLastRegistrationEpoch { epoch, reply } => {
                handle(reply, self.inner.update_last_registration_epoch(epoch).await);
            },
            EpochManagerRequest::IsEpochValid { epoch, reply } => handle(reply, Ok(self.inner.is_epoch_valid(epoch))),
            EpochManagerRequest::GetCommittees { epoch, shards, reply } => {
                handle(reply, self.inner.get_committees(epoch, &shards));
            },
            EpochManagerRequest::GetCommittee { epoch, shard, reply } => {
                handle(reply, self.inner.get_committee(epoch, shard));
            },
            EpochManagerRequest::IsValidatorInCommitteeForCurrentEpoch { shard, identity, reply } => {
                let epoch = self.inner.current_epoch();
                handle(reply, self.inner.is_validator_in_committee(epoch, shard, identity));
            },
            EpochManagerRequest::FilterToLocalShards {
                epoch,
                for_addr,
                available_shards,
                reply,
            } => {
                handle(
                    reply,
                    self.inner.filter_to_local_shards(epoch, &for_addr, &available_shards),
                );
            },
            EpochManagerRequest::Subscribe { reply } => handle(reply, Ok(self.events.1.resubscribe())),
        }
    }
}

fn handle<T>(reply: oneshot::Sender<Result<T, EpochManagerError>>, result: Result<T, EpochManagerError>) {
    if let Err(ref e) = result {
        error!(target: LOG_TARGET, "Request failed with error: {}", e);
    }
    if reply.send(result).is_err() {
        error!(target: LOG_TARGET, "Requester abandoned request");
    }
}
