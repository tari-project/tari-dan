//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, sync::Arc};

use tari_dan_common_types::ShardId;
use tari_engine_types::substate::{Substate, SubstateAddress};
use tari_transaction::{SubstateChange, Transaction};
use tari_validator_node_rpc::client::{SubstateResult, ValidatorNodeClientFactory};

use crate::{
    committee_provider::CommitteeProvider,
    error::IndexerError,
    substate_decoder::{find_related_substates, SubstateDecoderError},
    substate_scanner::SubstateScanner,
};

#[derive(Debug, thiserror::Error)]
pub enum TransactionAutofillerError {
    #[error("Could not decode the substate: {0}")]
    SubstateDecoderError(#[from] SubstateDecoderError),
    #[error("Indexer error: {0}")]
    IndexerError(#[from] IndexerError),
}

pub struct TransactionAutofiller<TCommitteeProvider, TVnClient> {
    substate_scanner: Arc<SubstateScanner<TCommitteeProvider, TVnClient>>,
}

impl<TCommitteeProvider, TVnClient> TransactionAutofiller<TCommitteeProvider, TVnClient>
where
    TCommitteeProvider: CommitteeProvider,
    TVnClient: ValidatorNodeClientFactory<Addr = TCommitteeProvider::Addr>,
    TCommitteeProvider::Addr: Display,
{
    pub fn new(substate_scanner: Arc<SubstateScanner<TCommitteeProvider, TVnClient>>) -> Self {
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
            let scan_res = self.substate_scanner.get_substate(r.address(), r.version()).await?;

            // TODO: should we return an error if some of the inputs are not "Up"?
            if let SubstateResult::Up { substate, .. } = scan_res {
                input_addresses.push((r.address().clone(), substate.version()));
                input_substates.push(substate);
            }
        }

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
        let related_addresses = related_addresses.into_iter().flatten();
        for address in related_addresses {
            // we need to fetch the latest version of all the related substates
            // note that if the version specified is "None", the scanner will fetch the latest version
            let scan_res = self.substate_scanner.get_substate(&address, None).await?;

            // TODO: should we return an error if some of the inputs are not "Up"?
            if let SubstateResult::Up { substate, .. } = scan_res {
                autofilled_inputs.push((address, substate.version()));
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
