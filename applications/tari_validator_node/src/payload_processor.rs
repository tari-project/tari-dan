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

use std::{collections::HashMap, sync::Arc};

use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{services::template_provider::TemplateProvider, ObjectPledge, ShardId, SubstateState};
use tari_dan_core::{
    models::TariDanPayload,
    services::{PayloadProcessor, PayloadProcessorError},
};
use tari_dan_engine::{
    bootstrap_state,
    fees::{FeeModule, FeeTable},
    packager::LoadedTemplate,
    runtime::{AuthParams, ConsensusContext, RuntimeModule},
    state_store::{memory::MemoryStateStore, AtomicDb, StateStoreError, StateWriter},
    transaction::TransactionProcessor,
};
use tari_engine_types::commit_result::{ExecuteResult, FinalizeResult, RejectReason};
use tari_template_lib::{crypto::RistrettoPublicKeyBytes, prelude::NonFungibleAddress};
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

        let state_store = create_populated_state_store(pledges.into_values())?;

        // Include ownership token for the signers of this in the auth scope
        let owner_token = get_auth_token(&transaction);
        let auth_params = AuthParams {
            initial_ownership_proofs: vec![owner_token],
        };

        let initial_cost = 0;
        let modules: Vec<Box<dyn RuntimeModule<TTemplateProvider>>> =
            vec![Box::new(FeeModule::new(initial_cost, self.fee_table.clone()))];

        let processor = TransactionProcessor::new(
            self.template_provider.clone(),
            state_store,
            auth_params,
            consensus,
            modules,
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

fn get_auth_token(transaction: &Transaction) -> NonFungibleAddress {
    let public_key = RistrettoPublicKeyBytes::from_bytes(transaction.sender_public_key().as_bytes())
        .expect("Expected public key to be 32 bytes");
    NonFungibleAddress::from_public_key(public_key)
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
