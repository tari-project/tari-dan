//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, sync::Arc};

use log::info;
use tari_consensus::traits::{BlockTransactionExecutor, BlockTransactionExecutorError, UncommittedChainTemplates};
use tari_engine::{
    state_store::{
        StateWriter,
        memory::{MemoryStateStore, ReadOnlyMemoryStateStore},
        new_memory_store,
    },
    template::{LoadedTemplate, NoUncommittedTemplates, UncommittedTemplates},
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
use tari_template_lib::types::TemplateAddress;

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

/// The uncommitted templates of one execution's chain, compiled through the cache shared by every execution.
struct ChainTemplates {
    templates: UncommittedChainTemplates,
    cache: UncommittedTemplateCache,
}

impl UncommittedTemplates for ChainTemplates {
    fn get_template(&self, address: &TemplateAddress) -> Option<Result<LoadedTemplate, String>> {
        let published = self.templates.get(address)?;
        Some(
            self.cache
                .load(*address, published.binary.as_slice())
                .map_err(|e| e.to_string()),
        )
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
        templates: &UncommittedChainTemplates,
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

        let uncommitted_templates: Arc<dyn UncommittedTemplates> = if templates.is_empty() {
            Arc::new(NoUncommittedTemplates)
        } else {
            Arc::new(ChainTemplates {
                templates: templates.clone(),
                cache: self.uncommitted_cache.clone(),
            })
        };

        // Execute the transaction and get the result
        let exec_output = self
            .executor
            .execute(
                transaction,
                state_db.into_read_only(),
                virtual_substates,
                burn_rate,
                uncommitted_templates,
            )
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
    use tari_template_lib::types::Hash32;
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
            uncommitted_templates: Arc<dyn UncommittedTemplates>,
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

    /// The uncommitted templates of a chain that publishes `CALLED` if `publishes` is set.
    fn chain(publishes: bool) -> UncommittedChainTemplates {
        if !publishes {
            return UncommittedChainTemplates::default();
        }
        UncommittedChainTemplates::new(HashMap::from([(
            CALLED,
            Arc::new(PublishedTemplate {
                template_name: "test".try_into().unwrap(),
                author: Default::default(),
                binary: get_template_builtin(&ACCOUNT_TEMPLATE_ADDRESS)
                    .to_vec()
                    .try_into()
                    .unwrap(),
                at_epoch: 0,
                metadata_hash: None,
            }),
        )]))
    }

    fn executor() -> TariBlockTransactionExecutor<CallingExecutor> {
        TariBlockTransactionExecutor::new(
            CallingExecutor::with_nothing_committed(),
            #[cfg(feature = "metrics")]
            PrometheusExecutionMetrics::register(&mut Default::default()),
        )
    }

    fn execute(
        executor: &TariBlockTransactionExecutor<CallingExecutor>,
        templates: &UncommittedChainTemplates,
    ) -> TransactionExecution {
        let transaction = Transaction::builder_localnet(Epoch(1)).build_and_seal(&PrivateKey::from(1u64));
        BlockTransactionExecutor::<TestStateStore>::execute(
            executor,
            &transaction,
            LockedEpoch::new(Epoch(1), Default::default(), Default::default()),
            &HashMap::new(),
            templates,
        )
        .unwrap()
    }

    fn executions(executor: &TariBlockTransactionExecutor<CallingExecutor>) -> usize {
        *executor.executor.executions.lock().unwrap()
    }

    #[test]
    fn a_template_an_uncommitted_ancestor_publishes_resolves_in_one_execution() {
        let executor = executor();
        let execution = execute(&executor, &chain(true));
        assert!(execution.decision().is_commit());
        assert_eq!(executions(&executor), 1);
    }

    #[test]
    fn a_template_the_chain_does_not_publish_is_not_found() {
        let executor = executor();
        let execution = execute(&executor, &chain(false));
        assert!(!execution.decision().is_commit());
        assert_eq!(executions(&executor), 1);
    }
}
