//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, time::Instant};

use async_trait::async_trait;
use indexmap::IndexMap;
use log::*;
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{Epoch, SubstateAddress};
use tari_dan_engine::state_store::StateStoreError;
use tari_dan_storage::{consensus_models::SubstateRecord, StateStore, StorageError};
use tari_engine_types::{
    instruction::Instruction,
    substate::{Substate, SubstateId},
    virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates},
};
use tari_epoch_manager::{EpochManagerError, EpochManagerReader};
use tari_indexer_lib::{error::IndexerError, substate_cache::SubstateCache, substate_scanner::SubstateScanner};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};
use tari_validator_node_rpc::client::{SubstateResult, ValidatorNodeClientFactory};

use crate::{
    p2p::services::mempool::{ResolvedSubstates, SubstateResolver},
    virtual_substate::{VirtualSubstateError, VirtualSubstateManager},
};

const LOG_TARGET: &str = "tari::dan::substate_resolver";

#[derive(Debug, Clone)]
pub struct TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory, TSubstateCache> {
    store: TStateStore,
    scanner: SubstateScanner<TEpochManager, TValidatorNodeClientFactory, TSubstateCache>,
    epoch_manager: TEpochManager,
    virtual_substate_manager: VirtualSubstateManager<TStateStore, TEpochManager>,
}

impl<TStateStore, TEpochManager, TValidatorNodeClientFactory, TSubstateCache>
    TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory, TSubstateCache>
where
    TStateStore: StateStore,
    TEpochManager: EpochManagerReader<Addr = TStateStore::Addr>,
    TValidatorNodeClientFactory: ValidatorNodeClientFactory<Addr = TStateStore::Addr>,
    TSubstateCache: SubstateCache,
{
    pub fn new(
        store: TStateStore,
        scanner: SubstateScanner<TEpochManager, TValidatorNodeClientFactory, TSubstateCache>,
        epoch_manager: TEpochManager,
        virtual_substate_manager: VirtualSubstateManager<TStateStore, TEpochManager>,
    ) -> Self {
        Self {
            store,
            scanner,
            epoch_manager,
            virtual_substate_manager,
        }
    }

    fn resolve_local_substates(&self, transaction: &Transaction) -> Result<ResolvedSubstates, SubstateResolverError> {
        let mut substates = IndexMap::new();
        let inputs = transaction.all_inputs_substate_ids_iter();
        let (mut found_local_substates, missing_substate_ids) = self
            .store
            .with_read_tx(|tx| SubstateRecord::get_any_max_version(tx, inputs))?;

        // Reconcile requested inputs with found local substates
        let mut missing_substates = HashSet::with_capacity(missing_substate_ids.len());
        for requested_input in transaction.all_inputs_iter() {
            if missing_substate_ids.contains(requested_input.substate_id()) {
                missing_substates.insert(requested_input);
                // Not a local substate, so we will need to fetch it remotely
                continue;
            }

            match requested_input.version() {
                // Specific version requested
                Some(requested_version) => {
                    let maybe_match = found_local_substates
                        .iter()
                        .find(|s| s.version() == requested_version && s.substate_id() == requested_input.substate_id());

                    match maybe_match {
                        Some(substate) => {
                            if substate.is_destroyed() {
                                return Err(SubstateResolverError::InputSubstateDowned {
                                    id: requested_input.into_substate_id(),
                                    version: requested_version,
                                });
                            }
                            // OK
                        },
                        // Requested substate or version not found. We know that the requested substate is not foreign
                        // because we checked missing_substate_ids
                        None => {
                            return Err(SubstateResolverError::InputSubstateDoesNotExist {
                                substate_requirement: requested_input,
                            });
                        },
                    }
                },
                // No version specified, so we will use the latest version
                None => {
                    let (pos, substate) = found_local_substates
                        .iter()
                        .enumerate()
                        .find(|(_, s)| s.substate_id() == requested_input.substate_id())
                        // This is not possible
                        .ok_or_else(|| {
                            error!(
                                target: LOG_TARGET,
                                "🐞 BUG: Requested substate {} was not missing but was also not found",
                                requested_input.substate_id()
                            );
                            SubstateResolverError::InputSubstateDoesNotExist { substate_requirement: requested_input.clone()}
                        })?;

                    if substate.is_destroyed() {
                        // The requested substate is downed locally, it may be available in a foreign shard so we add it
                        // to missing
                        let _substate = found_local_substates.remove(pos);
                        missing_substates.insert(requested_input);
                        continue;
                    }

                    // User did not specify the version, so we will use the latest version
                    // Ok
                },
            }
        }

        info!(
            target: LOG_TARGET,
            "Found {} local substates and {} missing substates",
            found_local_substates.len(),
            missing_substate_ids.len(),
        );

        substates.extend(
            found_local_substates
                .into_iter()
                .map(|s| (s.substate_id.clone(), s.into_substate())),
        );

        Ok(ResolvedSubstates {
            local: substates,
            unresolved_foreign: missing_substates,
        })
    }

    async fn resolve_remote_substates(
        &self,
        requested_substates: &HashSet<SubstateRequirement>,
    ) -> Result<IndexMap<SubstateId, Substate>, SubstateResolverError> {
        let mut substates = IndexMap::with_capacity(requested_substates.len());
        for substate_req in requested_substates {
            let timer = Instant::now();
            let substate_result = self
                .scanner
                .get_substate(substate_req.substate_id(), substate_req.version())
                .await?;

            match substate_result {
                SubstateResult::Up { id, substate, .. } => {
                    info!(
                        target: LOG_TARGET,
                        "Retrieved substate {} in {}ms",
                        id,
                        timer.elapsed().as_millis()
                    );
                    substates.insert(id, substate);
                },
                SubstateResult::Down { id, version, .. } => {
                    return Err(SubstateResolverError::InputSubstateDowned { id, version });
                },
                SubstateResult::DoesNotExist => {
                    return Err(SubstateResolverError::InputSubstateDoesNotExist {
                        substate_requirement: substate_req.clone(),
                    });
                },
            }
        }

        Ok(substates)
    }

    async fn resolve_remote_virtual_substates(
        &self,
        claim_instructions: Vec<(Epoch, PublicKey, SubstateAddress)>,
    ) -> Result<VirtualSubstates, SubstateResolverError> {
        let mut retrieved_substates = VirtualSubstates::with_capacity(claim_instructions.len());
        for (epoch, vn_pk, shard) in claim_instructions {
            let timer = Instant::now();
            let address = VirtualSubstateId::UnclaimedValidatorFee {
                epoch: epoch.as_u64(),
                address: vn_pk,
            };

            let virtual_substate = self
                .scanner
                .get_virtual_substate_from_committee(address.clone(), shard)
                .await?;

            info!(
                target: LOG_TARGET,
                "Retrieved virtual substate {} in {:.2?}",
                address,
                timer.elapsed()
            );
            retrieved_substates.insert(address, virtual_substate);
        }

        Ok(retrieved_substates)
    }
}

