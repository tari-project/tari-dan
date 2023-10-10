//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, sync::Arc};

use log::*;
use tari_dan_common_types::{NodeAddressable, ShardId};
use tari_engine_types::{
    indexed_value::IndexedValueVisitorError,
    substate::{Substate, SubstateAddress},
};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::{SubstateRequirement, Transaction};
use tari_validator_node_rpc::client::{SubstateResult, ValidatorNodeClientFactory};

use crate::{error::IndexerError, substate_decoder::find_related_substates, substate_scanner::SubstateScanner};

const LOG_TARGET: &str = "tari::indexer::transaction_autofiller";

#[derive(Debug, thiserror::Error)]
pub enum TransactionAutofillerError {
    #[error("Could not decode the substate: {0}")]
    IndexedValueVisitorError(#[from] IndexedValueVisitorError),
    #[error("Indexer error: {0}")]
    IndexerError(#[from] IndexerError),
}

pub struct TransactionAutofiller<TEpochManager, TVnClient> {
    substate_scanner: Arc<SubstateScanner<TEpochManager, TVnClient>>,
}

impl<TEpochManager, TVnClient, TAddr> TransactionAutofiller<TEpochManager, TVnClient>
where
    TEpochManager: EpochManagerReader<Addr = TAddr>,
    TVnClient: ValidatorNodeClientFactory<Addr = TAddr>,
    TAddr: NodeAddressable,
{
    pub fn new(substate_scanner: Arc<SubstateScanner<TEpochManager, TVnClient>>) -> Self {
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

        // scan the network to fetch all the substates for each required input
        // TODO: perform this loop concurrently by spawning a tokio task for each scan
        let mut input_shards = vec![];
        let mut found_substates = HashMap::new();
        for r in &substate_requirements {
            let scan_res = match r.version() {
                Some(version) => {
                    let shard = ShardId::from_address(r.address(), version);
                    if autofilled_transaction.all_inputs_iter().any(|s| *s == shard) {
                        // Shard is already an input
                        continue;
                    }

                    // if the client specified a version, we need to retrieve it
                    self.substate_scanner
                        .get_specific_substate_from_committee(r.address(), version)
                        .await?
                },
                None => {
                    // if the client didn't specify a version, we fetch the latest one
                    self.substate_scanner.get_substate(r.address(), None).await?
                },
            };

            if let SubstateResult::Up { substate, address, .. } = scan_res {
                info!(
                    target: LOG_TARGET,
                    "✏️Filling input substate {}:v{}",
                    address,
                    substate.version()
                );
                let shard = ShardId::from_address(&address, substate.version());
                if autofilled_transaction.all_inputs_iter().any(|s| *s == shard) {
                    // Shard is already an input (TODO: what a waste)
                    continue;
                }
                input_shards.push(shard);
                found_substates.insert(address, substate);
            } else {
                warn!(
                    target: LOG_TARGET,
                    "🖋️ The substate for input requirement {} is not in UP status, skipping", r
                );
            }
        }

        info!(target: LOG_TARGET, "✏️️ Found {} input substates", found_substates.len());
        autofilled_transaction.filled_inputs_mut().extend(input_shards);

        // let mut found_this_round = 0;

        const MAX_RECURSION: usize = 1;

        for _i in 0..MAX_RECURSION {
            // add all substates related to the inputs
            // TODO: perform this loop concurrently by spawning a tokio task for each scan
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

            for address in related_addresses {
                info!(target: LOG_TARGET, "✏️️️ Found {} related substate", address);

                // we need to fetch the latest version of all the related substates
                // note that if the version specified is "None", the scanner will fetch the latest version
                let scan_res = self.substate_scanner.get_substate(&address, None).await?;

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
