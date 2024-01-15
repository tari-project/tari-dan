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

use std::{collections::HashSet, fmt::Display, future::Future, iter, sync::Arc};

use log::*;
use tari_consensus::traits::VoteSignatureService;
use tari_dan_common_types::{
    optional::{IsNotFoundError, Optional},
    DerivableFromPublicKey,
    ShardId,
};
use tari_engine_types::substate::SubstateAddress;
use tari_epoch_manager::EpochManagerReader;
use tari_indexer_lib::{
    substate_cache::SubstateCache,
    substate_scanner::SubstateScanner,
    transaction_autofiller::TransactionAutofiller,
};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};
use tari_validator_node_rpc::client::{
    SubstateResult,
    TransactionResultStatus,
    ValidatorNodeClientFactory,
    ValidatorNodeRpcClient,
};

use crate::transaction_manager::error::TransactionManagerError;

const LOG_TARGET: &str = "tari::indexer::transaction_manager";

pub struct TransactionManager<TEpochManager, TClientFactory, TSubstateCache, TSignatureService> {
    epoch_manager: TEpochManager,
    client_provider: TClientFactory,
    transaction_autofiller: TransactionAutofiller<TEpochManager, TClientFactory, TSubstateCache, TSignatureService>,
}

impl<TEpochManager, TClientFactory, TAddr, TSubstateCache, TSignatureService>
    TransactionManager<TEpochManager, TClientFactory, TSubstateCache, TSignatureService>
where
    TAddr: DerivableFromPublicKey + 'static,
    TEpochManager: EpochManagerReader<Addr = TAddr> + 'static,
    TClientFactory: ValidatorNodeClientFactory<Addr = TAddr> + 'static,
    <TClientFactory::Client as ValidatorNodeRpcClient>::Error: IsNotFoundError + 'static,
    TSubstateCache: SubstateCache + 'static,
    TSignatureService: VoteSignatureService + Send + Sync + Clone + 'static,
{
    pub fn new(
        epoch_manager: TEpochManager,
        client_provider: TClientFactory,
        substate_scanner: Arc<SubstateScanner<TEpochManager, TClientFactory, TSubstateCache, TSignatureService>>,
    ) -> Self {
        Self {
            epoch_manager,
            client_provider,
            transaction_autofiller: TransactionAutofiller::new(substate_scanner),
        }
    }

    pub async fn submit_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionId, TransactionManagerError> {
        let tx_hash = *transaction.id();

        info!(
            target: LOG_TARGET,
            "Submitting transaction with hash {} to the validator node", tx_hash
        );
        // automatically scan the inputs and add all related involved objects
        // note that this operation does not alter the transaction hash
        let (autofilled_transaction, _) = self
            .transaction_autofiller
            .autofill_transaction(transaction, required_substates)
            .await?;

        let transaction_shard_id = ShardId::for_transaction_receipt(tx_hash.into_array().into());

        if autofilled_transaction.involved_shards_iter().count() == 0 {
            self.try_with_committee(iter::once(transaction_shard_id), |mut client| {
                let transaction = autofilled_transaction.clone();
                async move { client.submit_transaction(transaction).await }
            })
            .await
        } else {
            self.try_with_committee(autofilled_transaction.involved_shards_iter().copied(), |mut client| {
                let transaction = autofilled_transaction.clone();
                async move { client.submit_transaction(transaction).await }
            })
            .await
        }
    }

    pub async fn get_transaction_result(
        &self,
        transaction_id: TransactionId,
    ) -> Result<TransactionResultStatus, TransactionManagerError> {
        let transaction_shard_id = ShardId::for_transaction_receipt(transaction_id.into_array().into());
        self.try_with_committee(iter::once(transaction_shard_id), |mut client| async move {
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
        substate_address: SubstateAddress,
        version: u32,
    ) -> Result<SubstateResult, TransactionManagerError> {
        let shard = ShardId::from_address(&substate_address, version);

        self.try_with_committee(iter::once(shard), |mut client| {
            // This double clone looks strange, but it's needed because this function is called in a loop
            // and each iteration needs its own copy of the address (because of the move).
            let substate_address = substate_address.clone();
            async move {
                let substate_address = substate_address.clone();
                client
                    .get_substate(ShardId::from_address(&substate_address, version))
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
        shard_ids: IShard,
        mut callback: F,
    ) -> Result<T, TransactionManagerError>
    where
        F: FnMut(TClientFactory::Client) -> TFut,
        TClientFactory::Client: 'a,
        TFut: Future<Output = Result<T, E>> + 'a,
        T: 'static,
        E: Display,
        IShard: IntoIterator<Item = ShardId>,
    {
        let epoch = self.epoch_manager.current_epoch().await?;
        // Get all unique members. The hashset already "shuffles" items owing to the random hash function.
        let mut all_members = HashSet::new();
        for shard_id in shard_ids {
            let committee = self.epoch_manager.get_committee(epoch, shard_id).await?;
            all_members.extend(committee.into_addresses());
        }

        let committee_size = all_members.len();
        if committee_size == 0 {
            return Err(TransactionManagerError::NoCommitteeMembers);
        }

        for validator in all_members {
            let client = self.client_provider.create_client(&validator);
            match callback(client).await {
                Ok(ret) => {
                    return Ok(ret);
                },
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "Failed to dial validator node '{}': {}", validator, err
                    );
                    continue;
                },
            }
        }

        Err(TransactionManagerError::AllValidatorsFailed { committee_size })
    }
}