#[async_trait]
impl<TStateStore, TEpochManager, TValidatorNodeClientFactory, TSubstateCache> SubstateResolver
    for TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory, TSubstateCache>
where
    TStateStore: StateStore + Sync + Send,
    TEpochManager: EpochManagerReader<Addr = TStateStore::Addr>,
    TValidatorNodeClientFactory: ValidatorNodeClientFactory<Addr = TStateStore::Addr>,
    TSubstateCache: SubstateCache,
{
    type Error = SubstateResolverError;

    fn try_resolve_local(&self, transaction: &Transaction) -> Result<ResolvedSubstates, Self::Error> {
        self.resolve_local_substates(transaction)
    }

    async fn try_resolve_foreign(
        &self,
        requested_substates: &HashSet<SubstateRequirement>,
    ) -> Result<IndexMap<SubstateId, Substate>, Self::Error> {
        self.resolve_remote_substates(requested_substates).await
    }

    async fn resolve_virtual_substates(
        &self,
        transaction: &Transaction,
        current_epoch: Epoch,
    ) -> Result<VirtualSubstates, Self::Error> {
        let claim_epoch_and_public_key = transaction
            .instructions()
            .iter()
            .chain(transaction.fee_instructions())
            .filter_map(|instruction| {
                if let Instruction::ClaimValidatorFees {
                    epoch,
                    validator_public_key,
                } = instruction
                {
                    Some((Epoch(*epoch), validator_public_key.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let mut virtual_substates = VirtualSubstates::new();
        virtual_substates.insert(
            VirtualSubstateId::CurrentEpoch,
            VirtualSubstate::CurrentEpoch(current_epoch.as_u64()),
        );

        if claim_epoch_and_public_key.is_empty() {
            return Ok(virtual_substates);
        }

        let local_committee_shard = self.epoch_manager.get_local_committee_info(current_epoch).await?;
        #[allow(clippy::mutable_key_type)]
        let validators = self
            .epoch_manager
            .get_many_validator_nodes(claim_epoch_and_public_key.clone())
            .await?;

        if let Some(vn) = validators.values().find(|vn| {
            transaction
                .signatures()
                .iter()
                .all(|sig| vn.fee_claim_public_key != *sig.public_key())
        }) {
            return Err(SubstateResolverError::UnauthorizedFeeClaim {
                validator_address: vn.address.to_string(),
                transaction_id: *transaction.id(),
            });
        }

        // Partition the claim instructions into local and remote claims
        let mut local_claim_vns = Vec::new();
        let mut remote_claim_vns = Vec::new();
        claim_epoch_and_public_key.into_iter().for_each(|(epoch, public_key)| {
            let vn = validators.get(&(epoch, public_key.clone())).unwrap();
            if local_committee_shard.includes_substate_address(&vn.shard_key) {
                local_claim_vns.push((epoch, public_key))
            } else {
                remote_claim_vns.push((epoch, public_key, vn.shard_key))
            }
        });

        let local_virtual_substates = self.virtual_substate_manager.get_virtual_substates(local_claim_vns)?;
        let remote_virtual_substates = self.resolve_remote_virtual_substates(remote_claim_vns).await?;

        Ok(virtual_substates
            .into_iter()
            .chain(local_virtual_substates)
            .chain(remote_virtual_substates)
            .collect())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubstateResolverError {
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
    #[error("Indexer error: {0}")]
    IndexerError(#[from] IndexerError),
    #[error("Input substate does not exist: {substate_requirement}")]
    InputSubstateDoesNotExist { substate_requirement: SubstateRequirement },
    #[error("Input substate is downed: {id} (version: {version})")]
    InputSubstateDowned { id: SubstateId, version: u32 },
    #[error("Virtual substate error: {0}")]
    VirtualSubstateError(#[from] VirtualSubstateError),
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Unauthorized fee claim: validator node {validator_address} (transaction: {transaction_id})")]
    UnauthorizedFeeClaim {
        validator_address: String,
        transaction_id: TransactionId,
    },
    #[error("State store error: {0}")]
    StateStorageError(#[from] StateStoreError),
}
