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

use log::{error, info, trace};
use tari_base_node_client::grpc::GrpcBaseNodeClient;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{DerivableFromPublicKey, NodeAddressable};
use tari_dan_storage::global::GlobalDb;
use tari_dan_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{broadcast, mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::{
    base_layer::{
        base_layer_epoch_manager::BaseLayerEpochManager,
        config::EpochManagerConfig,
        types::EpochManagerRequest,
    },
    error::EpochManagerError,
    EpochManagerEvent,
};

const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

pub struct EpochManagerService<TAddr, TGlobalStore, TBaseNodeClient> {
    rx_request: Receiver<EpochManagerRequest<TAddr>>,
    inner: BaseLayerEpochManager<TGlobalStore, TBaseNodeClient>,
    events: broadcast::Sender<EpochManagerEvent>,
}

impl<TAddr: NodeAddressable + DerivableFromPublicKey + 'static>
    EpochManagerService<TAddr, SqliteGlobalDbAdapter<TAddr>, GrpcBaseNodeClient>
{
    pub fn spawn(
        config: EpochManagerConfig,
        rx_request: Receiver<EpochManagerRequest<TAddr>>,
        shutdown: ShutdownSignal,
        global_db: GlobalDb<SqliteGlobalDbAdapter<TAddr>>,
        base_node_client: GrpcBaseNodeClient,
        node_public_key: PublicKey,
    ) -> JoinHandle<anyhow::Result<()>> {
        tokio::spawn(async move {
            let (tx, _) = broadcast::channel(100);
            EpochManagerService {
                rx_request,
                inner: BaseLayerEpochManager::new(config, global_db, base_node_client, tx.clone(), node_public_key),
                events: tx,
            }
            .run(shutdown)
            .await?;
            Ok(())
        })
    }

    pub async fn run(&mut self, mut shutdown: ShutdownSignal) -> Result<(), EpochManagerError> {
        info!(target: LOG_TARGET, "Starting epoch manager");
        info!(target: LOG_TARGET, "Loading initial state");
        // first, load initial state
        self.inner.load_initial_state().await?;

        loop {
            tokio::select! {
                req = self.rx_request.recv() => {
                    match req {
                        Some(req) => self.handle_request(req).await,
                        None => {
                            error!(target: LOG_TARGET, "Channel closed. Shutting down epoch manager");
                            break;
                        }
                    }
                },
                _ = shutdown.wait() => {
                    dbg!("Shutting down epoch manager");
                    break;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_request(&mut self, req: EpochManagerRequest<TAddr>) {
        trace!(target: LOG_TARGET, "Received request: {:?}", req);
        let context = &format!("{:?}", req);
        match req {
            EpochManagerRequest::CurrentEpoch { reply } => handle(reply, Ok(self.inner.current_epoch()), context),
            EpochManagerRequest::CurrentBlockInfo { reply } => {
                handle(reply, Ok(self.inner.current_block_info()), context)
            },
            EpochManagerRequest::GetLastBlockOfTheEpoch { reply } => {
                handle(reply, Ok(self.inner.last_block_of_current_epoch()), context)
            },
            EpochManagerRequest::IsLastBlockOfTheEpoch { block_height, reply } => {
                handle(reply, self.inner.is_last_block_of_epoch(block_height).await, context)
            },
            EpochManagerRequest::GetValidatorNode { epoch, addr, reply } => handle(
                reply,
                self.inner.get_validator_node_by_address(epoch, &addr).and_then(|x| {
                    x.ok_or(EpochManagerError::ValidatorNodeNotRegistered {
                        address: addr.to_string(),
                        epoch,
                    })
                }),
                context,
            ),
            EpochManagerRequest::GetValidatorNodeByPublicKey {
                epoch,
                public_key,
                reply,
            } => handle(
                reply,
                self.inner
                    .get_validator_node_by_public_key(epoch, &public_key)
                    .and_then(|x| {
                        x.ok_or(EpochManagerError::ValidatorNodeNotRegistered {
                            address: public_key.to_string(),
                            epoch,
                        })
                    }),
                context,
            ),
            EpochManagerRequest::GetManyValidatorNodes { query, reply } => {
                handle(reply, self.inner.get_many_validator_nodes(query), context);
            },
            EpochManagerRequest::AddBlockHash {
                block_height,
                block_hash,
                reply,
            } => {
                handle(
                    reply,
                    self.inner.add_base_layer_block_info(block_height, block_hash),
                    context,
                );
            },
            EpochManagerRequest::UpdateEpoch {
                block_height,
                block_hash,
                reply,
            } => {
                handle(reply, self.inner.update_epoch(block_height, block_hash).await, context);
            },
            EpochManagerRequest::LastRegistrationEpoch { reply } => {
                handle(reply, self.inner.last_registration_epoch(), context)
            },

            EpochManagerRequest::UpdateLastRegistrationEpoch { epoch, reply } => {
                handle(reply, self.inner.update_last_registration_epoch(epoch), context);
            },
            EpochManagerRequest::IsEpochValid { epoch, reply } => {
                handle(reply, Ok(self.inner.is_epoch_valid(epoch)), context)
            },
            EpochManagerRequest::GetCommittees { epoch, reply } => {
                handle(reply, self.inner.get_committees(epoch), context);
            },
            EpochManagerRequest::GetCommitteeInfoByAddress { epoch, address, reply } => handle(
                reply,
                self.inner.get_committee_info_by_validator_address(epoch, address),
                context,
            ),
            EpochManagerRequest::GetCommitteeForSubstate {
                epoch,
                substate_address,
                reply,
            } => {
                handle(
                    reply,
                    self.inner.get_committee_for_substate(epoch, substate_address),
                    context,
                );
            },
            EpochManagerRequest::Subscribe { reply } => handle(reply, Ok(self.events.subscribe()), context),
            EpochManagerRequest::GetValidatorNodesPerEpoch { epoch, reply } => {
                handle(reply, self.inner.get_validator_nodes_per_epoch(epoch), context)
            },
            EpochManagerRequest::AddValidatorNodeRegistration {
                block_height,
                registration,
                value: _value,
                reply,
            } => handle(
                reply,
                self.inner
                    .add_validator_node_registration(block_height, registration)
                    .await,
                context,
            ),
            // TODO: This should be rather be a state machine event
            EpochManagerRequest::NotifyScanningComplete { reply } => {
                handle(reply, self.inner.on_scanning_complete().await, context)
            },
            EpochManagerRequest::RemainingRegistrationEpochs { reply } => {
                handle(reply, self.inner.remaining_registration_epochs().await, context)
            },
            EpochManagerRequest::GetBaseLayerConsensusConstants { reply } => handle(
                reply,
                self.inner.get_base_layer_consensus_constants().await.cloned(),
                context,
            ),
            EpochManagerRequest::GetOurValidatorNode { epoch, reply } => {
                handle(reply, self.inner.get_our_validator_node(epoch), context)
            },
            EpochManagerRequest::GetCommitteeInfo {
                epoch,
                substate_address,
                reply,
            } => handle(
                reply,
                self.inner.get_committee_info_for_substate(epoch, substate_address),
                context,
            ),
            EpochManagerRequest::GetLocalCommitteeInfo { epoch, reply } => {
                handle(reply, self.inner.get_local_committee_info(epoch), context)
            },
            EpochManagerRequest::GetNumCommittees { epoch, reply } => {
                handle(reply, self.inner.get_num_committees(epoch), context)
            },
            EpochManagerRequest::GetCommitteesForShards { epoch, shards, reply } => {
                handle(reply, self.inner.get_committees_for_shards(epoch, shards), context)
            },
            EpochManagerRequest::GetFeeClaimPublicKey { reply } => {
                handle(reply, self.inner.get_fee_claim_public_key(), context)
            },
            EpochManagerRequest::SetFeeClaimPublicKey { public_key, reply } => {
                handle(reply, self.inner.set_fee_claim_public_key(public_key), context)
            },
            EpochManagerRequest::GetBaseLayerBlockHeight { hash, reply } => {
                handle(reply, self.inner.get_base_layer_block_height(hash).await, context)
            },
            EpochManagerRequest::GetNetworkCommittees { reply } => {
                handle(reply, self.inner.get_network_committees().await, context)
            },
        }
    }
}

fn handle<T>(
    reply: oneshot::Sender<Result<T, EpochManagerError>>,
    result: Result<T, EpochManagerError>,
    context: &str,
) {
    if let Err(ref e) = result {
        error!(target: LOG_TARGET, "Request {} failed with error: {}", context, e);
    }
    if reply.send(result).is_err() {
        error!(target: LOG_TARGET, "Requester abandoned request");
    }
}
