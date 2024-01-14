//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, sync::Arc};

use log::*;
use tari_consensus::traits::VoteSignatureService;
use tari_dan_common_types::{ShardId, DerivableFromPublicKey};
use tari_engine_types::{
    indexed_value::IndexedValueError,
    substate::{Substate, SubstateAddress},
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::{SubstateRequirement, Transaction};
use tari_validator_node_rpc::client::{SubstateResult, ValidatorNodeClientFactory};
use tokio::task::JoinError;

use crate::{
    error::IndexerError,
    substate_cache::SubstateCache,
    substate_decoder::find_related_substates,
    substate_scanner::SubstateScanner,
};

const LOG_TARGET: &str = "tari::indexer::transaction_autofiller";

#[derive(Debug, thiserror::Error)]
pub enum TransactionAutofillerError {
    #[error("Could not decode the substate: {0}")]
    IndexedValueVisitorError(#[from] IndexedValueError),
    #[error("Indexer error: {0}")]
    IndexerError(#[from] IndexerError),
    #[error("Tokio join error: {0}")]
    JoinError(#[from] JoinError),
}

pub struct TransactionAutofiller<TEpochManager, TVnClient, TSubstateCache, TSignatureService> {
    substate_scanner: Arc<SubstateScanner<TEpochManager, TVnClient, TSubstateCache, TSignatureService>>,
}

impl<TEpochManager, TVnClient, TAddr, TSubstateCache, TSignatureService> TransactionAutofiller<TEpochManager, TVnClient, TSubstateCache, TSignatureService>
where
    TEpochManager: EpochManagerReader<Addr = TAddr> + 'static,
    TVnClient: ValidatorNodeClientFactory<Addr = TAddr> + 'static,
    TAddr: DerivableFromPublicKey + 'static,
    TSubstateCache: SubstateCache + 'static,
    TSignatureService: VoteSignatureService + Send + Sync + 'static,
{
    pub fn new(substate_scanner: Arc<SubstateScanner<TEpochManager, TVnClient, TSubstateCache, TSignatureService>>) -> Self {
        Self { substate_scanner }
    }

    pub async fn autofill_transaction(
        &self,
        original_transaction: Transaction,
        substate_requirements: Vec<SubstateRequirement>,
    ) -> Result<(Transaction, HashMap<SubstateAddress, Substate>), TransactionAutofillerError> {
        // we will include the inputs and outputs into the "involved_objects" field
        // note that the transaction hash will not change as the "involved_objects" is not part of the hash
        let mut autofilled_transaction = original_transaction;

        // scan the network in parallel to fetch all the substates for each required input
        let mut input_shards = vec![];
        let mut found_substates = HashMap::new();
        let substate_scanner_ref = self.substate_scanner.clone();
        let transaction_ref = Arc::new(autofilled_transaction.clone());
        let mut handles = Vec::new();
        for requirement in &substate_requirements {
            let handle = tokio::spawn(get_substate_requirement(
                substate_scanner_ref.clone(),
                transaction_ref.clone(),
                requirement.clone(),
            ));
            handles.push(handle);
        }
        for handle in handles {
            let res = handle.await??;
            if let Some((address, substate)) = res {
                let shard = ShardId::from_address(&address, substate.version());
                input_shards.push(shard);
                found_substates.insert(address, substate);
            }
        }
        info!(target: LOG_TARGET, "✏️️ Found {} input substates", found_substates.len());
        autofilled_transaction.filled_inputs_mut().extend(input_shards);

        // let mut found_this_round = 0;

        const MAX_RECURSION: usize = 2;

        for _i in 0..MAX_RECURSION {
            // add all substates related to the inputs
            // TODO: we are going to only check the first level of recursion, for composability we may want to do it
            // recursively (with a recursion limit)
            let mut autofilled_inputs = vec![];
            let related_addresses: Vec<Vec<SubstateAddress>> = found_substates
                .values()
                .map(find_related_substates)
                .collect::<Result<_, _>>()?;

            info!(target: LOG_TARGET, "✏️️️ Found {} related substates", related_addresses.len());
            // exclude related substates that have been already included as requirement by the client
            let related_addresses = related_addresses
                .into_iter()
                .flatten()
                .filter(|s| !substate_requirements.iter().any(|r| r.address() == s));

            // we need to fetch (in parallel) the latest version of all the related substates
            let mut handles = HashMap::new();
            let substate_scanner_ref = self.substate_scanner.clone();
            for address in related_addresses {
                info!(target: LOG_TARGET, "✏️️️ Found {} related substates", address);
                let handle = tokio::spawn(get_substate(substate_scanner_ref.clone(), address.clone(), None));
                handles.insert(address.clone(), handle);
            }
            for (address, handle) in handles {
                let scan_res = handle.await??;

                if let SubstateResult::Up { substate, address, .. } = scan_res {
                    info!(
                        target: LOG_TARGET,
                        "✏️ Filling related substate {}:v{}",
                        address,
                        substate.version()
                    );
                    let shard = ShardId::from_address(&address, substate.version());
                    if autofilled_transaction.all_inputs_iter().any(|s| *s == shard) {
                        // Shard is already an input (TODO: what a waste)
                        continue;
                    }
                    autofilled_inputs.push(ShardId::from_address(&address, substate.version()));
                    found_substates.insert(address, substate);
                //       found_this_round += 1;
                } else {
                    warn!(
                        target: LOG_TARGET,
                        "✏️️ The related substate {} is not in UP status, skipping", address
                    );
                }
            }

            autofilled_transaction.filled_inputs_mut().extend(autofilled_inputs);
            //   if found_this_round == 0 {
            //      break;
            // }
        }

        Ok((autofilled_transaction, found_substates))
    }
}

pub async fn get_substate_requirement<TEpochManager, TVnClient, TAddr, TSubstateCache, TSignatureService>(
    substate_scanner: Arc<SubstateScanner<TEpochManager, TVnClient, TSubstateCache, TSignatureService>>,
    transaction: Arc<Transaction>,
    req: SubstateRequirement,
) -> Result<Option<(SubstateAddress, Substate)>, IndexerError>
where
    TEpochManager: EpochManagerReader<Addr = TAddr>,
    TVnClient: ValidatorNodeClientFactory<Addr = TAddr>,
    TAddr: DerivableFromPublicKey,
    TSubstateCache: SubstateCache,
    TSignatureService: VoteSignatureService,
{
    let scan_res = match req.version() {
        Some(version) => {
            let shard = ShardId::from_address(req.address(), version);
            if transaction.all_inputs_iter().any(|s| *s == shard) {
                // Shard is already an input
                return Ok(None);
            }

            // if the client specified a version, we need to retrieve it
            substate_scanner
                .get_specific_substate_from_committee(req.address(), version)
                .await?
        },
        None => {
            // if the client didn't specify a version, we fetch the latest one
            substate_scanner.get_substate(req.address(), None).await?
        },
    };

    if let SubstateResult::Up { substate, address, .. } = &scan_res {
        info!(
            target: LOG_TARGET,
            "Filling input substate {}:v{}",
            address,
            substate.version()
        );
        let shard = ShardId::from_address(address, substate.version());
        if transaction.all_inputs_iter().any(|s| *s == shard) {
            // Shard is already an input (TODO: what a waste)
            return Ok(None);
        }

        Ok(Some((address.clone(), substate.clone())))
    } else {
        warn!(
            target: LOG_TARGET,
            "🖋️ The substate for input requirement {} is not in UP status, skipping", req
        );
        Ok(None)
    }
}

pub async fn get_substate<TEpochManager, TVnClient, TAddr, TSubstateCache, TSignatureService>(
    substate_scanner: Arc<SubstateScanner<TEpochManager, TVnClient, TSubstateCache, TSignatureService>>,
    substate_address: SubstateAddress,
    version_hint: Option<u32>,
) -> Result<SubstateResult, IndexerError>
where
    TEpochManager: EpochManagerReader<Addr = TAddr>,
    TVnClient: ValidatorNodeClientFactory<Addr = TAddr>,
    TAddr: DerivableFromPublicKey,
    TSubstateCache: SubstateCache,
    TSignatureService: VoteSignatureService,
{
    substate_scanner.get_substate(&substate_address, version_hint).await
}
