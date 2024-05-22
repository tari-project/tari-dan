//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use indexmap::{IndexMap, IndexSet};
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
use tari_dan_storage::consensus_models::{SubstateLockFlag, VersionedSubstateIdLockIntent};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason},
    substate::Substate,
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
    pub outputs: Vec<VersionedSubstateId>,
    pub execution_time: Duration,
}

impl ExecutionOutput {
    pub fn resolve_inputs(
        &self,
        inputs: IndexMap<VersionedSubstateId, Substate>,
    ) -> IndexSet<VersionedSubstateIdLockIntent> {
        if let Some(diff) = self.result.finalize.accept() {
            inputs
                .into_iter()
                .map(|(versioned_id, _)| {
                    let lock_flag = if diff.down_iter().any(|(id, _)| *id == versioned_id.substate_id) {
                        // Update all inputs that were DOWNed to be write locked
                        SubstateLockFlag::Write
                    } else {
                        // Any input not downed, gets a read lock
                        SubstateLockFlag::Read
                    };
                    VersionedSubstateIdLockIntent::new(versioned_id, lock_flag)
                })
                .collect()
        } else {
            // TODO: we might want to have a SubstateLockFlag::None for rejected transactions so that we still know the
            // shards involved but do not lock them. We dont actually lock anything for rejected transactions anyway.
            inputs
                .into_iter()
                .map(|(versioned_id, _)| VersionedSubstateIdLockIntent::new(versioned_id, SubstateLockFlag::Read))
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
        let timer = Instant::now();
        // Include ownership token for the signers of this in the auth scope
        let owner_token = get_auth_token(transaction.signer_public_key());
        let auth_params = AuthParams {
            initial_ownership_proofs: vec![owner_token],
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
        let tx_id = transaction.hash();
        let result = match processor.execute(transaction.clone()) {
            Ok(result) => result,
            Err(err) => ExecuteResult {
                finalize: FinalizeResult::new_rejected(tx_id, RejectReason::ExecutionFailure(err.to_string())),
            },
        };

        let outputs = result
            .finalize
            .result
            .accept()
            .map(|diff| {
                diff.up_iter()
                    .map(|(addr, substate)| VersionedSubstateId::new(addr.clone(), substate.version()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(ExecutionOutput {
            transaction,
            result,
            outputs,
            execution_time: timer.elapsed(),
        })
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
