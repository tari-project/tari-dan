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

use std::{
    mem,
    sync::{Arc, atomic::AtomicU64},
    time::Instant,
};

use log::*;
use tari_ootle_common_types::{
    Epoch,
    SubstateAddress,
    VotePower,
    committee::Committee,
    diag_event,
    diagnostics::DiagnosticSink,
    displayable::Displayable,
    optional::{IsNotFoundError, Optional},
};
use tari_ootle_storage::global::GlobalDb;
use tari_ootle_storage_sqlite::global::SqliteGlobalDbAdapter;
use tari_shutdown::ShutdownSignal;
use tari_sidechain::FixedHash;
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    EpochManagerEvent,
    epoch_event_oracle::{EpochEvent, EpochEventOracle, ValidatorNodeChange},
    error::EpochManagerError,
    service::{
        CommitteeCache,
        EpochManagerHandle,
        config::EpochManagerConfig,
        epoch_manager::EpochManager,
        types::EpochManagerRequest,
    },
    traits::EpochManagerSpec,
};

const LOG_TARGET: &str = "tari::ootle::epoch_manager";

pub struct EpochManagerService<TSpec: EpochManagerSpec> {
    rx_request: mpsc::Receiver<EpochManagerRequest<TSpec::Addr>>,
    inner: EpochManager<TSpec>,
    epoch_events: TSpec::EpochEventOracle,

    tx_events: broadcast::Sender<EpochManagerEvent>,
    is_initial_epoch_sync_complete: bool,
    has_epoch_changed: bool,
    waiting_for_scanning_complete: Vec<oneshot::Sender<Result<(), EpochManagerError>>>,

    /// Shared with the [`EpochManagerHandle`] so handle clones see the same
    /// cache. Cleared whenever committees are assigned — see `activate_epoch`.
    committee_cache: CommitteeCache<TSpec::Addr>,

    diagnostics: Arc<dyn DiagnosticSink>,

    shutdown: ShutdownSignal,
}

impl<TSpec: EpochManagerSpec> EpochManagerService<TSpec> {
    pub fn spawn(
        config: EpochManagerConfig,
        global_db: GlobalDb<SqliteGlobalDbAdapter<TSpec::Addr>>,
        epoch_events: TSpec::EpochEventOracle,
        node_public_key: RistrettoPublicKeyBytes,
        diagnostics: Arc<dyn DiagnosticSink>,
        shutdown: ShutdownSignal,
    ) -> (EpochManagerHandle<TSpec::Addr>, JoinHandle<anyhow::Result<()>>) {
        let (tx_request, rx_request) = mpsc::channel(10);
        let (events, _) = broadcast::channel(100);
        let current_epoch = Arc::new(AtomicU64::new(0));
        let committee_cache = CommitteeCache::<TSpec::Addr>::new();
        let epoch_manager_handle = EpochManagerHandle::new(
            tx_request,
            events.downgrade(),
            current_epoch.clone(),
            committee_cache.clone(),
        );

        let task_handle = tokio::spawn(async move {
            Self {
                rx_request,
                inner: EpochManager::new(config, global_db, node_public_key, current_epoch),
                tx_events: events,
                has_epoch_changed: false,
                is_initial_epoch_sync_complete: false,
                waiting_for_scanning_complete: Vec::new(),
                epoch_events,
                committee_cache,
                diagnostics,
                shutdown,
            }
            .run()
            .await?;
            Ok(())
        });

        (epoch_manager_handle, task_handle)
    }

