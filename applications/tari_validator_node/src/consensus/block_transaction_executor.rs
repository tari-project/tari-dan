//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use log::info;
use tari_consensus::traits::{BlockTransactionExecutor, BlockTransactionExecutorError};
use tari_engine::state_store::{
    StateWriter,
    memory::{MemoryStateStore, ReadOnlyMemoryStateStore},
    new_memory_store,
};
use tari_engine_types::{
    substate::Substate,
    virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates},
};
use tari_ootle_app_utilities::transaction_executor::TransactionExecutor;
use tari_ootle_common_types::{SubstateRequirement, VersionedSubstateId};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{LockedEpoch, TransactionExecution, VersionedSubstateIdLockIntent},
};
use tari_ootle_transaction::Transaction;

#[cfg(feature = "metrics")]
use crate::metrics::PrometheusExecutionMetrics;

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::block_transaction_executor";

#[derive(Debug, Clone)]
pub struct TariBlockTransactionExecutor<TExecutor> {
    executor: TExecutor,
    #[cfg(feature = "metrics")]
    metrics: PrometheusExecutionMetrics,
}

impl<TExecutor: TransactionExecutor<ReadOnlyMemoryStateStore>> TariBlockTransactionExecutor<TExecutor> {
    pub fn new(executor: TExecutor, #[cfg(feature = "metrics")] metrics: PrometheusExecutionMetrics) -> Self {
        Self {
            executor,
            #[cfg(feature = "metrics")]
            metrics,
        }
    }

    fn add_substates_to_memory_db<'a, I: IntoIterator<Item = (&'a SubstateRequirement, &'a Substate)>>(
        inputs: I,
        out: &mut MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        // TODO: pass the SubstateStore directly into the engine
        for (id, substate) in inputs {
            out.set_state(id.substate_id().clone(), substate.clone())
                .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        }

        Ok(())
    }
}

impl<TExecutor, TStateStore> BlockTransactionExecutor<TStateStore> for TariBlockTransactionExecutor<TExecutor>
where
    TStateStore: StateStore,
    TExecutor: TransactionExecutor<ReadOnlyMemoryStateStore>,
{
    fn execute(
        &self,
        transaction: &Transaction,
        locked_epoch: LockedEpoch,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        let id = transaction.calculate_id();

        info!(target: LOG_TARGET, "Transaction {} executing. {} input(s)", id, resolved_inputs.len());

        // Create a memory db with all the input substates, needed for the transaction execution
        let mut state_db = new_memory_store();
        Self::add_substates_to_memory_db(resolved_inputs, &mut state_db)?;

        let (execute_epoch, execute_epoch_hash, burn_rate) = locked_epoch.destructure();
        let virtual_substates = VirtualSubstates::from_iter([
            (
                VirtualSubstateId::CurrentEpoch,
                VirtualSubstate::CurrentEpoch(execute_epoch.as_u64()),
            ),
            (
                VirtualSubstateId::CurrentEpochHash,
                VirtualSubstate::CurrentEpochHash(execute_epoch_hash),
            ),
        ]);

        // Execute the transaction and get the result
        let exec_output = self
            .executor
            .execute(transaction, state_db.into_read_only(), virtual_substates, burn_rate)
            .map_err(|e| BlockTransactionExecutorError::ExecutionThreadFailure(e.to_string()))?;

        #[cfg(feature = "metrics")]
        self.metrics
            .on_executed(&exec_output.result.finalize.result, exec_output.result.execution_time);

        // Generate the resolved inputs to set the specific version and required lock flag, as we know it after
        // execution
        let resolved_inputs = exec_output.resolve_input_locks(resolved_inputs);

        let resulting_outputs = exec_output
            .result
            .finalize
            .result
            .any_accept()
            .map(|diff| {
                diff.up_iter()
                    .map(|(addr, substate)| {
                        VersionedSubstateIdLockIntent::output(VersionedSubstateId::new(
                            addr.clone(),
                            substate.version(),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let exec = TransactionExecution::new(exec_output.result, resolved_inputs, resulting_outputs);

        info!(target: LOG_TARGET, "Transaction {} executed in {:.2?}. {}", id, exec.result().execution_time, exec.result().finalize.result);
        Ok(exec)
    }
}
