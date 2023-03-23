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
    collections::{BTreeSet, HashMap},
    convert::TryFrom,
    sync::Arc,
};

use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{services::template_provider::TemplateProvider, ObjectPledge, ShardId, SubstateState};
use tari_dan_core::{
    models::TariDanPayload,
    services::{PayloadProcessor, PayloadProcessorError},
};
use tari_dan_engine::{
    bootstrap_state,
    fees::{FeeModule, FeeTable},
    packager::{LoadedTemplate, Package},
    runtime::{AuthParams, ConsensusContext, RuntimeModule},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateStoreError, StateWriter},
    transaction::TransactionProcessor,
};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason},
    substate::{Substate, SubstateAddress},
};
use tari_template_lib::{
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, ComponentAddress, TemplateAddress},
    prelude::NonFungibleAddress,
};
use tari_transaction::Transaction;

#[derive(Debug, Clone)]
pub struct TariDanPayloadProcessor<TTemplateProvider> {
    template_provider: Arc<TTemplateProvider>,
    fee_table: FeeTable,
}

impl<TTemplateProvider> TariDanPayloadProcessor<TTemplateProvider> {
    pub fn new(template_provider: TTemplateProvider, fee_table: FeeTable) -> Self {
        Self {
            template_provider: Arc::new(template_provider),
            fee_table,
        }
    }
}

impl<TTemplateProvider> PayloadProcessor<TariDanPayload> for TariDanPayloadProcessor<TTemplateProvider>
where TTemplateProvider: TemplateProvider<Template = LoadedTemplate>
{
    fn process_payload(
        &self,
        payload: TariDanPayload,
        pledges: HashMap<ShardId, ObjectPledge>,
        consensus: ConsensusContext,
    ) -> Result<ExecuteResult, PayloadProcessorError> {
        let transaction = payload.into_payload();
        // let mut template_addresses = transaction.required_templates();
        // let components = transaction.required_components();

        let state_store = create_populated_state_store(pledges.into_values())?;
        // template_addresses.extend(load_template_addresses_for_components(&state_store, &components)?);

        // let package = build_package(&self.template_provider, template_addresses)?;

        // Include ownership token for the signers of this in the auth scope
        let owner_token = get_auth_token(&transaction);
        let auth_params = AuthParams {
            initial_ownership_proofs: vec![owner_token],
        };

        // 1 per byte
        // Divide by 2 to account for the cost of CBOR
        let initial_cost = self.fee_table.per_kb_wasm_size() * package.total_code_byte_size() as u64 / 1024 / 2;
        let modules: Vec<Box<dyn RuntimeModule>> = vec![Box::new(FeeModule::new(initial_cost, self.fee_table.clone()))];

        let processor = TransactionProcessor::new(
            self.template_provider.clone(),
            state_store,
            auth_params,
            consensus,
            modules,
            Amount::try_from(self.fee_table.loan()).expect("Fee loan too large"),
        );
        let tx_hash = *transaction.hash();
        match processor.execute(transaction) {
            Ok(result) => Ok(result),
            Err(err) => Ok(ExecuteResult {
                finalize: FinalizeResult::reject(tx_hash, RejectReason::ExecutionFailure(err.to_string())),
                transaction_failure: None,
                fee_receipt: None,
            }),
        }
    }
}

fn build_package<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
    template_provider: &TTemplateProvider,
    template_addresses: BTreeSet<TemplateAddress>,
) -> Result<Package, PayloadProcessorError> {
    let mut builder = Package::builder();

    for addr in template_addresses {
        let template = template_provider
            .get_template_module(&addr)
            .map_err(|err| PayloadProcessorError::FailedToLoadTemplate(err.into()))?;
        builder.add_template(addr, template);
    }

    Ok(builder.build())
}

fn get_auth_token(transaction: &Transaction) -> NonFungibleAddress {
    let public_key = RistrettoPublicKeyBytes::from_bytes(transaction.sender_public_key().as_bytes())
        .expect("Expected public key to be 32 bytes");
    NonFungibleAddress::from_public_key(public_key)
}

fn load_template_addresses_for_components(
    state_db: &MemoryStateStore,
    components: &BTreeSet<ComponentAddress>,
) -> Result<BTreeSet<TemplateAddress>, PayloadProcessorError> {
    let access = state_db
        .read_access()
        .map_err(PayloadProcessorError::FailedToLoadTemplate)?;
    let mut template_addresses = BTreeSet::new();
    for component in components {
        let component = access.get_state::<_, Substate>(&SubstateAddress::Component(*component))?;
        let component = component
            .into_substate_value()
            .into_component()
            .expect("Component substate should be a component");
        template_addresses.insert(component.template_address);
    }
    Ok(template_addresses)
}

fn create_populated_state_store<I: IntoIterator<Item = ObjectPledge>>(
    inputs: I,
) -> Result<MemoryStateStore, StateStoreError> {
    let state_db = MemoryStateStore::default();

    // Populate state db with inputs
    let mut tx = state_db.write_access()?;
    // Add bootstrapped substates
    bootstrap_state(&mut tx)?;

    for input in inputs {
        match input.current_state {
            SubstateState::Up { address, data, .. } => {
                log::debug!(target: "tari::dan_layer::payload_processor",
                    "State store input substate: {} v{}",
                    address,
                    data.version()
                );
                tx.set_state(&address, data)?;
            },
            SubstateState::DoesNotExist | SubstateState::Down { .. } => { /* Do nothing */ },
        }
    }
    tx.commit()?;

    Ok(state_db)
}
