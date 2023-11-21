//  Copyright 2023. The Tari Project
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

use std::{collections::HashMap, sync::Arc};

use log::info;
use tari_comms::types::CommsPublicKey;
use tari_dan_app_utilities::{
    template_manager::implementation::TemplateManager,
    transaction_executor::{TariDanTransactionProcessor, TransactionExecutor},
};
use tari_dan_common_types::{optional::IsNotFoundError, Epoch, ShardId};
use tari_dan_engine::{
    bootstrap_state,
    fees::FeeTable,
    runtime::VirtualSubstates,
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
};
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateAddress},
    virtual_substate::{VirtualSubstate, VirtualSubstateAddress},
};
use tari_epoch_manager::EpochManagerReader;
use tari_indexer_lib::{substate_scanner::SubstateScanner, transaction_autofiller::TransactionAutofiller, substate_cache::SubstateCache};
use tari_transaction::{SubstateRequirement, Transaction};
use tari_validator_node_rpc::client::{SubstateResult, ValidatorNodeClientFactory, ValidatorNodeRpcClient};
use tokio::task;

use crate::dry_run::error::DryRunTransactionProcessorError;

const LOG_TARGET: &str = "tari::indexer::dry_run_transaction_processor";

pub struct DryRunTransactionProcessor<TEpochManager, TClientFactory, TSubstateCache> {
    epoch_manager: TEpochManager,
    client_provider: TClientFactory,
    transaction_autofiller: TransactionAutofiller<TEpochManager, TClientFactory, TSubstateCache>,
    template_manager: TemplateManager,
}

impl<TEpochManager, TClientFactory, TSubstateCache> DryRunTransactionProcessor<TEpochManager, TClientFactory, TSubstateCache>
where
    TEpochManager: EpochManagerReader<Addr = CommsPublicKey> + 'static,
    TClientFactory: ValidatorNodeClientFactory<Addr = CommsPublicKey> + 'static,
    <TClientFactory::Client as ValidatorNodeRpcClient>::Error: IsNotFoundError,
    TSubstateCache: SubstateCache + 'static,
{
    pub fn new(
        epoch_manager: TEpochManager,
        client_provider: TClientFactory,
        substate_scanner: Arc<SubstateScanner<TEpochManager, TClientFactory, TSubstateCache>>,
        template_manager: TemplateManager,
    ) -> Self {
        let transaction_autofiller = TransactionAutofiller::new(substate_scanner);

        Self {
            epoch_manager,
            client_provider,
            transaction_autofiller,
            template_manager,
        }
    }

    pub async fn process_transaction(
        &self,
        transaction: Transaction,
        substate_requirements: Vec<SubstateRequirement>,
    ) -> Result<ExecuteResult, DryRunTransactionProcessorError> {
        info!(target: LOG_TARGET, "process_transaction: {}", transaction.hash());

        // automatically scan the inputs and add all related involved objects
        // note that this operation does not alter the transaction hash
        let (transaction, mut found_substates) = self
            .transaction_autofiller
            .autofill_transaction(transaction, substate_requirements)
            .await?;

        let epoch = self.epoch_manager.current_epoch().await?;
        found_substates.extend(self.fetch_input_substates(&transaction, epoch).await?);

        let payload_processor = self.build_payload_processor(&transaction);

        let virtual_substates = Self::get_virtual_substates(epoch);

        let mut state_store = new_state_store();
        state_store.extend(found_substates);

        // execute the payload in the WASM engine and return the result
        let result = task::block_in_place(|| payload_processor.execute(transaction, state_store, virtual_substates))?;

        Ok(result.into_result())
    }

    fn build_payload_processor(&self, transaction: &Transaction) -> TariDanTransactionProcessor<TemplateManager> {
        // simulate fees if the transaction requires it
        let fee_table = if Self::transaction_includes_fees(transaction) {
            // TODO: should match the VN fee table, should the fee table values be a consensus constant?
            FeeTable {
                per_module_call_cost: 1,
                per_byte_storage_cost: 1,
                per_event_cost: 1,
                per_log_cost: 1,
            }
        } else {
            FeeTable::zero_rated()
        };

        TariDanTransactionProcessor::new(self.template_manager.clone(), fee_table)
    }

    fn transaction_includes_fees(transaction: &Transaction) -> bool {
        !transaction.fee_instructions().is_empty()
    }

    async fn fetch_input_substates(
        &self,
        transaction: &Transaction,
        epoch: Epoch,
    ) -> Result<HashMap<SubstateAddress, Substate>, DryRunTransactionProcessorError> {
        let mut substates = HashMap::new();

        for shard_id in transaction.inputs().iter().chain(transaction.input_refs()) {
            // If the input has been filled, we've already fetched the substate
            if transaction.filled_inputs().contains(shard_id) {
                continue;
            }

            let (address, substate) = self.fetch_substate(*shard_id, epoch).await?;
            substates.insert(address, substate);
        }

        Ok(substates)
    }

    pub async fn fetch_substate(
        &self,
        shard_id: ShardId,
        epoch: Epoch,
    ) -> Result<(SubstateAddress, Substate), DryRunTransactionProcessorError> {
        let mut committee = self.epoch_manager.get_committee(epoch, shard_id).await?;
        committee.shuffle();

        let mut nexist_count = 0;
        let mut err_count = 0;

        for vn_public_key in &committee {
            // build a client with the VN
            let mut client = self.client_provider.create_client(vn_public_key);

            match client.get_substate(shard_id).await {
                Ok(SubstateResult::Up { substate, address, .. }) => {
                    return Ok((address, substate));
                },
                Ok(SubstateResult::Down { address, version, .. }) => {
                    // TODO: we should seek proof of this.
                    return Err(DryRunTransactionProcessorError::SubstateDowned { address, version });
                },
                Ok(SubstateResult::DoesNotExist) => {
                    // we do not stop when an individual claims DoesNotExist, we try all Vns
                    nexist_count += 1;
                    continue;
                },
                Err(e) => {
                    info!(target: LOG_TARGET, "Unable to get pledge from peer: {} ", e.to_string());
                    // we do not stop when an individual request errors, we try all Vns
                    err_count += 1;
                    continue;
                },
            };
        }

        // The substate does not exist on any VN or all validator nodes are offline, we return an error
        Err(DryRunTransactionProcessorError::AllValidatorsFailedToReturnSubstate {
            shard_id,
            epoch,
            nexist_count,
            err_count,
            committee_size: committee.members().len(),
        })
    }

    fn get_virtual_substates(epoch: Epoch) -> VirtualSubstates {
        let mut virtual_substates = VirtualSubstates::new();

        virtual_substates.insert(
            VirtualSubstateAddress::CurrentEpoch,
            VirtualSubstate::CurrentEpoch(epoch.as_u64()),
        );

        virtual_substates
    }
}

fn new_state_store() -> MemoryStateStore {
    let state_store = MemoryStateStore::new();
    let mut tx = state_store.write_access().unwrap();
    bootstrap_state(&mut tx).unwrap();
    tx.commit().unwrap();
    state_store
}
