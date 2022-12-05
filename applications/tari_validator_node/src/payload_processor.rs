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
    packager::{Package, TemplateModuleLoader},
    runtime::{IdProvider, RuntimeInterfaceImpl, StateTracker},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateStoreError, StateWriter},
    transaction::TransactionProcessor,
};
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason},
    substate::{Substate, SubstateAddress, SubstateValue},
};
use tari_template_lib::models::{ComponentAddress, ResourceAddress, TemplateAddress, VaultId};

#[derive(Debug, Default, Clone)]
pub struct TariDanPayloadProcessor<TTemplateProvider> {
    template_provider: TTemplateProvider,
}

impl<TTemplateProvider> TariDanPayloadProcessor<TTemplateProvider> {
    pub fn new(template_provider: TTemplateProvider) -> Self {
        Self { template_provider }
    }
}

impl<TTemplateProvider> PayloadProcessor<TariDanPayload> for TariDanPayloadProcessor<TTemplateProvider>
where TTemplateProvider: TemplateProvider
{
    fn process_payload(
        &self,
        payload: TariDanPayload,
        pledges: HashMap<ShardId, Option<ObjectPledge>>,
    ) -> Result<FinalizeResult, PayloadProcessorError> {
        let transaction = payload.into_payload();
        let mut template_addresses = HashSet::<_, RandomState>::from_iter(transaction.required_templates());
        let components = transaction.required_components();

        let state_db = create_populated_state_store(pledges.into_values().flatten())?;
        template_addresses.extend(load_template_addresses_for_components(&state_db, &components)?);

        let id_provider = IdProvider::new(*transaction.hash(), transaction.meta().max_outputs());
        let tracker = StateTracker::new(state_db, id_provider);
        let runtime = RuntimeInterfaceImpl::new(tracker);

        let mut builder = Package::builder();

        for addr in template_addresses {
            let template = self
                .template_provider
                .get_template_module(&addr)
                .map_err(|err| PayloadProcessorError::FailedToLoadTemplate(err.into()))?;
            let loaded_template = template
                .load_template()
                .map_err(|err| PayloadProcessorError::FailedToLoadTemplate(err.into()))?;
            builder.add_template(addr, loaded_template);
        }
        let package = builder.build();

        let processor = TransactionProcessor::new(runtime, package);
        let tx_hash = *transaction.hash();
        let result = match processor.execute(transaction) {
            Ok(result) => result,
            Err(err) => FinalizeResult::errored(tx_hash, RejectReason::ExecutionFailure(err.to_string())),
        };
        Ok(result)
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
            .into_substate()
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
                // TODO: The 1:1 mapping between ShardId and component/resource address could be cleaned up
                match data.substate_value() {
                    SubstateValue::Component(_) => {
                        tx.set_state(
                            &SubstateAddress::Component(ComponentAddress::from(input.shard_id.into_array())),
                            data,
                        )
                        .unwrap();
                    },
                    SubstateValue::Resource(_) => {
                        tx.set_state(
                            &SubstateAddress::Resource(ResourceAddress::from(input.shard_id.into_array())),
                            data,
                        )
                        .unwrap();
                    },
                    SubstateValue::Vault(_) => {
                        tx.set_state(
                            &SubstateAddress::Vault(VaultId::from(input.shard_id.into_array())),
                            data,
                        )
                        .unwrap();
                    },
                }
            },
            SubstateState::DoesNotExist | SubstateState::Down { .. } => { /* Do nothing */ },
        }
    }
    tx.commit()?;

    Ok(state_db)
}
