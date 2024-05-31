//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use indexmap::IndexMap;
use log::info;
use tari_consensus::{
    hotstuff::substate_store::PendingSubstateStore,
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError, ReadableSubstateStore},
};
use tari_dan_app_utilities::transaction_executor::TransactionExecutor;
use tari_dan_common_types::optional::Optional;
use tari_dan_engine::{
    bootstrap_state,
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
};
use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore};
use tari_engine_types::{
    substate::{Substate, SubstateId},
    virtual_substate::VirtualSubstates,
};
use tari_transaction::{Transaction, VersionedSubstateId};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

#[derive(Debug, Clone)]
pub struct TariDanBlockTransactionExecutor<TEpochManager, TExecutor> {
    // TODO: we will need the epoch manager for virtual substates and other operations in the future
    #[allow(dead_code)]
    epoch_manager: TEpochManager,
    executor: TExecutor,
}

impl<TEpochManager, TExecutor, TStateStore> BlockTransactionExecutor<TStateStore>
    for TariDanBlockTransactionExecutor<TEpochManager, TExecutor>
where
    TStateStore: StateStore,
    TExecutor: TransactionExecutor,
{
    fn execute(
        &self,
        transaction: Transaction,
        store: &PendingSubstateStore<TStateStore>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let id: tari_transaction::TransactionId = *transaction.id();

        // Get the latest input substates
        let inputs = self.resolve_substates::<TStateStore>(&transaction, store)?;
        info!(target: LOG_TARGET, "Transaction {} executing. Inputs: {:?}", id, inputs);

        // Create a memory db with all the input substates, needed for the transaction execution
        let state_db = Self::new_state_db();
        self.add_substates_to_memory_db(&inputs, &state_db)?;

        // TODO: create the virtual substates for execution
        let virtual_substates = VirtualSubstates::new();

        // Execute the transaction and get the result
        let exec_output = self
            .executor
            .execute(transaction, state_db, virtual_substates)
            .map_err(|e| BlockTransactionExecutorError::ExecutionThreadFailure(e.to_string()))?;

        // Generate the resolved inputs to set the specific version and required lock flag, as we know it after
        // execution
        let resolved_inputs = exec_output.resolve_inputs(inputs);

        let executed = ExecutedTransaction::new(
            exec_output.transaction,
            exec_output.result,
            resolved_inputs,
            exec_output.outputs,
            exec_output.execution_time,
        );
        info!(target: LOG_TARGET, "Transaction {} executed. {}", id,executed.result().finalize.result);
        Ok(executed)
    }
}

impl<TEpochManager, TExecutor> TariDanBlockTransactionExecutor<TEpochManager, TExecutor> {
    pub fn new(epoch_manager: TEpochManager, executor: TExecutor) -> Self {
        Self {
            epoch_manager,
            executor,
        }
    }

    fn resolve_substates<TStateStore: StateStore>(
        &self,
        transaction: &Transaction,
        store: &PendingSubstateStore<TStateStore>,
    ) -> Result<IndexMap<VersionedSubstateId, Substate>, BlockTransactionExecutorError> {
        let mut resolved_substates = IndexMap::with_capacity(transaction.num_unique_inputs());

        for input in transaction.all_inputs_iter() {
            match input.version() {
                Some(version) => {
                    let id = VersionedSubstateId::new(input.substate_id, version);
                    let substate = store.get(&id.to_substate_address())?;
                    info!(target: LOG_TARGET, "Resolved substate: {id}");
                    resolved_substates.insert(id, substate);
                },
                None => {
                    let (id, substate) = self.resolve_local_substate::<TStateStore>(input.substate_id, store)?;
                    info!(target: LOG_TARGET, "Resolved unversioned substate: {id}");
                    resolved_substates.insert(id, substate);
                },
            }
        }
        // TODO: we assume local only transactions, we need to implement multi-shard transactions.
        //       Suggest once we have pledges for foreign substates, we add them to a temporary pledge store and use
        //       that to resolve inputs.
        Ok(resolved_substates)
    }

    fn resolve_local_substate<TStateStore: StateStore>(
        &self,
        id: SubstateId,
        store: &PendingSubstateStore<TStateStore>,
    ) -> Result<(VersionedSubstateId, Substate), BlockTransactionExecutorError> {
        let substate = store.get_latest(&id).optional()?.ok_or_else(|| {
            BlockTransactionExecutorError::UnableToResolveSubstateId {
                substate_id: id.clone(),
            }
        })?;

        Ok((VersionedSubstateId::new(id, substate.version()), substate))
    }

    fn add_substates_to_memory_db(
        &self,
        inputs: &IndexMap<VersionedSubstateId, Substate>,
        out: &MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        // TODO: pass the impl SubstateStore directly into the engine
        let mut access = out
            .write_access()
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        for (id, substate) in inputs {
            access
                .set_state(id.substate_id(), substate)
                .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        }
        access
            .commit()
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;

        Ok(())
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
}
