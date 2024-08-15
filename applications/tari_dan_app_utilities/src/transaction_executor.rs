//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use indexmap::IndexMap;
use log::*;
use tari_common::configuration::Network;
use tari_common_types::types::PublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_dan_engine::{
    fees::{FeeModule, FeeTable},
    runtime::{AuthParams, RuntimeModule},
    state_store::{memory::MemoryStateStore, StateStoreError},
    template::LoadedTemplate,
    transaction::{TransactionError, TransactionProcessor},
};
use tari_dan_storage::consensus_models::{SubstateLockType, VersionedSubstateIdLockIntent};
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateId},
    virtual_substate::VirtualSubstates,
};
use tari_template_lib::{crypto::RistrettoPublicKeyBytes, prelude::NonFungibleAddress};
use tari_transaction::{Transaction, VersionedSubstateId};

const _LOG_TARGET: &str = "tari::dan::transaction_executor";

pub trait TransactionExecutor {
    type Error: std::error::Error + Send + Sync + 'static;

    fn execute(
        &self,
        transaction: Transaction,
        state_store: MemoryStateStore,
        virtual_substates: VirtualSubstates,
    ) -> Result<ExecutionOutput, Self::Error>;
}

#[derive(Debug, Clone)]
pub struct ExecutionOutput {
    pub transaction: Transaction,
    pub result: ExecuteResult,
}

impl ExecutionOutput {
    pub fn resolve_inputs(&self, inputs: &IndexMap<SubstateId, Substate>) -> Vec<VersionedSubstateIdLockIntent> {
        if let Some(diff) = self.result.finalize.accept() {
            inputs
                .iter()
                .map(|(substate_id, substate)| {
                    let lock_flag = if diff.down_iter().any(|(id, _)| id == substate_id) {
                        // Update all inputs that were DOWNed to be write locked
                        SubstateLockType::Write
                    } else {
                        // Any input not downed, gets a read lock
                        SubstateLockType::Read
                    };
                    VersionedSubstateIdLockIntent::new(
                        VersionedSubstateId::new(substate_id.clone(), substate.version()),
                        lock_flag,
                    )
                })
                .collect()
        } else {
            // TODO: we might want to have a SubstateLockFlag::None for rejected transactions so that we still know the
            // shards involved but do not lock them. We dont actually lock anything for rejected transactions anyway.
            inputs
                .iter()
                .map(|(substate_id, substate)| {
                    VersionedSubstateIdLockIntent::new(
                        VersionedSubstateId::new(substate_id.clone(), substate.version()),
                        SubstateLockType::Read,
                    )
                })
                .collect()
        }
    }
}

#[derive(Debug, Clone)]
pub struct TariDanTransactionProcessor<TTemplateProvider> {
    template_provider: Arc<TTemplateProvider>,
    fee_table: FeeTable,
    network: Network,
}

impl<TTemplateProvider> TariDanTransactionProcessor<TTemplateProvider> {
    pub fn new(network: Network, template_provider: TTemplateProvider, fee_table: FeeTable) -> Self {
        Self {
            template_provider: Arc::new(template_provider),
            fee_table,
            network,
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
    ) -> Result<ExecutionOutput, Self::Error> {
        // Include signature public key badges for all transaction signers in the initial auth scope
        // NOTE: we assume all signatures have already been validated.
        let initial_ownership_proofs = transaction
            .signatures()
            .iter()
            .map(|sig| public_key_to_fungible_address(sig.public_key()))
            .collect();
        let auth_params = AuthParams {
            initial_ownership_proofs,
        };

        let initial_cost = 0;
        let modules: Vec<Arc<dyn RuntimeModule>> = vec![Arc::new(FeeModule::new(initial_cost, self.fee_table.clone()))];

        let processor = TransactionProcessor::new(
            self.template_provider.clone(),
            state_store.clone(),
            auth_params,
            virtual_substates,
            modules,
            self.network,
        );
        let result = processor.execute(transaction.clone())?;
        //     Ok(result) => result,
        //     // TODO: This may occur due to an internal error (e.g. OOM, etc).
        //     Err(err) => ExecuteResult {
        //         finalize: FinalizeResult::new_rejected(
        //             tx_id,
        //             RejectReason::ExecutionFailure(format!("BUG: {err}")),
        //         ),
        //         execution_time: Duration::default(),
        //     },
        // };

        Ok(ExecutionOutput { transaction, result })
    }
}

fn public_key_to_fungible_address(public_key: &PublicKey) -> NonFungibleAddress {
    RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes())
        .expect("Expected public key to be 32 bytes")
        .to_non_fungible_address()
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionProcessorError {
    #[error(transparent)]
    TransactionError(#[from] TransactionError),
    #[error(transparent)]
    StateStoreError(#[from] StateStoreError),
}
