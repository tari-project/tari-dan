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

use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{ObjectPledge, ShardId, SubstateState};
use tari_dan_core::{
    models::TariDanPayload,
    services::{PayloadProcessor, PayloadProcessorError, TemplateProvider},
};
use tari_dan_engine::{
    bootstrap_state,
    packager::{LoadedTemplate, Package},
    runtime::{AuthParams, ConsensusContext},
    state_store::{memory::MemoryStateStore, AtomicDb, StateReader, StateStoreError, StateWriter},
    transaction::{Transaction, TransactionError, TransactionProcessor},
    wasm::WasmExecutionError,
};
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason},
    substate::{Substate, SubstateAddress},
};
use tari_template_lib::{
    crypto::RistrettoPublicKeyBytes,
    models::{ComponentAddress, TemplateAddress},
    prelude::NonFungibleAddress,
};

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
where TTemplateProvider: TemplateProvider<Template = LoadedTemplate>
{
    fn process_payload(
        &self,
        payload: TariDanPayload,
        pledges: HashMap<ShardId, ObjectPledge>,
        consensus: ConsensusContext,
    ) -> Result<FinalizeResult, PayloadProcessorError> {
        let transaction = payload.into_payload();
        let mut template_addresses = HashSet::<_, RandomState>::from_iter(transaction.required_templates());
        let components = transaction.required_components();

        let state_store = create_populated_state_store(pledges.into_values())?;
        template_addresses.extend(load_template_addresses_for_components(&state_store, &components)?);

        let package = build_package(&self.template_provider, template_addresses)?;

        // Include ownership token for the signers of this in the auth scope
        let owner_token = get_auth_token(&transaction);
        let auth_params = AuthParams {
            initial_ownership_proofs: vec![owner_token],
        };

        let modules = vec![]; // No modules for now, currently used in tests. Also will be useful for more advanced use-cases like fees, etc.

        let processor = TransactionProcessor::new(package, state_store, auth_params, consensus, modules);
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

fn build_package<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
    template_provider: &TTemplateProvider,
    template_addresses: HashSet<TemplateAddress>,
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
