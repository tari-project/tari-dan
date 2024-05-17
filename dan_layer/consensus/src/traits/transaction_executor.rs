//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore, StorageError};
use tari_engine_types::substate::SubstateId;
use tari_transaction::Transaction;

use crate::hotstuff::substate_store::{PendingSubstateStore, SubstateStoreError};

#[derive(thiserror::Error, Debug)]
pub enum BlockTransactionExecutorError {
    #[error("Unable to resolve substate id: {substate_id}")]
    UnableToResolveSubstateId { substate_id: SubstateId },
    #[error("Execution thread failure: {0}")]
    ExecutionThreadFailure(String),
    #[error(transparent)]
    StorageError(#[from] StorageError),
    // TODO: remove this variant when we have a remote substate implementation
    #[error("Remote substates are now allowed")]
    RemoteSubstatesNotAllowed,
    #[error("State store error: {0}")]
    StateStoreError(String),
    #[error("Substate store error: {0}")]
    SubstateStoreError(#[from] SubstateStoreError),
}

pub trait BlockTransactionExecutor<TStateStore: StateStore> {
    fn execute(
        &self,
        transaction: Transaction,
        store: &PendingSubstateStore<TStateStore>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError>;
}
