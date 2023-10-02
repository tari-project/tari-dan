//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{sync::Arc, time::Instant};

use log::*;
use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{services::template_provider::TemplateProvider, ShardId};
use tari_dan_engine::{
    fees::{FeeModule, FeeTable},
    packager::LoadedTemplate,
    runtime::{AuthParams, RuntimeModule, VirtualSubstates},
    state_store::{memory::MemoryStateStore, StateStoreError},
    transaction::{TransactionError, TransactionProcessor},
};
use tari_dan_storage::consensus_models::ExecutedTransaction;
use tari_engine_types::commit_result::{ExecuteResult, FinalizeResult, RejectReason};
use tari_template_lib::{crypto::RistrettoPublicKeyBytes, prelude::NonFungibleAddress};
use tari_transaction::Transaction;

const _LOG_TARGET: &str = "tari::dan::transaction_executor";

pub trait TransactionExecutor {
    type Error: Send + Sync + 'static;

    fn execute(
        &self,
        transaction: Transaction,
        state_store: MemoryStateStore,
        virtual_substates: VirtualSubstates,
    ) -> Result<ExecutedTransaction, Self::Error>;
}

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
        virtual_substates: VirtualSubstates,
    ) -> Result<ExecutedTransaction, Self::Error> {
        let timer = Instant::now();
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
            virtual_substates,
            modules,
        );
        let tx_id = transaction.hash();
        let result = match processor.execute(transaction.clone()) {
            Ok(result) => result,
            Err(err) => ExecuteResult {
                finalize: FinalizeResult::new_rejectted(tx_id, RejectReason::ExecutionFailure(err.to_string())),
                transaction_failure: None,
                fee_receipt: None,
            },
        };

        let outputs = result
            .finalize
            .result
            .accept()
            .map(|diff| {
                diff.up_iter()
                    .map(|(addr, substate)| ShardId::from_address(addr, substate.version()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(ExecutedTransaction::new(transaction, result, outputs, timer.elapsed()))
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
