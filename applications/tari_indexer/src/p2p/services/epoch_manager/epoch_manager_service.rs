//  Copyright 2023. The Tari Project
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

use log::error;
use tari_dan_app_utilities::{
    base_node_client::GrpcBaseNodeClient,
    epoch_manager::{EpochManagerEvent, EpochManagerRequest},
};
use tari_dan_core::{consensus_constants::ConsensusConstants, services::epoch_manager::EpochManagerError};
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{broadcast, mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::p2p::services::{
    epoch_manager::base_layer_epoch_manager::BaseLayerEpochManager,
    rpc_client::TariCommsValidatorNodeClientFactory,
};

const LOG_TARGET: &str = "tari::indexer::epoch_manager";

pub struct EpochManagerService {
    rx_request: Receiver<EpochManagerRequest>,
    inner: BaseLayerEpochManager,
    events: (
        broadcast::Sender<EpochManagerEvent>,
        broadcast::Receiver<EpochManagerEvent>,
    ),
}

impl EpochManagerService {
    pub fn spawn(
        rx_request: Receiver<EpochManagerRequest>,
        shutdown: ShutdownSignal,
        global_db: GlobalDb<SqliteGlobalDbAdapter>,
        base_node_client: GrpcBaseNodeClient,
        consensus_constants: ConsensusConstants,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let (tx, rx) = broadcast::channel(10);
            let result = EpochManagerService {
                rx_request,
                inner: BaseLayerEpochManager::new(
                    global_db,
                    base_node_client,
                    consensus_constants,
                    validator_node_client_factory,
                ),
                events: (tx, rx),
            }
            .run(shutdown)
            .await;

            if let Err(err) = result {
                error!(target: LOG_TARGET, "Epoch manager service failed with error: {}", err);
            }
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
            EpochManagerRequest::UpdateEpoch {
                block_height,
                block_hash,
                reply,
            } => {
                handle(reply, self.inner.update_epoch(block_height, block_hash).await);
            },
            EpochManagerRequest::LastRegistrationEpoch { reply } => handle(reply, self.inner.last_registration_epoch()),

            EpochManagerRequest::UpdateLastRegistrationEpoch { epoch, reply } => {
                handle(reply, self.inner.update_last_registration_epoch(epoch));
            },
            EpochManagerRequest::IsEpochValid { epoch, reply } => handle(reply, Ok(self.inner.is_epoch_valid(epoch))),
            EpochManagerRequest::GetCommittees { epoch, shards, reply } => {
                handle(reply, self.inner.get_committees(epoch, &shards));
            },
            EpochManagerRequest::GetCommittee { epoch, shard, reply } => {
                handle(reply, self.inner.get_committee(epoch, shard));
            },
            EpochManagerRequest::GetValidatorNodesPerEpoch { epoch, reply } => {
                handle(reply, self.inner.get_validator_nodes_per_epoch(epoch))
            },
            EpochManagerRequest::AddValidatorNodeRegistration {
                block_height,
                registration,
                reply,
            } => handle(
                reply,
                self.inner
                    .add_validator_node_registration(block_height, registration)
                    .await,
            ),
            // TODO: This should be rather be a state machine event
            EpochManagerRequest::NotifyScanningComplete { reply } => {
                handle(reply, self.inner.on_scanning_complete().await)
            },
            EpochManagerRequest::GetValidatorShardKey { .. } => todo!(),
            EpochManagerRequest::GetValidatorNodeMmr { .. } => todo!(),
            EpochManagerRequest::GetValidatorNodeMerkleRoot { .. } => todo!(),
            EpochManagerRequest::IsValidatorInCommitteeForCurrentEpoch { .. } => todo!(),
            EpochManagerRequest::FilterToLocalShards { .. } => todo!(),
            EpochManagerRequest::Subscribe { .. } => todo!(),
            EpochManagerRequest::RemainingRegistrationEpochs { .. } => todo!(),
            EpochManagerRequest::GetBaseLayerConsensusConstants { .. } => todo!(),
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
