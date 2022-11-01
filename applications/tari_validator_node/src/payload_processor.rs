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

use std::collections::HashMap;

use tari_dan_common_types::{ShardId, SubstateState};
use tari_dan_core::{
    models::{ObjectPledge, TariDanPayload},
    services::{PayloadProcessor, PayloadProcessorError, TemplateProvider},
};
use tari_dan_engine::{
    packager::{Package, TemplateModuleLoader},
    runtime::{RuntimeInterfaceImpl, StateTracker},
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
    transaction::TransactionProcessor,
};
use tari_engine_types::{commit_result::FinalizeResult, substate::SubstateValue};

#[derive(Debug, Default)]
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
        pledges: HashMap<ShardId, Vec<ObjectPledge>>,
    ) -> Result<FinalizeResult, PayloadProcessorError> {
        let transaction = payload.into_payload();
        let template_addresses = transaction.required_templates();

        let state_db = create_populated_state_store(pledges.into_values().flatten());
        let tracker = StateTracker::new(state_db, *transaction.hash());
        let runtime = RuntimeInterfaceImpl::new(tracker);

        // let mut builder = Package::builder();
        // for wasm in wasms {
        //     builder.add_wasm_module(wasm);
        // }
        // let package = builder.build().unwrap();
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
        let result = processor.execute(transaction)?;
        Ok(result)
    }
}

fn create_populated_state_store<I: IntoIterator<Item = ObjectPledge>>(inputs: I) -> MemoryStateStore {
    let state_db = MemoryStateStore::default();

    // Populate state db with inputs
    let mut tx = state_db.write_access().unwrap();
    for input in inputs {
        match input.current_state {
            SubstateState::Up { data, .. } => {
                // TODO: Engine should be able to read SubstateValue
                match data.into_substate() {
                    SubstateValue::Component(component) => {
                        tx.set_state_raw(
                            input.shard_id.as_bytes(),
                            tari_dan_engine::abi::encode(&component).unwrap(),
                        )
                        .unwrap();
                    },
                    SubstateValue::Resource(resx) => {
                        tx.set_state_raw(input.shard_id.as_bytes(), tari_dan_engine::abi::encode(&resx).unwrap())
                            .unwrap();
                    },
                }
            },
            SubstateState::DoesNotExist | SubstateState::Down { .. } => { /* Do nothing */ },
        }
    }
    tx.commit().unwrap();

    state_db
}
