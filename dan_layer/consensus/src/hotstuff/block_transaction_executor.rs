//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::{HashMap, HashSet}, sync::Arc, time::Instant};

use log::info;
use tari_dan_common_types::{Epoch, SubstateAddress};
use tari_dan_engine::{
    bootstrap_state, fees::FeeModule, runtime::{AuthParams, RuntimeModule}, state_store::{memory::MemoryStateStore, AtomicDb, StateWriter}, transaction::TransactionProcessor
};
use tari_dan_storage::{consensus_models::{ExecutedTransaction, SubstateRecord}, StateStore, StateStoreReadTransaction, StorageError};
use tari_engine_types::{substate::SubstateId, virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates}};
use tari_epoch_manager::EpochManagerReader;
use tari_mmr::ArrayLikeExt;
use tari_transaction::{Instruction, SubstateRequirement, Transaction};

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
    // TODO: use the epoch manager to build virtual substates
    #[allow(dead_code)] 
    epoch_manager: TConsensusSpec::EpochManager,
    executor: TConsensusSpec::TransactionExecutor,
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
        }
    }

    pub fn execute(&self, transaction: Transaction, mut db_tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let id: tari_transaction::TransactionId = *transaction.id();
          
        // Get the latest input substates
        let inputs: HashSet<SubstateRequirement> = self.resolve_substates(&transaction, &mut db_tx)?;

        // Create a memory db with all the input substates
        let state_db = Self::new_state_db();
        self.add_substates_to_memory_db(db_tx, &inputs, &state_db)?;

        // TODO: create the virtual substates for execution
        let virtual_substates = VirtualSubstates::new();

        // Execute the transaction and get the result
        info!(target: LOG_TARGET, "Transaction {} executing. virtual_substates = [{}]", id, virtual_substates.keys().map(|addr| addr.to_string()).collect::<Vec<_>>().join(", "));        
        let mut executed = self.executor.execute(transaction, state_db, virtual_substates)
            .map_err(|e| BlockTransactionExecutorError::ExecutionThreadFailure(e.to_string()))?;
        info!(target: LOG_TARGET, "Transaction {} executed. result: {:?}", id, executed);

        // Update the filled inputs to set the specific version, as we already know it after execution
        info!(target: LOG_TARGET, "Transaction {} old filled inputs: {:?}", id, executed.transaction().filled_inputs());
        let filled_inputs_mut = executed.transaction_mut().filled_inputs_mut();
        inputs.into_iter().for_each(|input| {
            filled_inputs_mut.push(input);
        });
        info!(target: LOG_TARGET, "Transaction {} new filled inputs: {:?}", id, executed.transaction().filled_inputs());

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
        let input_addresses = self.resolve_local_substates(transaction, db_tx)?;

        if input_addresses.is_empty() {
            // TODO: for now we are going to err if there is any unknown or remote substate in the tx
            Err(BlockTransactionExecutorError::RemoteSubstatesNotAllowed)        
        } else {
            Ok(input_addresses)
        }
    }

    fn resolve_local_substates(&self, transaction: &Transaction, db_tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>) -> Result<HashSet<SubstateRequirement>, BlockTransactionExecutorError> {
        let inputs = transaction.all_inputs_iter();

        let input_requirements = inputs.map(|input| {
            if input.version.is_some() {
                Ok(input.clone())
            } else {       
                // TODO: what to do if the DB does not have the substate? for now we return an error
                let version = Self::get_last_substate_version(db_tx, input.substate_id())
                    .ok_or(BlockTransactionExecutorError::PlaceHolderError)?;
                Ok(SubstateRequirement::new(input.substate_id().clone(), Some(version)))
            }
        }).collect::<Result<HashSet<SubstateRequirement>, BlockTransactionExecutorError>>()?;

        info!(
            target: LOG_TARGET,
            "resolve_local_substates 2, input_addresses: {:?}", input_requirements);

        Ok(input_requirements)
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
            match db_tx.substates_get(&address) {
                Ok(_) => {
                    version += 1;
                },
                Err(_) => {
                    if version > 0 {
                        info!(
                            target: LOG_TARGET,
                            "get_last_substate_version: {}:{}", substate_id, version);
                        return Some(version - 1)
                    } else {
                        return None;
                    }
                },
            }
        }
    }
}
