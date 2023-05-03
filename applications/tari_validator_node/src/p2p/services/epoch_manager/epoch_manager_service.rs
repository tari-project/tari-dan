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
use tari_comms::NodeIdentity;
use tari_dan_app_utilities::{
    base_node_client::GrpcBaseNodeClient,
    epoch_manager::{EpochManagerEvent, EpochManagerRequest},
};
use tari_dan_core::{consensus_constants::ConsensusConstants, services::epoch_manager::EpochManagerError};
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::{global::SqliteGlobalDbAdapter, sqlite_shard_store_factory::SqliteShardStore};
use tari_shutdown::ShutdownSignal;
use tari_validator_node_rpc::client::TariCommsValidatorNodeClientFactory;
use tokio::{
    sync::{broadcast, mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::p2p::services::epoch_manager::base_layer_epoch_manager::BaseLayerEpochManager;

const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

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
        shard_store: SqliteShardStore,
        base_node_client: GrpcBaseNodeClient,
        consensus_constants: ConsensusConstants,
        node_identity: Arc<NodeIdentity>,
        validator_node_client_factory: TariCommsValidatorNodeClientFactory,
    ) -> JoinHandle<anyhow::Result<()>> {
        tokio::spawn(async move {
            let (tx, rx) = broadcast::channel(10);
            EpochManagerService {
                rx_request,
                inner: BaseLayerEpochManager::new(
                    global_db,
                    shard_store,
                    base_node_client,
                    consensus_constants,
                    tx.clone(),
                    node_identity,
                    validator_node_client_factory,
                ),
                events: (tx, rx),
            }
            .run(shutdown)
            .await?;
            Ok(())
        })
    }

    pub async fn run(&mut self, mut shutdown: ShutdownSignal) -> Result<(), EpochManagerError> {
        // first, load initial state
        self.inner.load_initial_state()?;

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
            EpochManagerRequest::CurrentBlockHeight { reply } => handle(reply, Ok(self.inner.current_block_height())),
            EpochManagerRequest::GetValidatorShardKey { epoch, addr, reply } => handle(
                reply,
                self.inner
                    .get_validator_shard_key(epoch, &addr)
                    .and_then(|x| x.ok_or(EpochManagerError::ValidatorNodeNotRegistered)),
            ),
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
            EpochManagerRequest::GetValidatorNodeBMT { epoch, reply } => {
                handle(reply, self.inner.get_validator_node_bmt(epoch))
            },
            EpochManagerRequest::GetValidatorNodeMerkleRoot { epoch, reply } => {
                handle(reply, self.inner.get_validator_node_merkle_root(epoch))
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
            EpochManagerRequest::RemainingRegistrationEpochs { reply } => {
                handle(reply, self.inner.remaining_registration_epochs().await)
            },
            EpochManagerRequest::GetBaseLayerConsensusConstants { reply } => {
                handle(reply, self.inner.get_base_layer_consensus_constants().await.cloned())
            },
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
