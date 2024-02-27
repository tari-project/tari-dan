//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore, StorageError};
use tari_transaction::Transaction;

// TODO: more refined errors
#[derive(thiserror::Error, Debug)]
pub enum BlockTransactionExecutorError {
    #[error("Placeholder error")]
    PlaceHolderError,
    #[error("Execution thread failure: {0}")]
    ExecutionThreadFailure(String),
    #[error(transparent)]
    StorageError(#[from] StorageError),
    // TODO: remove this variant when we have a remote substate implementation
    #[error("Remote substates are now allowed")]
    RemoteSubstatesNotAllowed,
}

pub trait BlockTransactionExecutor<TStateStore: StateStore> {
    fn execute(
        &mut self,
        transaction: Transaction,
        db_tx: &mut TStateStore::ReadTransaction<'_>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError>;
}

pub trait BlockTransactionExecutorBuilder<TStateStore: StateStore> {
    fn build(&self) -> Box<dyn BlockTransactionExecutor<TStateStore>>;
}
