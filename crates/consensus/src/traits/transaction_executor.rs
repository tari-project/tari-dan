//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

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

/// The templates published in the chain a block builds on. A template published earlier in the block itself is not
/// included: a block's transactions do not depend on one another, so every validator may execute them in any order.
pub trait ParentChainTemplates {
    fn get_published_template(
        &self,
        address: &TemplateAddress,
    ) -> Result<Option<PublishedTemplate>, BlockTransactionExecutorError>;
}

pub trait BlockTransactionExecutor<TStateStore: StateStore> {
    /// Executes `transaction` in a block whose parent chain publishes `templates`. The execution must resolve a
    /// template exactly when `templates` holds it, whatever this validator has committed, so that every validator
    /// executing the block reaches the same result.
    fn execute(
        &self,
        transaction: &Transaction,
        locked_epoch: LockedEpoch,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
        templates: &dyn ParentChainTemplates,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError>;
}
