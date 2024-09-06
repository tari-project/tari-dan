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

pub(crate) mod error;

use std::{collections::HashSet, fmt::Display, future::Future, iter, sync::Arc};

use log::*;
use tari_dan_common_types::{
    optional::{IsNotFoundError, Optional},
    NodeAddressable,
    SubstateAddress,
    SubstateRequirement,
    ToSubstateAddress,
};
use tari_engine_types::substate::SubstateId;
use tari_epoch_manager::EpochManagerReader;
use tari_indexer_lib::{
    substate_cache::SubstateCache,
    substate_scanner::SubstateScanner,
    transaction_autofiller::TransactionAutofiller,
};
use tari_transaction::{Transaction, TransactionId};
use tari_validator_node_rpc::client::{
    SubstateResult,
    TransactionResultStatus,
    ValidatorNodeClientFactory,
    ValidatorNodeRpcClient,
};

use crate::transaction_manager::error::TransactionManagerError;

const LOG_TARGET: &str = "tari::indexer::transaction_manager";

pub struct TransactionManager<TEpochManager, TClientFactory, TSubstateCache> {
    epoch_manager: TEpochManager,
    client_provider: TClientFactory,
    transaction_autofiller: TransactionAutofiller<TEpochManager, TClientFactory, TSubstateCache>,
}

impl<TEpochManager, TClientFactory, TAddr, TSubstateCache>
    TransactionManager<TEpochManager, TClientFactory, TSubstateCache>
where
    TAddr: NodeAddressable + 'static,
    TEpochManager: EpochManagerReader<Addr = TAddr> + 'static,
    TClientFactory: ValidatorNodeClientFactory<Addr = TAddr> + 'static,
    <TClientFactory::Client as ValidatorNodeRpcClient>::Error: IsNotFoundError + 'static,
    TSubstateCache: SubstateCache + 'static,
{
    pub fn new(
        epoch_manager: TEpochManager,
        client_provider: TClientFactory,
        substate_scanner: Arc<SubstateScanner<TEpochManager, TClientFactory, TSubstateCache>>,
    ) -> Self {
        Self {
            epoch_manager,
            client_provider,
            transaction_autofiller: TransactionAutofiller::new(substate_scanner),
        }
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<TransactionId, TransactionManagerError> {
        let tx_hash = *transaction.id();

        info!(
            target: LOG_TARGET,
            "Submitting transaction with hash {} to the validator node", tx_hash
        );
        let transaction_substate_address = SubstateAddress::for_transaction_receipt(tx_hash.into_array().into());

        if transaction.all_inputs_iter().next().is_none() {
            self.try_with_committee(iter::once(transaction_substate_address), 2, |mut client| {
                let transaction = transaction.clone();
                async move { client.submit_transaction(transaction).await }
            })
            .await
        } else {
            let involved = transaction
                .all_inputs_iter()
                // If there is no version specified, submit to the validator node with version 0
                .map(|i| i.or_zero_version().to_substate_address());
            self.try_with_committee(involved, 2, |mut client| {
                let transaction = transaction.clone();
                async move { client.submit_transaction(transaction).await }
            })
            .await
        }
    }

    pub async fn autofill_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<Transaction, TransactionManagerError> {
        let (transaction, _) = self
            .transaction_autofiller
            .autofill_transaction(transaction, required_substates)
            .await?;
        Ok(transaction)
    }

    pub async fn get_transaction_result(
        &self,
        transaction_id: TransactionId,
    ) -> Result<TransactionResultStatus, TransactionManagerError> {
        let transaction_substate_address = SubstateAddress::for_transaction_receipt(transaction_id.into_array().into());
        self.try_with_committee(iter::once(transaction_substate_address), 1, |mut client| async move {
            client.get_finalized_transaction_result(transaction_id).await.optional()
        })
        .await?
        .ok_or_else(|| TransactionManagerError::NotFound {
            entity: "Transaction result",
            key: transaction_id.to_string(),
        })
    }

    pub async fn get_substate(
        &self,
        substate_address: SubstateId,
        version: u32,
    ) -> Result<SubstateResult, TransactionManagerError> {
        let shard = SubstateAddress::from_substate_id(&substate_address, version);

        self.try_with_committee(iter::once(shard), 1, |mut client| {
            // This double clone looks strange, but it's needed because this function is called in a loop
            // and each iteration needs its own copy of the address (because of the move).
            let substate_address = substate_address.clone();
            async move {
                let substate_address = substate_address.clone();
                client
                    .get_substate(SubstateAddress::from_substate_id(&substate_address, version))
                    .await
            }
        })
        .await
    }

    /// Fetches the committee members for the given shard and calls the given callback with each member until
    /// the callback returns a `Ok` result. If the callback returns an `Err` result, the next committee member is
    /// called.
    async fn try_with_committee<'a, F, T, E, TFut, IShard>(
        &self,
        substate_addresses: IShard,
        mut num_to_query: usize,
        mut callback: F,
    ) -> Result<T, TransactionManagerError>
    where
        F: FnMut(TClientFactory::Client) -> TFut,
        TClientFactory::Client: 'a,
        TFut: Future<Output = Result<T, E>> + 'a,
        T: 'static,
        E: Display,
        IShard: IntoIterator<Item = SubstateAddress>,
    {
        let epoch = self.epoch_manager.current_epoch().await?;
        // Get all unique members. The hashset already "shuffles" items owing to the random hash function.
        let mut all_members = HashSet::new();
        for substate_address in substate_addresses {
            let committee = self
                .epoch_manager
                .get_committee_for_substate(epoch, substate_address)
                .await?;
            all_members.extend(committee.into_addresses());
        }

        let committee_size = all_members.len();
        if committee_size == 0 {
            return Err(TransactionManagerError::NoCommitteeMembers);
        }

        let mut num_succeeded = 0;
        let mut last_error = None;
        let mut last_return = None;
        for validator in all_members {
            let client = self.client_provider.create_client(&validator);
            match callback(client).await {
                Ok(ret) => {
                    num_to_query = num_to_query.saturating_sub(1);
                    num_succeeded += 1;
                    last_return = Some(ret);
                    if num_to_query == 0 {
                        break;
                    }
                },
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "Request failed for validator '{}': {}", validator, err
                    );
                    last_error = Some(err.to_string());
                },
            }
        }

        if num_succeeded == 0 {
            return Err(TransactionManagerError::AllValidatorsFailed {
                committee_size,
                last_error,
            });
        }

        Ok(last_return.expect("last_return must be Some if num_succeeded > 0"))
    }
}
