//   Copyright 2023. The Tari Project
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

use std::sync::Arc;

use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_app_utilities::transaction_executor::TransactionExecutor;
use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_dan_engine::{
    fees::{FeeModule, FeeTable},
    runtime::{AuthParams, ConsensusContext, RuntimeModule},
    state_store::{memory::MemoryStateStore, StateStoreError},
    template::LoadedTemplate,
    transaction::{TransactionError, TransactionProcessor},
};
use tari_dan_storage::consensus_models::ExecutedTransaction;
use tari_engine_types::commit_result::{ExecuteResult, FinalizeResult, RejectReason};
use tari_template_lib::{crypto::RistrettoPublicKeyBytes, prelude::NonFungibleAddress};
use tari_transaction::Transaction;

#[derive(Debug, Clone)]
pub struct TariDanTransactionProcessor<TTemplateProvider> {
    template_provider: Arc<TTemplateProvider>,
    fee_table: FeeTable,
}

impl<TTemplateProvider> TariDanTransactionProcessor<TTemplateProvider> {
    pub fn new(template_provider: TTemplateProvider, fee_table: FeeTable) -> Self {
        Self {
            template_provider: Arc::new(template_provider),
            fee_table,
        }
    }
}

impl<TTemplateProvider> TransactionExecutor for TariDanTransactionProcessor<TTemplateProvider>
where TTemplateProvider: TemplateProvider<Template = LoadedTemplate>
{
    type Error = TransactionProcessorError;

    fn execute(
        &self,
        transaction: Transaction,
        state_store: MemoryStateStore,
        consensus_context: ConsensusContext,
    ) -> Result<ExecutedTransaction, Self::Error> {
        // Include ownership token for the signers of this in the auth scope
        let owner_token = get_auth_token(transaction.signer_public_key());
        let auth_params = AuthParams {
            initial_ownership_proofs: vec![owner_token],
        };

        let initial_cost = 0;
        let modules: Vec<Arc<dyn RuntimeModule<TTemplateProvider>>> =
            vec![Arc::new(FeeModule::new(initial_cost, self.fee_table.clone()))];

        let processor = TransactionProcessor::new(
            self.template_provider.clone(),
            state_store,
            auth_params,
            consensus_context,
            modules,
        );
        let tx_id = transaction.hash();
        let result = match processor.execute(transaction.clone()) {
            Ok(result) => result,
            Err(err) => ExecuteResult {
                finalize: FinalizeResult::reject(tx_id, RejectReason::ExecutionFailure(err.to_string())),
                transaction_failure: None,
                fee_receipt: None,
            },
        };

        Ok(ExecutedTransaction::new(transaction, result))
    }
}

fn get_auth_token(public_key: &PublicKey) -> NonFungibleAddress {
    let public_key =
        RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes()).expect("Expected public key to be 32 bytes");
    NonFungibleAddress::from_public_key(public_key)
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionProcessorError {
    #[error(transparent)]
    TransactionError(#[from] TransactionError),
    #[error(transparent)]
    StateStoreError(#[from] StateStoreError),
}
