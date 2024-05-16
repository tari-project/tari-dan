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
use tari_common::configuration::Network;
use tari_dan_app_utilities::{
    template_manager::implementation::TemplateManager,
    transaction_executor::{TariDanTransactionProcessor, TransactionExecutor as _},
};
use tari_dan_common_types::{Epoch, PeerAddress, SubstateAddress};
use tari_dan_engine::{
    bootstrap_state,
    fees::FeeTable,
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
};
use tari_engine_types::{
    commit_result::ExecuteResult,
    instruction::Instruction,
    substate::{Substate, SubstateId},
    virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates},
};
use tari_epoch_manager::{base_layer::EpochManagerHandle, EpochManagerReader};
use tari_indexer_lib::{
    substate_cache::SubstateCache,
    substate_scanner::SubstateScanner,
    transaction_autofiller::TransactionAutofiller,
};
use tari_transaction::{SubstateRequirement, Transaction};
use tari_validator_node_rpc::client::{
    SubstateResult,
    TariValidatorNodeRpcClientFactory,
    ValidatorNodeClientFactory,
    ValidatorNodeRpcClient,
};
use tokio::task;

use crate::dry_run::error::DryRunTransactionProcessorError;

const LOG_TARGET: &str = "tari::indexer::dry_run_transaction_processor";

pub struct DryRunTransactionProcessor<TSubstateCache> {
    epoch_manager: EpochManagerHandle<PeerAddress>,
    client_provider: TariValidatorNodeRpcClientFactory,
    transaction_autofiller:
        TransactionAutofiller<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, TSubstateCache>,
    template_manager: TemplateManager<PeerAddress>,
    substate_scanner:
        Arc<SubstateScanner<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, TSubstateCache>>,
    network: Network,
}

impl<TSubstateCache> DryRunTransactionProcessor<TSubstateCache>
where TSubstateCache: SubstateCache + 'static
{
    pub fn new(
        epoch_manager: EpochManagerHandle<PeerAddress>,
        client_provider: TariValidatorNodeRpcClientFactory,
        substate_scanner: Arc<
            SubstateScanner<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, TSubstateCache>,
        >,
        template_manager: TemplateManager<PeerAddress>,
        network: Network,
    ) -> Self {
        let transaction_autofiller = TransactionAutofiller::new(substate_scanner.clone());

        Self {
            epoch_manager,
            client_provider,
            transaction_autofiller,
            template_manager,
            substate_scanner,
            network,
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

        let virtual_substates = self.get_virtual_substates(&transaction, epoch).await?;

        let state_store = new_state_store();
        state_store.set_many(found_substates)?;

        // execute the payload in the WASM engine and return the result
        let result = task::block_in_place(|| payload_processor.execute(transaction, state_store, virtual_substates))?;

        Ok(result.into_result())
    }

    fn build_payload_processor(
        &self,
        transaction: &Transaction,
    ) -> TariDanTransactionProcessor<TemplateManager<PeerAddress>> {
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

        TariDanTransactionProcessor::new(self.network, self.template_manager.clone(), fee_table)
    }

    fn transaction_includes_fees(transaction: &Transaction) -> bool {
        !transaction.fee_instructions().is_empty()
    }

    async fn fetch_input_substates(
        &self,
        transaction: &Transaction,
        epoch: Epoch,
    ) -> Result<HashMap<SubstateId, Substate>, DryRunTransactionProcessorError> {
        let mut substates = HashMap::new();

        // Fetch explicit inputs that may not have been resolved by the autofiller
        for requirement in transaction.inputs() {
            let Some(address) = requirement.to_substate_address() else {
                // No version, we cant fetch it
                continue;
            };
            // If the input has been filled, we've already fetched the substate
            // Note: this works because VersionedSubstateId hashes the same as SubstateId internally.
            if transaction.filled_inputs().contains(&requirement.substate_id) {
                continue;
            }

            let (id, substate) = self.fetch_substate(address, epoch).await?;
            substates.insert(id, substate);
        }

        Ok(substates)
    }

    pub async fn fetch_substate(
        &self,
        address: SubstateAddress,
        epoch: Epoch,
    ) -> Result<(SubstateId, Substate), DryRunTransactionProcessorError> {
        let mut committee = self.epoch_manager.get_committee_for_substate(epoch, address).await?;
        committee.shuffle();

        let mut nexist_count = 0;
        let mut err_count = 0;

        for vn_addr in committee.addresses() {
            // build a client with the VN
            let mut client = self.client_provider.create_client(vn_addr);

            match client.get_substate(address).await {
                Ok(SubstateResult::Up { substate, id, .. }) => {
                    return Ok((id, substate));
                },
                Ok(SubstateResult::Down { id, version, .. }) => {
                    // TODO: we should seek proof of this.
                    return Err(DryRunTransactionProcessorError::SubstateDowned { id, version });
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
            address,
            epoch,
            nexist_count,
            err_count,
            committee_size: committee.members().count(),
        })
    }

    async fn get_virtual_substates(
        &self,
        transaction: &Transaction,
        epoch: Epoch,
    ) -> Result<VirtualSubstates, DryRunTransactionProcessorError> {
        let mut virtual_substates = VirtualSubstates::new();

        virtual_substates.insert(
            VirtualSubstateId::CurrentEpoch,
            VirtualSubstate::CurrentEpoch(epoch.as_u64()),
        );

        let claim_instructions = transaction
            .instructions()
            .iter()
            .chain(transaction.fee_instructions())
            .filter_map(|instruction| {
                if let Instruction::ClaimValidatorFees {
                    epoch,
                    validator_public_key,
                } = instruction
                {
                    Some((Epoch(*epoch), validator_public_key.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if !claim_instructions.is_empty() {
            for (epoch, public_key) in claim_instructions {
                let vn = self
                    .epoch_manager
                    .get_validator_node_by_public_key(epoch, &public_key)
                    .await?;
                let address = VirtualSubstateId::UnclaimedValidatorFee {
                    epoch: epoch.as_u64(),
                    address: public_key,
                };
                let virtual_substate = self
                    .substate_scanner
                    .get_virtual_substate_from_committee(address.clone(), vn.shard_key)
                    .await?;
                virtual_substates.insert(address, virtual_substate);
            }
        }

        Ok(virtual_substates)
    }
}

fn new_state_store() -> MemoryStateStore {
    let state_store = MemoryStateStore::new();
    let mut tx = state_store.write_access().unwrap();
    bootstrap_state(&mut tx).unwrap();
    tx.commit().unwrap();
    state_store
}
