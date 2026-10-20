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

use std::{collections::HashMap, sync::Arc, time::Duration};

use log::{debug, info};
use ootle_network::Network;
use tari_engine::{fees::FeeTable, state_store::new_memory_store, traits::ClaimProofVerifier, wasm::WasmModuleCache};
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateId},
    virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates},
};
use tari_epoch_manager::{EpochManagerReader, service::EpochManagerHandle};
use tari_ootle_app_utilities::transaction_executor::{TariTransactionProcessor, TransactionExecutor as _};
use tari_ootle_common_types::{Epoch, SubstateRequirementRef};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_template_provider::TemplateConfig;
use tari_ootle_transaction::Transaction;
use tari_ootle_transaction_validation::{Validator, create_unsigned_structural_transaction_validator};
use tari_template_lib_types::constants::TARI_TOKEN;
use tokio::{runtime::Handle, sync::Semaphore, task};

use crate::{
    dry_run::{
        error::DryRunTransactionProcessorError,
        template_provider::{DryRunTemplateProvider, build_dry_run_template_provider},
    },
    exhaust_burn_rate::resolve_exhaust_burn_rate_for_epoch,
    network_state_sync::ConsensusEpoch,
    substate_manager::SubstateManager,
};

const LOG_TARGET: &str = "tari::indexer::dry_run_transaction_processor";

/// How long a dry run waits for an execution slot before it is refused. Long enough to ride out a
/// burst of wallet estimates, short enough that a saturated node answers rather than piling up
/// requests.
const EXECUTION_SLOT_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct DryRunTransactionProcessor {
    network: Network,
    fee_table: FeeTable,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    consensus_epoch: ConsensusEpoch,
    template_provider: DryRunTemplateProvider,
    substate_manager: SubstateManager,
    claim_burn_proof_verifier: Arc<dyn ClaimProofVerifier + Send + Sync + 'static>,
    max_transaction_weight: u64,
    max_transaction_size_bytes: usize,
    execution_slots: Arc<Semaphore>,
}

impl DryRunTransactionProcessor {
    pub fn new(
        network: Network,
        fee_table: FeeTable,
        epoch_manager: EpochManagerHandle<PeerAddress>,
        consensus_epoch: ConsensusEpoch,
        substate_manager: SubstateManager,
        wasm_cache: WasmModuleCache,
        template_config: &TemplateConfig,
        claim_burn_proof_verifier: impl ClaimProofVerifier + Send + Sync + 'static,
        max_transaction_weight: u64,
        max_transaction_size_bytes: usize,
        max_concurrent_executions: usize,
    ) -> Result<Self, std::io::Error> {
        let handle = Handle::try_current().map_err(std::io::Error::other)?;
        let template_provider =
            build_dry_run_template_provider(handle, substate_manager.clone(), wasm_cache, template_config);
        Ok(Self {
            network,
            fee_table,
            epoch_manager,
            consensus_epoch,
            template_provider,
            substate_manager,
            claim_burn_proof_verifier: Arc::new(claim_burn_proof_verifier),
            max_transaction_weight,
            max_transaction_size_bytes,
            execution_slots: Arc::new(Semaphore::new(max_concurrent_executions.max(1))),
        })
    }

    pub async fn process_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<ExecuteResult, DryRunTransactionProcessorError> {
        if !transaction.is_dry_run() {
            return Err(DryRunTransactionProcessorError::NonDryRunTransaction);
        }

        info!(target: LOG_TARGET, "process_transaction: {}", transaction.calculate_id());

        create_unsigned_structural_transaction_validator(
            self.network,
            self.max_transaction_weight,
            self.max_transaction_size_bytes,
        )
        .validate(&(), &transaction)?;

        let mut found_substates = self.fetch_input_substates(&transaction).await?;
        // Add the TARI resource - this is what consensus does, so we'll need to do it for dry runs
        let id = TARI_TOKEN.into();
        if !found_substates.contains_key(&id) {
            let tari_token = self
                .substate_manager
                .get_substate(SubstateRequirementRef::unversioned(&id))
                .await?;
            found_substates.insert(TARI_TOKEN.into(), tari_token);
        }

        let epoch = self.execution_epoch().await?;
        let virtual_substates = self.get_virtual_substates(epoch).await?;

        // Estimate the burn at the rate in effect for the epoch the transaction executes in.
        let burn_rate = resolve_exhaust_burn_rate_for_epoch(&self.substate_manager, self.network, epoch).await;

        let mut state_store = new_memory_store();
        state_store.set_many(found_substates)?;

        // execute the payload in the WASM engine and return the result
        let processor = TariTransactionProcessor::new(
            self.network,
            self.fee_table.clone(),
            true,
            self.claim_burn_proof_verifier.clone(),
        );
        let template_provider = self.template_provider.clone();
        let slot = tokio::time::timeout(EXECUTION_SLOT_WAIT, self.execution_slots.clone().acquire_owned())
            .await
            .map_err(|_| {
                debug!(target: LOG_TARGET, "Refusing dry run: every execution slot stayed busy for {EXECUTION_SLOT_WAIT:?}");
                DryRunTransactionProcessorError::Busy
            })?
            .expect("execution_slots is never closed");
        let exec_output = task::spawn_blocking(move || {
            let _slot = slot;
            processor.execute(
                &transaction,
                template_provider,
                state_store.into_read_only(),
                virtual_substates,
                burn_rate,
            )
        })
        .await??;

        Ok(exec_output.result)
    }

    async fn fetch_input_substates(
        &self,
        transaction: &Transaction,
    ) -> Result<HashMap<SubstateId, Substate>, DryRunTransactionProcessorError> {
        let substates = self
            .substate_manager
            .get_input_substates(
                transaction
                    .inputs()
                    .iter()
                    .map(|req| req.as_ref().to_substate_requirement_ref()),
            )
            .await?;
        Ok(substates
            .into_iter()
            .map(|(id, fetched)| (id, fetched.substate))
            .collect())
    }

    /// The epoch the validators would execute the transaction in. Until the state sync has observed
    /// every committee, the epoch manager's epoch stands in for it.
    async fn execution_epoch(&self) -> Result<Epoch, DryRunTransactionProcessorError> {
        let epoch_manager_epoch = self.epoch_manager.current_epoch().await?;
        Ok(self
            .consensus_epoch
            .current_within(epoch_manager_epoch)
            .unwrap_or(epoch_manager_epoch))
    }

    async fn get_virtual_substates(&self, epoch: Epoch) -> Result<VirtualSubstates, DryRunTransactionProcessorError> {
        let epoch_hash = self
            .epoch_manager
            .get_epoch_hash(epoch)
            .await
            .map_err(DryRunTransactionProcessorError::EpochManager)?;
        Ok(VirtualSubstates::from_iter([
            (
                VirtualSubstateId::CurrentEpoch,
                VirtualSubstate::CurrentEpoch(epoch.as_u64()),
            ),
            (
                VirtualSubstateId::CurrentEpochHash,
                VirtualSubstate::CurrentEpochHash(epoch_hash.into_array().into()),
            ),
        ]))
    }
}
