//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::{hash_map::RandomState, HashMap, HashSet},
    iter::FromIterator,
};

use tari_dan_common_types::{ObjectPledge, ShardId, SubstateState};
use tari_dan_core::{
    models::TariDanPayload,
    services::{PayloadProcessor, PayloadProcessorError, TemplateProvider},
};
use tari_dan_engine::{
    packager::{LoadedTemplate, Package},
    runtime::{ConsensusProvider, IdProvider, RuntimeInterfaceImpl, StateTracker},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateStoreError, StateWriter},
    transaction::{TransactionError, TransactionProcessor},
    wasm::WasmExecutionError,
};
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason},
    substate::{Substate, SubstateAddress},
};
use tari_template_lib::models::{ComponentAddress, TemplateAddress};

#[derive(Debug, Default, Clone)]
pub struct TariDanPayloadProcessor<TTemplateProvider, TConsensusProvider> {
    template_provider: TTemplateProvider,
    consensus_provider: TConsensusProvider,
}

impl<TTemplateProvider, TConsensusProvider> TariDanPayloadProcessor<TTemplateProvider, TConsensusProvider> {
    pub fn new(template_provider: TTemplateProvider, consensus_provider: TConsensusProvider) -> Self {
        Self {
            template_provider,
            consensus_provider,
        }
    }
}

impl<TTemplateProvider, TConsensusProvider> PayloadProcessor<TariDanPayload>
    for TariDanPayloadProcessor<TTemplateProvider, TConsensusProvider>
where
    TTemplateProvider: TemplateProvider<Template = LoadedTemplate>,
    TConsensusProvider: ConsensusProvider + 'static,
{
    fn process_payload(
        &self,
        payload: TariDanPayload,
        pledges: HashMap<ShardId, ObjectPledge>,
    ) -> Result<FinalizeResult, PayloadProcessorError> {
        let transaction = payload.into_payload();
        let mut template_addresses = HashSet::<_, RandomState>::from_iter(transaction.required_templates());
        let components = transaction.required_components();

        let state_db = create_populated_state_store(pledges.into_values())?;
        template_addresses.extend(load_template_addresses_for_components(&state_db, &components)?);

        // Execution will fail if more than the max addresses are created
        // let id_provider = IdProvider::new(*transaction.hash(), transaction.meta().max_outputs());
        // Execution will fail if more than 64 new addresses are created
        let id_provider = IdProvider::new(*transaction.hash(), 64);
        let tracker = StateTracker::new(state_db, id_provider);
        let runtime = RuntimeInterfaceImpl::new(tracker, self.consensus_provider.clone());
        let mut builder = Package::builder();

        for addr in template_addresses {
            let template = self
                .template_provider
                .get_template_module(&addr)
                .map_err(|err| PayloadProcessorError::FailedToLoadTemplate(err.into()))?;
            builder.add_template(addr, template);
        }
        let package = builder.build();

        let processor = TransactionProcessor::new(runtime, package);
        let tx_hash = *transaction.hash();
        match processor.execute(transaction) {
            Ok(result) => Ok(result),
            Err(TransactionError::WasmExecutionError(WasmExecutionError::Panic { message, .. })) => {
                Ok(FinalizeResult::reject(tx_hash, RejectReason::ExecutionFailure(message)))
            },
            Err(err) => Ok(FinalizeResult::reject(
                tx_hash,
                RejectReason::ExecutionFailure(err.to_string()),
            )),
        }
    }
}

fn load_template_addresses_for_components(
    state_db: &MemoryStateStore,
    components: &[ComponentAddress],
) -> Result<Vec<TemplateAddress>, PayloadProcessorError> {
    let access = state_db
        .read_access()
        .map_err(PayloadProcessorError::FailedToLoadTemplate)?;
    let mut template_addresses = Vec::with_capacity(components.len());
    for component in components {
        let component = access.get_state::<_, Substate>(&SubstateAddress::Component(*component))?;
        let component = component
            .into_substate_value()
            .into_component()
            .expect("Component substate should be a component");
        template_addresses.push(component.template_address);
    }
    Ok(template_addresses)
}

fn create_populated_state_store<I: IntoIterator<Item = ObjectPledge>>(
    inputs: I,
) -> Result<MemoryStateStore, StateStoreError> {
    let state_db = MemoryStateStore::default();

    // Populate state db with inputs
    let mut tx = state_db.write_access()?;
    for input in inputs {
        match input.current_state {
            SubstateState::Up { data, .. } => {
                // TODO: address and state should be separate
                let addr = data.substate_address().clone();
                log::debug!(target: "tari::dan_layer::payload_processor",
                    "State store input substate: {} v{}",
                    addr,
                    data.version()
                );
                tx.set_state(&addr, data)?;
            },
            SubstateState::DoesNotExist | SubstateState::Down { .. } => { /* Do nothing */ },
        }
    }
    tx.commit()?;

    Ok(state_db)
}
