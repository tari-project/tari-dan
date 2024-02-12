//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::info;
use tari_dan_common_types::{Epoch, SubstateAddress};
use tari_dan_engine::{
    bootstrap_state,
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
};
use tari_dan_storage::{consensus_models::{ExecutedTransaction, SubstateRecord}, StateStore, StorageError};
use tari_engine_types::virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::{Instruction, Transaction};

use crate::traits::{ConsensusSpec, TransactionExecutor};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

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

// TODO: we should keep a "proxy" hashmap (or memory storage) of updated states from previous transactions in the same
// block,       so each consecutive transaction gets the most updated version for their inputs.
//       If no previous tx wrote on a input, just get it from the regular state store
#[derive(Debug, Clone)]
pub struct BlockTransactionExecutor<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    // TODO: use the epoch manager to build virtual substates
    #[allow(dead_code)] 
    epoch_manager: TConsensusSpec::EpochManager,
    executor: TConsensusSpec::TransactionExecutor,
}

impl<TConsensusSpec> BlockTransactionExecutor<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        executor: TConsensusSpec::TransactionExecutor,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            executor,
        }
    }

    pub fn execute(&self, transaction: Transaction) -> Result<Option<ExecutedTransaction>, BlockTransactionExecutorError> {
        let id: tari_transaction::TransactionId = *transaction.id();

        // We only need to re-execute a transaction if any of its input versions is "None"
        let must_reexecute = transaction.has_inputs_without_version();
        if !must_reexecute {
            info!(
                target: LOG_TARGET,
                "Skipping transaction {} execution as all inputs specify the version",
                id,
            );
            return Ok(None)
        }

        info!(
            target: LOG_TARGET,
            "Executing transaction: {}",
            id,
        );

        let state_db = self.new_state_db();

        // TODO
        let virtual_substates = VirtualSubstates::new();

        info!(target: LOG_TARGET, "Transaction {} executing. virtual_substates = [{}]", transaction.id(), virtual_substates.keys().map(|addr| addr.to_string()).collect::<Vec<_>>().join(", "));
        let executor = self.executor.clone();
        let result = match self.resolve_substates(&transaction, &state_db) {
            Ok(()) => {
                executor
                    .execute(transaction, state_db, virtual_substates)
                    .map_err(|_| BlockTransactionExecutorError::PlaceHolderError)?
            },
            Err(err) => return Err(err.into()),
        };

        Ok(Some(result))
    }

    fn new_state_db(&self) -> MemoryStateStore {
        let state_db = MemoryStateStore::new();
        // unwrap: Memory state store is infallible
        let mut tx = state_db.write_access().unwrap();
        // Add bootstrapped substates
        bootstrap_state(&mut tx).unwrap();
        tx.commit().unwrap();
        state_db
    }

    fn resolve_substates(
        &self,
        transaction: &Transaction,
        out: &MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        let missing_shards = self.resolve_local_substates(transaction, out)?;

        if missing_shards.is_empty() {
            Ok(())
        } else {
            // TODO: for now we are going to err if there is any remote substate in the tx
            Err(BlockTransactionExecutorError::RemoteSubstatesNotAllowed)
        }
    }

    fn resolve_local_substates(&self, transaction: &Transaction, out: &MemoryStateStore) -> Result<HashSet<SubstateAddress>, BlockTransactionExecutorError> {
        let inputs = transaction.all_input_addresses_iter();
        let (local_substates, missing_shards) = self
            .store
            .with_read_tx(|tx| SubstateRecord::get_any(tx, inputs))?;

        info!(
            target: LOG_TARGET,
            "Found {} local substates and {} missing shards",
            local_substates.len(),
            missing_shards.len());

        out.set_all(
            local_substates
                .into_iter()
                .map(|s| (s.substate_id.clone(), s.into_substate())),
        );

        Ok(missing_shards)
    }
}
