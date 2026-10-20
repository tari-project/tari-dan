//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use log::info;
use tari_consensus::traits::{BlockTransactionExecutor, BlockTransactionExecutorError, ParentChainTemplates};
use tari_engine::{
    state_store::{
        StateWriter,
        memory::{MemoryStateStore, ReadOnlyMemoryStateStore},
        new_memory_store,
    },
    template::UncommittedTemplates,
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
use tari_ootle_template_provider::UncommittedTemplateCache;
use tari_ootle_transaction::Transaction;

#[cfg(feature = "metrics")]
use crate::metrics::PrometheusExecutionMetrics;

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::block_transaction_executor";

/// Bounds the compiled templates held for templates published in uncommitted blocks.
const UNCOMMITTED_TEMPLATE_CACHE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct TariBlockTransactionExecutor<TExecutor> {
    executor: TExecutor,
    uncommitted_cache: UncommittedTemplateCache,
    #[cfg(feature = "metrics")]
    metrics: PrometheusExecutionMetrics,
}

impl<TExecutor: TransactionExecutor<ReadOnlyMemoryStateStore>> TariBlockTransactionExecutor<TExecutor> {
    pub fn new(executor: TExecutor, #[cfg(feature = "metrics")] metrics: PrometheusExecutionMetrics) -> Self {
        Self {
            executor,
            uncommitted_cache: UncommittedTemplateCache::new(UNCOMMITTED_TEMPLATE_CACHE_BYTES),
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
        templates: &dyn ParentChainTemplates,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        let id = transaction.calculate_id();

        info!(target: LOG_TARGET, "Transaction {} executing. {} input(s)", id, resolved_inputs.len());

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

        // The executor resolves templates from committed state, which may stop short of the chain this block builds
        // on. A template it misses that the chain publishes is resolved from the chain and the transaction executed
        // again, until every template it misses is one the chain does not publish either.
        let mut uncommitted_templates = UncommittedTemplates::default();
        let exec_output = loop {
            // Create a memory db with all the input substates, needed for the transaction execution
            let mut state_db = new_memory_store();
            Self::add_substates_to_memory_db(resolved_inputs, &mut state_db)?;

            let exec_output = self
                .executor
                .execute(
                    transaction,
                    state_db.into_read_only(),
                    virtual_substates.clone(),
                    burn_rate,
                    uncommitted_templates.clone(),
                )
                .map_err(|e| BlockTransactionExecutorError::ExecutionThreadFailure(e.to_string()))?;

            let mut resolved_any = false;
            for address in uncommitted_templates.take_misses() {
                if let Some(published) = templates.get_published_template(&address)? {
                    info!(
                        target: LOG_TARGET,
                        "Transaction {} calls template {} from the uncommitted chain; executing again",
                        id,
                        address,
                    );
                    let loaded = self
                        .uncommitted_cache
                        .load(address, published.binary.as_slice())
                        .map_err(|e| e.to_string());
                    uncommitted_templates = uncommitted_templates.with_template(address, loaded);
                    resolved_any = true;
                }
            }
            if !resolved_any {
                break exec_output;
            }
        };

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

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use tari_common_types::types::PrivateKey;
    use tari_engine::{template::ExecutionTemplateProvider, wasm::DiskCachedWasmTemplateProvider};
    use tari_engine_types::{
        commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
        fees::{ExhaustBurnRate, FeeReceipt},
        published_template::PublishedTemplate,
        substate::SubstateDiff,
    };
    use tari_ootle_app_utilities::transaction_executor::ExecutionOutput;
    use tari_ootle_common_types::{Epoch, services::template_provider::TemplateProvider};
    use tari_ootle_template_provider::{MemoryCacheTemplateProvider, TemplateConfig};
    use tari_template_builtin::{ACCOUNT_TEMPLATE_ADDRESS, get_template_builtin};
    use tari_template_lib::types::{Hash32, TemplateAddress};
    use tempfile::TempDir;

    use super::*;

    type TestStateStore = tari_state_store_rocksdb::RocksDbStateStore<String>;
    type CommittedTemplates = MemoryCacheTemplateProvider<DiskCachedWasmTemplateProvider<TestStateStore>>;

    const CALLED: TemplateAddress = TemplateAddress::from_array([7; 32]);

    /// Stands in for the engine: a transaction commits if `CALLED` resolves through the validator's template
    /// provider stack, and is rejected otherwise.
    struct CallingExecutor {
        committed: Arc<CommittedTemplates>,
        executions: Mutex<usize>,
        _dir: TempDir,
    }

    impl CallingExecutor {
        /// Committed state holds no templates.
        fn with_nothing_committed() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = TestStateStore::open(dir.path().join("db"), Default::default()).unwrap();
            let disk = DiskCachedWasmTemplateProvider::open(store, dir.path().join("wasm_cache"), u64::MAX).unwrap();
            Self {
                committed: Arc::new(MemoryCacheTemplateProvider::new(disk, &TemplateConfig::default())),
                executions: Mutex::new(0),
                _dir: dir,
            }
        }
    }

    impl TransactionExecutor<ReadOnlyMemoryStateStore> for CallingExecutor {
        type Error = Infallible;

        fn execute(
            &self,
            _transaction: &Transaction,
            _state_store: ReadOnlyMemoryStateStore,
            _virtual_substates: VirtualSubstates,
            _burn_rate: ExhaustBurnRate,
            uncommitted_templates: UncommittedTemplates,
        ) -> Result<ExecutionOutput, Self::Error> {
            *self.executions.lock().unwrap() += 1;
            let provider = ExecutionTemplateProvider::new(self.committed.clone(), uncommitted_templates);
            let finalize = if provider.get_template(&CALLED).unwrap().is_some() {
                FinalizeResult::new(
                    Hash32::default(),
                    vec![],
                    vec![],
                    TransactionResult::Accept(SubstateDiff::new()),
                    FeeReceipt::default(),
                )
            } else {
                FinalizeResult::new_rejected(Hash32::default(), RejectReason::SubstateNotFound(CALLED.to_string()))
            };
            Ok(ExecutionOutput {
                result: ExecuteResult {
                    finalize,
                    execution_time: Duration::ZERO,
                    execute_epoch: None,
                    wasm_execution_points: 0,
                    native_execution_points: 0,
                },
            })
        }
    }

    /// A parent chain that publishes `CALLED` if `publishes` is set.
    struct Chain {
        publishes: bool,
    }

    impl ParentChainTemplates for Chain {
        fn get_published_template(
            &self,
            address: &TemplateAddress,
        ) -> Result<Option<PublishedTemplate>, BlockTransactionExecutorError> {
            if !self.publishes || *address != CALLED {
                return Ok(None);
            }
            Ok(Some(PublishedTemplate {
                template_name: "test".try_into().unwrap(),
                author: Default::default(),
                binary: get_template_builtin(&ACCOUNT_TEMPLATE_ADDRESS)
                    .to_vec()
                    .try_into()
                    .unwrap(),
                at_epoch: 0,
                metadata_hash: None,
            }))
        }
    }

    fn executor() -> TariBlockTransactionExecutor<CallingExecutor> {
        TariBlockTransactionExecutor::new(
            CallingExecutor::with_nothing_committed(),
            #[cfg(feature = "metrics")]
            PrometheusExecutionMetrics::register(&mut Default::default()),
        )
    }

    fn execute(executor: &TariBlockTransactionExecutor<CallingExecutor>, chain: &Chain) -> TransactionExecution {
        let transaction = Transaction::builder_localnet(Epoch(1)).build_and_seal(&PrivateKey::from(1u64));
        BlockTransactionExecutor::<TestStateStore>::execute(
            executor,
            &transaction,
            LockedEpoch::new(Epoch(1), Default::default(), Default::default()),
            &HashMap::new(),
            chain,
        )
        .unwrap()
    }

    fn executions(executor: &TariBlockTransactionExecutor<CallingExecutor>) -> usize {
        *executor.executor.executions.lock().unwrap()
    }

    #[test]
    fn a_template_the_parent_chain_publishes_resolves_though_it_is_not_committed() {
        let executor = executor();
        let execution = execute(&executor, &Chain { publishes: true });
        assert!(execution.decision().is_commit());
        assert_eq!(executions(&executor), 2);
    }

    #[test]
    fn a_template_the_parent_chain_does_not_publish_is_not_found() {
        let executor = executor();
        let execution = execute(&executor, &Chain { publishes: false });
        assert!(!execution.decision().is_commit());
        assert_eq!(executions(&executor), 1);
    }
}
