//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::time::Instant;

use async_trait::async_trait;
use log::*;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_storage::{consensus_models::SubstateRecord, StateStore, StorageError};
use tari_engine_types::substate::{Substate, SubstateAddress};
use tari_epoch_manager::EpochManagerReader;
use tari_indexer_lib::{error::IndexerError, substate_scanner::SubstateScanner};
use tari_transaction::Transaction;
use tari_validator_node_rpc::client::{SubstateResult, ValidatorNodeClientFactory};

use crate::p2p::services::mempool::SubstateResolver;

const LOG_TARGET: &str = "tari::dan::substate_resolver";

#[derive(Debug, Clone)]
pub struct TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory> {
    store: TStateStore,
    scanner: SubstateScanner<TEpochManager, TValidatorNodeClientFactory>,
}

impl<TStateStore, TEpochManager, TValidatorNodeClientFactory>
    TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory>
where
    TStateStore: StateStore,
    TEpochManager: EpochManagerReader<Addr = CommsPublicKey>,
    TValidatorNodeClientFactory: ValidatorNodeClientFactory,
{
    pub fn new(store: TStateStore, scanner: SubstateScanner<TEpochManager, TValidatorNodeClientFactory>) -> Self {
        Self { store, scanner }
    }
}

#[async_trait]
impl<TStateStore, TEpochManager, TValidatorNodeClientFactory> SubstateResolver
    for TariSubstateResolver<TStateStore, TEpochManager, TValidatorNodeClientFactory>
where
    TStateStore: StateStore + Sync + Send,
    TEpochManager: EpochManagerReader<Addr = CommsPublicKey>,
    TValidatorNodeClientFactory: ValidatorNodeClientFactory<Addr = CommsPublicKey>,
{
    type Error = SubstateResolverError;

    async fn resolve<T>(&self, transaction: &Transaction, out: &mut T) -> Result<(), Self::Error>
    where T: Extend<(SubstateAddress, Substate)> + Send + Send {
        // TODO: on second thoughts, we should error if any local shards dont exist rather than requesting from remotes
        let (local_substates, missing_shards) = self
            .store
            .with_read_tx(|tx| SubstateRecord::get_any(tx, transaction.all_inputs_iter()))?;

        // TODO: If any of the missing shards are local we should/could error early here rather than asking the local
        // committee
        info!(
            target: LOG_TARGET,
            "Found {} local substates and {} missing shards",
            local_substates.len(),
            missing_shards.len());

        out.extend(
            local_substates
                .into_iter()
                .map(|s| (s.address.clone(), s.into_substate())),
        );

        let mut retrieved_substates = Vec::with_capacity(missing_shards.len());
        for shard in missing_shards {
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

        out.extend(retrieved_substates);

        Ok(())
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
}
