//  Copyright 2023, The Tari Project
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

mod error;

use std::{fmt::Display, future::Future, sync::Arc};

use log::*;
use rand::{rngs::OsRng, seq::SliceRandom};
use tari_dan_app_utilities::epoch_manager::EpochManagerHandle;
use tari_dan_common_types::{
    optional::{IsNotFoundError, Optional},
    PayloadId,
    ShardId,
};
use tari_engine_types::substate::{SubstateAddress, Substate};
use tari_indexer_lib::{committee_provider::CommitteeProvider, substate_scanner::SubstateScanner};
use tari_transaction::{SubstateChange, Transaction};
use tari_validator_node_rpc::client::{
    SubstateResult,
    TariCommsValidatorNodeClientFactory,
    TransactionResultStatus,
    ValidatorNodeClientFactory,
    ValidatorNodeRpcClient,
};

use crate::{substate_decoder::find_related_substates, transaction_manager::error::TransactionManagerError};

const LOG_TARGET: &str = "tari::indexer::transaction_manager";

pub struct TransactionManager<TCommitteeProvider, TClientFactory> {
    store: TCommitteeProvider,
    client_provider: TClientFactory,
    substate_scanner: Arc<SubstateScanner<EpochManagerHandle, TariCommsValidatorNodeClientFactory>>,
}

impl<TCommitteeProvider, TClientFactory> TransactionManager<TCommitteeProvider, TClientFactory>
where
    TCommitteeProvider: CommitteeProvider,
    TCommitteeProvider::Addr: Display,
    TClientFactory: ValidatorNodeClientFactory<Addr = TCommitteeProvider::Addr>,
    <TClientFactory::Client as ValidatorNodeRpcClient>::Error: IsNotFoundError,
{
    pub fn new(
        store: TCommitteeProvider,
        client_provider: TClientFactory,
        substate_scanner: Arc<SubstateScanner<EpochManagerHandle, TariCommsValidatorNodeClientFactory>>,
    ) -> Self {
        Self {
            store,
            client_provider,
            substate_scanner,
        }
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<PayloadId, TransactionManagerError> {
        let tx_hash = *transaction.hash();

        // include the inputs and aoutputs into the "involved_objects" field
        // note that the transaction hash will not change as the "involved_objects" is not part of the hash
        let mut autofilled_transaction = transaction.clone();

        // scan the network to fetch all the substates for each required input
        // TODO: perform this loop concurrently by spawning a tokio task for each scan
        let mut input_substates : Vec<(SubstateAddress, u32, Substate)> = vec![];
        for r in transaction.meta().required_inputs() {
            // note that if the version specified is "None", the scanner will fetch the latest version
            let scan_res = self.substate_scanner.get_substate(r.address(), r.version()).await?;

            // TODO: should we return an error if some of the inputs are not "Up"?
            if let SubstateResult::Up { substate, .. } = scan_res {
                input_substates.push((r.address().clone(), substate.version(), substate));
            }
        }

        let input_shards: Vec<(ShardId, SubstateChange)> = input_substates
            .iter()
            .map(|s| {
                let shard_id = ShardId::from_address(&s.0, s.1);
                (shard_id, SubstateChange::Exists)
            })
            .collect();
        autofilled_transaction
            .meta_mut()
            .involved_objects_mut()
            .extend(input_shards);


        // get all related substates related to the inputs the inputs
        // TODO: perform this loop concurrently by spawning a tokio task for each scan
        // TODO: we are going to only check the first level of recursion, for composability we may want to do it
        // recursively (with a recursion limit)
        let mut autofilled_inputs: Vec<(SubstateAddress, u32)> = vec![];
        let related_addresses: Vec<Vec<SubstateAddress>> = input_substates
            .iter()
            .map(|s| find_related_substates(&s.2))
            .collect::<Result<_, _>>()?;
        let related_addresses = related_addresses.into_iter().flatten();
        for address in related_addresses {
            // we need to fetch the latest version of all the related substates
            let scan_res = self.substate_scanner.get_substate(&address, None).await?;

            // TODO: should we return an error if some of the inputs are not "Up"?
            if let SubstateResult::Up { substate, .. } = scan_res {
                autofilled_inputs.push((address, substate.version()));
            }
        }

        // add add all inputs into involved objects
        // calculate the shard ids for each autofilled input
        let autofilled_input_objects: Vec<(ShardId, SubstateChange)> = autofilled_inputs
            .iter()
            .map(|i| {
                let shard_id = ShardId::from_address(&i.0, i.1);
                (shard_id, SubstateChange::Exists)
            })
            .collect();
        autofilled_transaction
            .meta_mut()
            .involved_objects_mut()
            .extend(autofilled_input_objects);

        // add the expected outputs into involved objects
        // TODO: we assume that all inputs will be consumed and produce a new output
        // however this is only the case when the object is mutated
        let autofilled_output_objects: Vec<(ShardId, SubstateChange)> = autofilled_inputs
            .iter()
            .map(|i| {
                let shard_id = ShardId::from_address(&i.0, i.1 + 1);
                (shard_id, SubstateChange::Create)
            })
            .collect();
        autofilled_transaction
            .meta_mut()
            .involved_objects_mut()
            .extend(autofilled_output_objects);
        

        self.try_with_committee(tx_hash.into_array().into(), move |mut client| {
            let transaction = autofilled_transaction.clone();
            async move { client.submit_transaction(transaction).await }
        })
        .await
    }

    pub async fn get_transaction_result(
        &self,
        payload_id: PayloadId,
    ) -> Result<TransactionResultStatus, TransactionManagerError> {
        self.try_with_committee(payload_id.into_array().into(), |mut client| async move {
            client.get_finalized_transaction_result(payload_id).await.optional()
        })
        .await?
        .ok_or_else(|| TransactionManagerError::NotFound {
            entity: "Transaction result",
            key: payload_id.to_string(),
        })
    }

    pub async fn get_substate(
        &self,
        substate_address: SubstateAddress,
        version: u32,
    ) -> Result<SubstateResult, TransactionManagerError> {
        let shard = ShardId::from_address(&substate_address, version);

        self.try_with_committee(shard, |mut client| {
            // This double clone looks strange, but it's needed because this function is called in a loop
            // and each iteration needs its own copy of the address (because of the move).
            let substate_address = substate_address.clone();
            async move {
                let substate_address = substate_address.clone();
                client.get_substate(&substate_address, version).await
            }
        })
        .await
    }

    /// Fetches the committee members for the given shard and calls the given callback with each member until
    /// the callback returns a `Ok` result. If the callback returns an `Err` result, the next committee member is
    /// called.
    async fn try_with_committee<F, T, E, TFut>(
        &self,
        shard_id: ShardId,
        mut callback: F,
    ) -> Result<T, TransactionManagerError>
    where
        F: FnMut(TClientFactory::Client) -> TFut,
        TFut: Future<Output = Result<T, E>>,
        E: Display,
    {
        let mut committee = self
            .store
            .get_committee(shard_id)
            .await
            .map_err(|e| TransactionManagerError::CommitteeProviderError(e.to_string()))?;

        committee.members.shuffle(&mut OsRng);

        let committee_size = committee.members.len();
        if committee_size == 0 {
            return Err(TransactionManagerError::NoCommitteeMembers);
        }

        for validator in committee.members {
            let client = self.client_provider.create_client(&validator);
            match callback(client).await {
                Ok(ret) => {
                    return Ok(ret);
                },
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "Failed to call validator node '{}': {}", validator, err
                    );
                    continue;
                },
            }
        }

        Err(TransactionManagerError::AllValidatorsFailed { committee_size })
    }
}
