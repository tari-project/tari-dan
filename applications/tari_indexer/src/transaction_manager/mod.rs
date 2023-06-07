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
use tari_dan_common_types::{
    optional::{IsNotFoundError, Optional},
    NodeAddressable, PayloadId, ShardId,
};
use tari_engine_types::substate::SubstateAddress;
use tari_epoch_manager::{base_layer::EpochManagerError, EpochManager};
use tari_indexer_lib::{substate_scanner::SubstateScanner, transaction_autofiller::TransactionAutofiller};
use tari_transaction::Transaction;
use tari_validator_node_rpc::client::{
    SubstateResult, TransactionResultStatus, ValidatorNodeClientFactory, ValidatorNodeRpcClient,
};

use crate::transaction_manager::error::TransactionManagerError;

const LOG_TARGET: &str = "tari::indexer::transaction_manager";

pub struct TransactionManager<TEpochManager, TClientFactory> {
    epoch_manager: TEpochManager,
    client_provider: TClientFactory,
    transaction_autofiller: TransactionAutofiller<TEpochManager, TClientFactory>,
}

impl<TEpochManager, TClientFactory, TAddr> TransactionManager<TEpochManager, TClientFactory>
where
    TAddr: NodeAddressable,
    TEpochManager: EpochManager<TAddr, Error = EpochManagerError>,
    TClientFactory: ValidatorNodeClientFactory<Addr = TAddr>,
    <TClientFactory::Client as ValidatorNodeRpcClient>::Error: IsNotFoundError,
{
    pub fn new(
        epoch_manager: TEpochManager,
        client_provider: TClientFactory,
        substate_scanner: Arc<SubstateScanner<TEpochManager, TClientFactory>>,
    ) -> Self {
        Self {
            epoch_manager,
            client_provider,
            transaction_autofiller: TransactionAutofiller::new(substate_scanner),
        }
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<PayloadId, TransactionManagerError> {
        let tx_hash = *transaction.hash();

        info!(
            target: LOG_TARGET,
            "Submitting transaction with hash {} to the validator node", tx_hash
        );
        // automatically scan the inputs and add all related involved objects
        // note that this operation does not alter the transaction hash
        let autofilled_transaction = self.transaction_autofiller.autofill_transaction(&transaction).await?;

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
        let epoch = self.epoch_manager.current_epoch().await?;
        let mut committee = self.epoch_manager.get_committee(epoch, shard_id).await?;

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
