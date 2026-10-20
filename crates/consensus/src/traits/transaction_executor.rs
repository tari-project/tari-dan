//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, sync::Arc};

use tari_engine_types::{published_template::PublishedTemplate, substate::Substate};
use tari_ootle_common_types::{SubstateRequirement, optional::IsNotFoundError};
use tari_ootle_storage::{
    StateStore,
    StorageError,
    consensus_models::{LockedEpoch, TransactionExecution, TransactionPoolError},
};
use tari_ootle_transaction::Transaction;
use tari_template_lib_types::TemplateAddress;

use crate::hotstuff::substate_store::SubstateStoreError;

#[derive(thiserror::Error, Debug)]
pub enum BlockTransactionExecutorError {
    #[error("Execution thread failure: {0}")]
    ExecutionThreadFailure(String),
    #[error(transparent)]
    StorageError(#[from] StorageError),
    #[error("State store error: {0}")]
    StateStoreError(String),
    #[error("Substate store error: {0}")]
    SubstateStoreError(#[from] SubstateStoreError),
    #[error("Transaction pool error: {0}")]
    TransactionPoolError(#[from] TransactionPoolError),
    #[error("BUG: Invariant error: {0}")]
    InvariantError(String),
}

impl IsNotFoundError for BlockTransactionExecutorError {
    fn is_not_found_error(&self) -> bool {
        match self {
            BlockTransactionExecutorError::StorageError(err) => err.is_not_found_error(),
            BlockTransactionExecutorError::SubstateStoreError(err) => err.is_not_found_error(),
            _ => false,
        }
    }
}

/// The templates published in the uncommitted ancestors of the block a transaction executes in. With the templates in
/// durably committed state, they are every template the block's chain publishes. The block's own changes are not
/// among them, so its transactions may execute in any order.
#[derive(Debug, Clone, Default)]
pub struct UncommittedChainTemplates {
    templates: Arc<HashMap<TemplateAddress, Arc<PublishedTemplate>>>,
}

impl UncommittedChainTemplates {
    pub fn new(templates: HashMap<TemplateAddress, Arc<PublishedTemplate>>) -> Self {
        Self {
            templates: Arc::new(templates),
        }
    }

    pub fn get(&self, address: &TemplateAddress) -> Option<&Arc<PublishedTemplate>> {
        self.templates.get(address)
    }

    pub fn is_empty(&self) -> bool {
        self.templates.is_empty()
    }
}

pub trait BlockTransactionExecutor<TStateStore: StateStore> {
    /// Executes `transaction` in a block whose uncommitted ancestors publish `templates`. The execution resolves a
    /// template from `templates`, then from durably committed state, so that every validator executing the block
    /// resolves the same templates however far it has committed.
    fn execute(
        &self,
        transaction: &Transaction,
        locked_epoch: LockedEpoch,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
        templates: &UncommittedChainTemplates,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError>;
}
