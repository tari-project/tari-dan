//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, time::Instant};

use async_trait::async_trait;
use log::*;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::{Epoch, ShardId};
use tari_dan_engine::{runtime::VirtualSubstates, state_store::memory::MemoryStateStore};
use tari_dan_storage::{consensus_models::SubstateRecord, StateStore, StorageError};
use tari_engine_types::{
    instruction::Instruction,
    substate::SubstateAddress,
    virtual_substate::{VirtualSubstate, VirtualSubstateAddress},
};
use tari_epoch_manager::{EpochManagerError, EpochManagerReader};
use tari_indexer_lib::{error::IndexerError, substate_scanner::SubstateScanner};
use tari_transaction::Transaction;
use tari_validator_node_rpc::client::{SubstateResult, ValidatorNodeClientFactory};

use crate::{
    p2p::services::mempool::SubstateResolver,
    virtual_substate::{VirtualSubstateError, VirtualSubstateManager},
};

const LOG_TARGET: &str = "tari::dan::substate_resolver";

#[derive(Debug, Clone)]
pub struct TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory> {
    store: TStateStore,
    scanner: SubstateScanner<TEpochManager, TValidatorNodeClientFactory>,
    epoch_manager: TEpochManager,
    virtual_substate_manager: VirtualSubstateManager<TStateStore, TEpochManager>,
}

impl<TStateStore, TEpochManager, TValidatorNodeClientFactory>
    TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory>
where
    TStateStore: StateStore,
    TEpochManager: EpochManagerReader<Addr = CommsPublicKey>,
    TValidatorNodeClientFactory: ValidatorNodeClientFactory<Addr = CommsPublicKey>,
{
    pub fn new(
        store: TStateStore,
        scanner: SubstateScanner<TEpochManager, TValidatorNodeClientFactory>,
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

    fn resolve_local_substates(
        &self,
        transaction: &Transaction,
        out: &MemoryStateStore,
    ) -> Result<HashSet<ShardId>, SubstateResolverError> {
        let (local_substates, missing_shards) = self
            .store
            .with_read_tx(|tx| SubstateRecord::get_any(tx, transaction.all_inputs_iter()))?;

        info!(
            target: LOG_TARGET,
            "Found {} local substates and {} missing shards",
            local_substates.len(),
            missing_shards.len());

        out.set_all(
            local_substates
                .into_iter()
                .map(|s| (s.address.clone(), s.into_substate())),
        );

        Ok(missing_shards)
    }

    async fn resolve_remote_substates(
        &self,
        shards: HashSet<ShardId>,
        out: &MemoryStateStore,
    ) -> Result<(), SubstateResolverError> {
        let mut retrieved_substates = Vec::with_capacity(shards.len());
        for shard in shards {
            let timer = Instant::now();
            let substate_result = self
                .scanner
                .get_specific_substate_from_committee_by_shard(shard)
                .await?;

            match substate_result {
                SubstateResult::Up { address, substate, .. } => {
                    info!(
                        target: LOG_TARGET,
                        "Retrieved substate {} in {}ms",
                        address,
                        timer.elapsed().as_millis()
                    );
                    retrieved_substates.push((address, substate));
                },
                SubstateResult::Down { address, version, .. } => {
                    return Err(SubstateResolverError::InputSubstateDowned { address, version });
                },
                SubstateResult::DoesNotExist => {
                    return Err(SubstateResolverError::InputSubstateDoesNotExist { shard });
                },
            }
        }

        out.set_all(retrieved_substates);

        Ok(())
    }

    async fn resolve_remote_virtual_substates(
        &self,
        claim_instructions: Vec<(Epoch, CommsPublicKey, ShardId)>,
    ) -> Result<VirtualSubstates, SubstateResolverError> {
        let mut retrieved_substates = VirtualSubstates::with_capacity(claim_instructions.len());
        for (epoch, vn_pk, shard) in claim_instructions {
            let timer = Instant::now();
            let address = VirtualSubstateAddress::UnclaimedValidatorFee {
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
impl<TStateStore, TEpochManager, TValidatorNodeClientFactory> SubstateResolver
    for TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory>
where
    TStateStore: StateStore<Addr = CommsPublicKey> + Sync + Send,
    TEpochManager: EpochManagerReader<Addr = CommsPublicKey>,
    TValidatorNodeClientFactory: ValidatorNodeClientFactory<Addr = CommsPublicKey>,
{
    type Error = SubstateResolverError;

    async fn resolve(&self, transaction: &Transaction, out: &MemoryStateStore) -> Result<(), Self::Error> {
        let missing_shards = self.resolve_local_substates(transaction, out)?;

        // TODO: If any of the missing shards are local we should error early here rather than asking the local
        //       committee

        self.resolve_remote_substates(missing_shards, out).await?;

        Ok(())
    }

    async fn resolve_virtual_substates(
        &self,
        transaction: &Transaction,
        current_epoch: Epoch,
    ) -> Result<VirtualSubstates, Self::Error> {
        let claim_instructions = transaction
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
            VirtualSubstateAddress::CurrentEpoch,
            VirtualSubstate::CurrentEpoch(current_epoch.as_u64()),
        );

        if claim_instructions.is_empty() {
            return Ok(virtual_substates);
        }

        let local_committee_shard = self.epoch_manager.get_local_committee_shard(current_epoch).await?;
        #[allow(clippy::mutable_key_type)]
        let validators = self
            .epoch_manager
            .get_many_validator_nodes(claim_instructions.clone())
            .await?;

        let signer = transaction.signer_public_key();
        if let Some(vn) = validators.values().find(|vn| vn.fee_claim_public_key != *signer) {
            return Err(SubstateResolverError::UnauthorizedFeeClaim {
                validator_address: vn.address.clone(),
                signer: signer.clone(),
            });
        }

        // Partition the claim instructions into local and remote claims
        let mut local_claim_vns = Vec::new();
        let mut remote_claim_vns = Vec::new();
        claim_instructions.into_iter().for_each(|(epoch, public_key)| {
            let vn = validators.get(&(epoch, public_key.clone())).unwrap();
            if local_committee_shard.includes_shard(&vn.shard_key) {
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
    #[error("Input substate does not exist: {shard}")]
    InputSubstateDoesNotExist { shard: ShardId },
    #[error("Input substate is downed: {address} (version: {version})")]
    InputSubstateDowned { address: SubstateAddress, version: u32 },
    #[error("Virtual substate error: {0}")]
    VirtualSubstateError(#[from] VirtualSubstateError),
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Unauthorized fee claim: validator node {validator_address} (transaction signed by: {signer})")]
    UnauthorizedFeeClaim {
        validator_address: CommsPublicKey,
        signer: CommsPublicKey,
    },
}
