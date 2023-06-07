//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use log::*;
use tari_dan_common_types::{NodeAddressable, ShardId};
use tari_engine_types::{
    indexed_value::IndexedValueVisitorError,
    substate::{Substate, SubstateAddress},
};
use tari_epoch_manager::{base_layer::EpochManagerError, EpochManager};
use tari_transaction::{SubstateChange, Transaction};
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
    TEpochManager: EpochManager<TAddr, Error = EpochManagerError>,
    TVnClient: ValidatorNodeClientFactory<Addr = TAddr>,
    TAddr: NodeAddressable,
{
    pub fn new(substate_scanner: Arc<SubstateScanner<TEpochManager, TVnClient>>) -> Self {
        Self { substate_scanner }
    }

    pub async fn autofill_transaction(
        &self,
        original_transaction: &Transaction,
    ) -> Result<Transaction, TransactionAutofillerError> {
        // we will include the inputs and outputs into the "involved_objects" field
        // note that the transaction hash will not change as the "involved_objects" is not part of the hash
        let mut autofilled_transaction = original_transaction.clone();

        // scan the network to fetch all the substates for each required input
        // TODO: perform this loop concurrently by spawning a tokio task for each scan
        let mut input_addresses: Vec<(SubstateAddress, u32)> = vec![];
        let mut input_substates: Vec<Substate> = vec![];
        for r in autofilled_transaction.meta().required_inputs() {
            let scan_res = match r.version() {
                Some(version) => {
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

            if let SubstateResult::Up { substate, .. } = scan_res {
                input_addresses.push((r.address().clone(), substate.version()));
                input_substates.push(substate);
            } else {
                warn!(
                    target: LOG_TARGET,
                    "🖋️ The substate for input requirement {} is not in UP status, skipping", r
                );
            }
        }

        info!(target: LOG_TARGET, "🖋️ Found {} input substates", input_substates.len());
        Self::add_involved_objects(&mut autofilled_transaction, &input_addresses, SubstateChange::Exists);

        // add all substates related to the inputs
        // TODO: perform this loop concurrently by spawning a tokio task for each scan
        // TODO: we are going to only check the first level of recursion, for composability we may want to do it
        // recursively (with a recursion limit)
        let mut autofilled_inputs: Vec<(SubstateAddress, u32)> = vec![];
        let related_addresses: Vec<Vec<SubstateAddress>> = input_substates
            .iter()
            .map(find_related_substates)
            .collect::<Result<_, _>>()?;
        // exclude related substates that have been already included as requirement by the client
        let related_addresses = related_addresses
            .into_iter()
            .flatten()
            .filter(|s| !original_transaction.meta().includes_substate(s));

        if let (_, Some(size)) = related_addresses.size_hint() {
            info!(target: LOG_TARGET, "🖋️ Found {} related substates", size);
        }

        for address in related_addresses {
            info!(target: LOG_TARGET, "🖋️ Found {} related substate", address);

            // we need to fetch the latest version of all the related substates
            // note that if the version specified is "None", the scanner will fetch the latest version
            let scan_res = self.substate_scanner.get_substate(&address, None).await?;

            if let SubstateResult::Up { substate, .. } = scan_res {
                info!(
                    target: LOG_TARGET,
                    "Adding related substate {}:v{}",
                    address,
                    substate.version()
                );
                autofilled_inputs.push((address, substate.version()));
            } else {
                warn!(
                    target: LOG_TARGET,
                    "🖋️ The related substate {} is not in UP status, skipping", address
                );
            }
        }
        Self::add_involved_objects(&mut autofilled_transaction, &autofilled_inputs, SubstateChange::Exists);

        // add the expected outputs into involved objects
        // TODO: we assume that all inputs will be consumed and produce a new output
        // however this is only the case when the object is mutated
        let autofilled_outputs: Vec<(SubstateAddress, u32)> =
            autofilled_inputs.into_iter().map(|i| (i.0, i.1 + 1)).collect();
        Self::add_involved_objects(&mut autofilled_transaction, &autofilled_outputs, SubstateChange::Create);

        Ok(autofilled_transaction)
    }

    fn add_involved_objects(
        transaction: &mut Transaction,
        versioned_addresses: &[(SubstateAddress, u32)],
        change: SubstateChange,
    ) {
        let new_objects: Vec<(ShardId, SubstateChange)> = versioned_addresses
            .iter()
            .map(|i| {
                let shard_id = ShardId::from_address(&i.0, i.1);
                (shard_id, change)
            })
            .collect();
        transaction.meta_mut().involved_objects_mut().extend(new_objects);
    }
}
