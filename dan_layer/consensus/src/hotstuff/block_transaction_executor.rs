//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::info;
use tari_dan_common_types::SubstateAddress;
use tari_dan_engine::{
    bootstrap_state, state_store::{memory::MemoryStateStore, AtomicDb, StateWriter}
};
use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore, StateStoreReadTransaction, StorageError};
use tari_engine_types::{commit_result::TransactionResult, substate::SubstateId, virtual_substate::VirtualSubstates};
use tari_transaction::{SubstateRequirement, Transaction};

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

#[derive(Debug, Clone)]
pub struct BlockTransactionExecutor<TConsensusSpec: ConsensusSpec> {
    #[allow(dead_code)] 
    epoch_manager: TConsensusSpec::EpochManager,
    executor: TConsensusSpec::TransactionExecutor,
    // "cache" hashmap of updated outputs from previous transactions in the same block
    // so each consecutive transaction gets the most updated version for their inputs
    // TODO: store also the substate content and not only the version
    output_versions: HashMap<SubstateId, u32>
}

impl<TConsensusSpec> BlockTransactionExecutor<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        epoch_manager: TConsensusSpec::EpochManager,
        executor: TConsensusSpec::TransactionExecutor,
    ) -> Self {
        Self {
            epoch_manager,
            executor,
            output_versions: HashMap::new()
        }
    }

    pub fn execute(&mut self, transaction: Transaction, db_tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let id: tari_transaction::TransactionId = *transaction.id();
          
        // Get the latest input substates
        let inputs: HashSet<SubstateRequirement> = self.resolve_substates(&transaction, db_tx)?;
        info!(target: LOG_TARGET, "Transaction {} executing. Inputs: {:?}", id, inputs);

        // Create a memory db with all the input substates, needed for the transaction execution
        let state_db = Self::new_state_db();
        self.add_substates_to_memory_db(db_tx, &inputs, &state_db)?;

        // TODO: create the virtual substates for execution
        let virtual_substates = VirtualSubstates::new();

        // Execute the transaction and get the result    
        let mut executed = self.executor.execute(transaction, state_db, virtual_substates)
            .map_err(|e| BlockTransactionExecutorError::ExecutionThreadFailure(e.to_string()))?;

        // Store the output versions for future concurrent transactions in the same block
        if let TransactionResult::Accept(diff) = &executed.result().finalize.result {
            diff.up_iter().for_each(|s| {
                self.output_versions.insert(s.0.clone(), s.1.version());
            });
        }

        // Update the filled inputs to set the specific version, as we already know it after execution
        let filled_inputs_mut = executed.transaction_mut().filled_inputs_mut();
        inputs.into_iter().for_each(|input| {
            filled_inputs_mut.push(input);
        });

        info!(target: LOG_TARGET, "Transaction {} executed. result: {:?}", id, executed);

        Ok(executed)
    }

    fn new_state_db() -> MemoryStateStore {
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
        db_tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>
    ) -> Result<HashSet<SubstateRequirement>, BlockTransactionExecutorError> {
        let mut resolved_substates = HashSet::new();
        for input in transaction.all_inputs_iter() {
            // We try to fetch each input from the block "cache", and only hit the DB if the input has not been used in the block before
            let resolved_substate = match self.output_versions.get(input.substate_id()) {
                Some(version) => SubstateRequirement::new(input.substate_id.clone(), Some(*version)),
                None => self.resolve_local_substate(input.substate_id(), db_tx)?,
            };
            resolved_substates.insert(resolved_substate);
        }
        // TODO: we assume local only transactions, we need to implement multi-shard transactions
        Ok(resolved_substates)
    }

    fn resolve_local_substate(&self, id: &SubstateId, db_tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>) -> Result<SubstateRequirement, BlockTransactionExecutorError> {
        let version = Self::get_last_substate_version(db_tx, id)
            .ok_or(BlockTransactionExecutorError::PlaceHolderError)?;
        Ok(SubstateRequirement::new(id.clone(), Some(version)))
    }

    fn add_substates_to_memory_db(&self, db_tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>, inputs: &HashSet<SubstateRequirement>, out: &MemoryStateStore) -> Result<(), BlockTransactionExecutorError>{
        let mut substates = vec![];
        for input in inputs {
            let address = input.to_substate_address();
            let substate = db_tx.substates_get(&address)?;
            substates.push(substate);
        }
        
        out.set_all(
            substates
                .into_iter()
                .map(|s| (s.substate_id.clone(), s.into_substate())),
        );

        Ok(())
    }

    fn get_last_substate_version(db_tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>, substate_id: &SubstateId) -> Option<u32> {
        // TODO: store in DB the substate_id and version so we can just fetch the latest one and we don't have to loop from 0
        let mut version = 0;
        loop {
            let address = SubstateAddress::from_address(substate_id, version);
            info!(target: LOG_TARGET, "get_last_substate_version {}:{}, address: {}", substate_id, version, address);
            match db_tx.substates_get(&address) {
                Ok(_) => {
                    version += 1;
                },
                Err(_) => {
                    if version > 0 {
                        return Some(version - 1)
                    } else {
                        return None;
                    }
                },
            }
        }
    }
}