    pub async fn run(&mut self) -> Result<(), EpochManagerError> {
        info!(target: LOG_TARGET, "🚀 Starting epoch manager");
        // first, load initial state
        self.inner.load_initial_state()?;

        loop {
            tokio::select! {
                maybe_event = self.epoch_events.next_epoch_event() => {
                    match maybe_event {
                        Some(event) => {
                            if let Err(err) = self.handle_epoch_event(event).await {
                                error!(target: LOG_TARGET, "🚨 Epoch event error: {err}");
                                self.diagnostics.emit(diag_event!(error, "epoch.event_error", "Failed to handle an epoch event: {err}", error => err));
                            }
                        }
                        None => {
                            warn!(target: LOG_TARGET, "💤 Shutting down epoch manager (no further epoch events)");
                            self.diagnostics.emit(diag_event!(error, "epoch.oracle_stopped", "The epoch oracle produced no further events. The node can no longer follow epoch changes"));
                            break;
                        }
                    }
                },

                req = self.rx_request.recv() => {
                    match req {
                        Some(req) => self.handle_request(req).await,
                        None => {
                            error!(target: LOG_TARGET, "Channel closed. Shutting down epoch manager");
                            break;
                        }
                    }
                },

                _ = self.shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down epoch manager (shutdown signal)");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_epoch_event(&mut self, event: EpochEvent) -> anyhow::Result<()> {
        match event {
            EpochEvent::Error(err) => return Err(err),
            EpochEvent::ActiveValidatorNodeSetChanged { epoch, node_changes } => {
                info!(
                    target: LOG_TARGET,
                    "⛓️ {} validator node change(s) for epoch {}", node_changes.len(), epoch,
                );
                // The birthday epoch is the first epoch any validator became active on the network; it
                // is used as a cheap floor for whether there is a previous epoch's checkpoint to sync.
                // Derive it from an activation epoch in this change set rather than the triggering
                // event's epoch, whose offset from the activation epoch differs between epoch oracle
                // implementations.
                if let Some(activation_epoch) = node_changes.iter().find_map(|c| match c {
                    ValidatorNodeChange::Add { activation_epoch, .. } => Some(*activation_epoch),
                    ValidatorNodeChange::Remove { .. } => None,
                }) {
                    self.inner.set_birthday_epoch_if_unset(activation_epoch)?;
                }

                for node_change in node_changes {
                    match node_change {
                        ValidatorNodeChange::Add {
                            claim_public_key,
                            validator_node_public_key,
                            activation_epoch,
                            minimum_value_promise: _minimum_value_promise,
                            shard_key,
                        } => {
                            info!(
                                target: LOG_TARGET,
                                "⛓️ Validator node {} activated at {}",
                                validator_node_public_key,
                                activation_epoch,
                            );

                            self.inner.add_validator_node_registration(
                                activation_epoch,
                                validator_node_public_key,
                                claim_public_key,
                                shard_key,
                                // minimum_value_promise,
                                // All validators currently get a vote power of 1
                                VotePower::of(1),
                            )?;
                        },
                        ValidatorNodeChange::Remove { public_key } => {
                            info!(
                                target: LOG_TARGET,
                                "⛓️ Deactivating validator node registration for {}",
                                public_key,
                            );

                            self.inner.deactivate_validator_node(public_key, epoch)?;
                        },
                    }
                }
            },
            EpochEvent::NewValidatorRegistered {
                epoch,
                validator_node_public_key,
                ..
            } => {
                // TODO: This occurs when a registration is detected, before the VN is activated and could be a good
                // time to start state sync
                info!(
                    target: LOG_TARGET,
                    "🖥️ New validator registered in {epoch} with public key {validator_node_public_key}",
                );
            },
            EpochEvent::NewValidatorNodeExit {
                epoch,
                validator_node_public_key,
                ..
            } => {
                info!(
                    target: LOG_TARGET,
                    "🖥️ validator exit in {epoch} with public key {validator_node_public_key}",
                );
            },
            EpochEvent::EpochChanged { epoch, epoch_hash } => {
                info!(
                    target: LOG_TARGET,
                    "🌟 new epoch {epoch} with hash {epoch_hash}",
                );
                self.activate_epoch(epoch, epoch_hash).await?;
            },

            EpochEvent::DoneForNow { epoch, .. } => {
                info!(target: LOG_TARGET, "Epoch event scanner done for now at {epoch}. Current epoch: {}", self.inner.current_epoch());
                self.on_scanning_complete()?;
            },
        }

        Ok(())
    }

    async fn activate_epoch(&mut self, epoch: Epoch, epoch_hash: FixedHash) -> Result<(), EpochManagerError> {
        use std::cmp::Ordering;
        match epoch.cmp(&self.current_epoch()) {
            Ordering::Less => Ok(()),
            Ordering::Equal => {
                // Same epoch re-activated. Permit a hash correction only if the epoch has not yet
                // been locked in by a committed EndEpoch block. This lets the base-layer scanner
                // self-heal during the pre-commit window if a reorg near the epoch boundary causes
                // the initial hash to be wrong, without ever rewriting a hash or committee that
                // consensus has already committed against. A correction reassigns the epoch's
                // committees too, since the rescan that produced it may have changed the validators
                // registered for the epoch.
                if self.inner.current_epoch_hash() == epoch_hash {
                    return Ok(());
                }
                if self.inner.is_epoch_locked(epoch) {
                    warn!(
                        target: LOG_TARGET,
                        "⛓️ Ignoring oracle hash correction for already-locked epoch {}: stored={}, candidate={}",
                        epoch,
                        self.inner.current_epoch_hash(),
                        epoch_hash,
                    );
                    return Ok(());
                }
                warn!(
                    target: LOG_TARGET,
                    "🩹 Correcting epoch_hash for {}: {} -> {}",
                    epoch,
                    self.inner.current_epoch_hash(),
                    epoch_hash,
                );
                self.diagnostics.emit(diag_event!(
                    warn,
                    "epoch.hash_corrected",
                    "Corrected the epoch hash for epoch {epoch}",
                    epoch => epoch,
                    previous => self.inner.current_epoch_hash(),
                    corrected => epoch_hash
                ));
                self.inner.advance_to_epoch(epoch, epoch_hash)?;
                self.committee_cache.clear().await;
                Ok(())
            },
            Ordering::Greater => {
                self.has_epoch_changed = true;
                self.diagnostics.emit(diag_event!(
                    info,
                    "epoch.changed",
                    "Epoch advanced to {epoch}",
                    epoch => epoch,
                    epoch_hash => epoch_hash
                ));
                // In the base layer case, the epoch_hash is the first block of the epoch
                // persist the epoch data including the validator node set
                self.inner.advance_to_epoch(epoch, epoch_hash)?;
                // Bust the shared cache so subsequent handle reads pick up the freshly assigned
                // committee rows for the new epoch.
                self.committee_cache.clear().await;
                Ok(())
            },
        }
    }

    fn current_epoch(&self) -> Epoch {
        self.inner.current_epoch()
    }

    fn on_scanning_complete(&mut self) -> Result<(), EpochManagerError> {
        let current_epoch = self.inner.current_epoch();
        if !self.is_initial_epoch_sync_complete {
            info!(
                target: LOG_TARGET,
                "🌟 Initial epoch sync complete. Current epoch is {}", current_epoch
            );
            self.is_initial_epoch_sync_complete = true;
            for reply in mem::take(&mut self.waiting_for_scanning_complete) {
                let _ignore = reply.send(Ok(()));
            }
        }

        if self.has_epoch_changed {
            let num_committees = self.inner.get_number_of_committees(current_epoch)?;
            let shard_group = self.inner.get_our_validator_node(current_epoch).optional()?.map(|vn| {
                vn.shard_key
                    .to_shard_group(self.inner.config().num_preshards, num_committees)
            });
            let level = if self.is_initial_epoch_sync_complete {
                Level::Info
            } else {
                Level::Debug
            };
            log!(target: LOG_TARGET, level, "🌟 A new epoch {} is upon us. Shard group: {}", current_epoch, shard_group.display());

            self.publish_event(EpochManagerEvent::EpochChanged {
                epoch: current_epoch,
                registered_shard_group: shard_group,
                activated_at: Instant::now(),
            });
            self.has_epoch_changed = false;
        }

        Ok(())
    }

    fn publish_event(&mut self, event: EpochManagerEvent) {
        let _ignore = self.tx_events.send(event);
    }

    fn add_notify_on_scanning_complete(&mut self, reply: oneshot::Sender<Result<(), EpochManagerError>>) {
        if self.is_initial_epoch_sync_complete {
            let _ignore = reply.send(Ok(()));
        } else {
            self.waiting_for_scanning_complete.push(reply);
        }
    }

    async fn get_committee_for_substate(
        &mut self,
        epoch: Epoch,
        substate_address: SubstateAddress,
    ) -> Result<Arc<Committee<TSpec::Addr>>, EpochManagerError> {
        let num_committees = self.inner.get_number_of_committees(epoch)?;
        let shard_group = substate_address.to_shard_group(self.inner.config().num_preshards, num_committees);

        self.committee_cache
            .get_or_try_init((epoch, shard_group), || async {
                Ok(Arc::new(
                    self.inner.get_committee_for_substate(epoch, substate_address)?,
                ))
            })
            .await
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_request(&mut self, req: EpochManagerRequest<TSpec::Addr>) {
        trace!(target: LOG_TARGET, "Received request: {:?}", req);
        let context = &format!("{:?}", req);
        match req {
            EpochManagerRequest::CurrentEpoch { reply } => handle(reply, Ok(self.inner.current_epoch()), context),
            EpochManagerRequest::GetEpochHash { epoch, reply } => {
                handle(reply, self.inner.get_epoch_hash(epoch), context)
            },
            EpochManagerRequest::GetCurrentEpochHash { reply } => {
                handle(reply, Ok(self.inner.current_epoch_hash()), context)
            },
            EpochManagerRequest::GetValidatorSetHash { epoch, reply } => {
                handle(reply, self.inner.get_validator_set_hash(epoch), context)
            },
            EpochManagerRequest::GetValidatorSetCommitteeProof {
                epoch,
                shard_group,
                reply,
            } => handle(
                reply,
                self.inner.get_validator_set_committee_proof(epoch, shard_group),
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
                    self.get_committee_for_substate(epoch, substate_address).await,
                    context,
                );
            },
            EpochManagerRequest::GetValidatorNodesPerEpoch { epoch, reply } => {
                handle(reply, self.inner.get_validator_nodes_per_epoch(epoch), context)
            },
            EpochManagerRequest::AddValidatorNodeRegistration {
                activation_epoch,
                validator_public_key,
                claim_public_key,
                shard_key,
                power,
                reply,
            } => handle(
                reply,
                self.inner.add_validator_node_registration(
                    activation_epoch,
                    validator_public_key,
                    claim_public_key,
                    shard_key,
                    power,
                ),
                context,
            ),
            EpochManagerRequest::DeactivateValidatorNode {
                public_key,
                deactivation_epoch,
                reply,
            } => handle(
                reply,
                self.inner.deactivate_validator_node(public_key, deactivation_epoch),
                context,
            ),
            EpochManagerRequest::IsInitialScanningComplete { reply } => {
                handle(reply, Ok(self.is_initial_epoch_sync_complete), context)
            },
            EpochManagerRequest::WaitForInitialScanningToComplete { reply } => {
                self.add_notify_on_scanning_complete(reply);
            },

            EpochManagerRequest::GetOurValidatorNode { epoch, reply } => {
                handle(reply, self.inner.get_our_validator_node(epoch), context)
            },
            EpochManagerRequest::GetCommitteeInfoForSubstate {
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
            EpochManagerRequest::GetCommitteeInfo {
                epoch,
                shard_group,
                reply,
            } => handle(reply, self.inner.get_committee_info(epoch, shard_group), context),
            EpochManagerRequest::GetNumCommittees { epoch, reply } => {
                handle(reply, self.inner.get_num_committees(epoch), context)
            },
            EpochManagerRequest::GetCommitteeForShardGroup {
                epoch,
                shard_group,
                reply,
            } => handle(
                reply,
                self.inner
                    .get_committee_for_shard_group(epoch, shard_group, None)
                    .map(Arc::new),
                context,
            ),
            EpochManagerRequest::GetCommitteesOverlappingShardGroup {
                epoch,
                shard_group,
                reply,
            } => handle(
                reply,
                self.inner.get_committees_overlapping_shard_group(epoch, shard_group),
                context,
            ),
            EpochManagerRequest::GetFeeClaimPublicKey { reply } => {
                handle(reply, self.inner.get_fee_claim_public_key(), context)
            },

            EpochManagerRequest::GetRandomCommitteeMemberFromShardGroup {
                epoch,
                shard_group,
                excluding,
                reply,
            } => handle(
                reply,
                self.inner
                    .get_random_committee_member_from_shard_group(epoch, shard_group, excluding),
                context,
            ),
            EpochManagerRequest::GetNetworkDescription { reply } => {
                handle(reply, self.inner.get_network_description(), context)
            },
            EpochManagerRequest::LockEpoch { epoch, reply } => {
                handle(reply, self.inner.lock_epoch(epoch), context);
            },
            EpochManagerRequest::GetObservedEpochHash { epoch, reply } => {
                // An activated epoch's stored hash wins: it is the one the self-healing correction
                // maintains and that `lock_epoch` freezes once consensus has committed against it.
                let result = match self.inner.get_epoch_hash(epoch).optional() {
                    Ok(Some(activated)) => Ok(Some(activated)),
                    Ok(None) => self.epoch_events.observed_epoch_boundary_hash(epoch).map_err(|err| {
                        // `{:#}` keeps anyhow's source chain: the diesel error underneath is the part
                        // that says why the store was unreadable.
                        EpochManagerError::EpochEventOracleError {
                            details: format!("{err:#}"),
                        }
                    }),
                    Err(err) => Err(err),
                };
                handle(reply, result, context);
            },
            EpochManagerRequest::GetBirthdayEpoch { reply } => {
                handle(reply, Ok(self.inner.birthday_epoch()), context);
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
        // These responses are not errors
        if !e.is_not_registered_error() && !e.is_not_found_error() {
            error!(target: LOG_TARGET, "Request {} failed with error: {}", context, e);
        }
    }
    if reply.send(result).is_err() {
        error!(target: LOG_TARGET, "Requester abandoned request");
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use diesel::{Connection, SqliteConnection};
    use tari_ootle_common_types::{NumPreshards, ShardGroup, diagnostics::NoopSink};
    use tari_shutdown::Shutdown;

    use super::*;

    type TestGlobalDb = GlobalDb<SqliteGlobalDbAdapter<RistrettoPublicKeyBytes>>;

    struct NoOracle;

    impl EpochEventOracle for NoOracle {
        async fn next_epoch_event(&mut self) -> Option<EpochEvent> {
            None
        }
    }

    struct TestSpec;

    impl EpochManagerSpec for TestSpec {
        type Addr = RistrettoPublicKeyBytes;
        type EpochEventOracle = NoOracle;
    }

    fn public_key(byte: u8) -> RistrettoPublicKeyBytes {
        RistrettoPublicKeyBytes::from_bytes(&[byte; 32]).unwrap()
    }

    fn register(global_db: &TestGlobalDb, byte: u8, start_epoch: Epoch) {
        let mut tx = global_db.create_transaction().unwrap();
        global_db
            .validator_nodes(&mut tx)
            .insert_validator_node(
                public_key(byte),
                public_key(byte),
                SubstateAddress::from_array([byte; SubstateAddress::LENGTH]),
                start_epoch,
                public_key(byte),
                VotePower::of(1),
            )
            .unwrap();
        tx.commit().unwrap();
    }

    fn create_service(global_db: TestGlobalDb, shutdown: &Shutdown) -> EpochManagerService<TestSpec> {
        let config = EpochManagerConfig {
            network: ootle_network::Network::LocalNet,
            base_layer_confirmations: 0,
            committee_size: NonZeroU32::new(10).unwrap(),
            validator_node_sidechain_id: None,
            num_preshards: NumPreshards::P256,
            fee_claim_public_key: RistrettoPublicKeyBytes::zero(),
        };
        let (_, rx_request) = mpsc::channel(1);
        let (tx_events, _) = broadcast::channel(1);
        EpochManagerService {
            rx_request,
            inner: EpochManager::new(
                config,
                global_db,
                RistrettoPublicKeyBytes::zero(),
                Arc::new(AtomicU64::new(0)),
            ),
            epoch_events: NoOracle,
            tx_events,
            is_initial_epoch_sync_complete: false,
            has_epoch_changed: false,
            waiting_for_scanning_complete: Vec::new(),
            committee_cache: CommitteeCache::new(),
            diagnostics: Arc::new(NoopSink),
            shutdown: shutdown.to_signal(),
        }
    }

    /// Reads the committee the way an [`EpochManagerHandle`] does, through the shared cache.
    async fn cached_committee(
        service: &EpochManagerService<TestSpec>,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Vec<RistrettoPublicKeyBytes> {
        let committee = service
            .committee_cache
            .get_or_try_init((epoch, shard_group), || async {
                service
                    .inner
                    .get_committee_for_shard_group(epoch, shard_group, None)
                    .map(Arc::new)
            })
            .await
            .unwrap();
        let mut keys = committee.public_keys().copied().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    /// When the oracle corrects the hash of the current, unlocked epoch, the epoch's committees must be
    /// reassigned from the validators registered for it at that point, as a node activating the epoch for the
    /// first time with the corrected hash would assign them.
    #[tokio::test]
    async fn hash_correction_reassigns_the_current_epochs_committees() {
        let shutdown = Shutdown::new();
        let all_shards = ShardGroup::all_shards(NumPreshards::P256);
        let global_db = GlobalDb::new(SqliteGlobalDbAdapter::new(
            SqliteConnection::establish(":memory:").unwrap(),
        ));
        global_db.adapter().migrate().unwrap();
        let mut service = create_service(global_db.clone(), &shutdown);

        register(&global_db, 1, Epoch(5));
        service
            .activate_epoch(Epoch(5), FixedHash::from([1; 32]))
            .await
            .unwrap();
        assert_eq!(cached_committee(&service, Epoch(5), all_shards).await, vec![
            public_key(1)
        ]);

        register(&global_db, 2, Epoch(5));
        service
            .activate_epoch(Epoch(5), FixedHash::from([2; 32]))
            .await
            .unwrap();

        assert_eq!(service.inner.current_epoch_hash(), FixedHash::from([2; 32]));
        let committee_info = service.inner.get_committee_info(Epoch(5), all_shards).unwrap();
        assert_eq!(committee_info.num_shard_group_members(), 2);
        assert!(
            service
                .inner
                .get_validator_node_by_public_key(Epoch(5), &public_key(2))
                .unwrap()
                .is_some()
        );
        assert_eq!(cached_committee(&service, Epoch(5), all_shards).await, vec![
            public_key(1),
            public_key(2)
        ]);
    }

    /// A locked epoch's committees are never reassigned, even if the oracle later reports a different hash.
    #[tokio::test]
    async fn hash_correction_leaves_a_locked_epochs_committees() {
        let shutdown = Shutdown::new();
        let all_shards = ShardGroup::all_shards(NumPreshards::P256);
        let global_db = GlobalDb::new(SqliteGlobalDbAdapter::new(
            SqliteConnection::establish(":memory:").unwrap(),
        ));
        global_db.adapter().migrate().unwrap();
        let mut service = create_service(global_db.clone(), &shutdown);

        register(&global_db, 1, Epoch(5));
        service
            .activate_epoch(Epoch(5), FixedHash::from([1; 32]))
            .await
            .unwrap();
        service.inner.lock_epoch(Epoch(5)).unwrap();

        register(&global_db, 2, Epoch(5));
        service
            .activate_epoch(Epoch(5), FixedHash::from([2; 32]))
            .await
            .unwrap();

        assert_eq!(service.inner.current_epoch_hash(), FixedHash::from([1; 32]));
        let committee_info = service.inner.get_committee_info(Epoch(5), all_shards).unwrap();
        assert_eq!(committee_info.num_shard_group_members(), 1);
    }
}
