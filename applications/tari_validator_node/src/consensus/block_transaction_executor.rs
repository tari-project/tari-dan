//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use log::info;
use tari_consensus::traits::{BlockTransactionExecutor, BlockTransactionExecutorError, UncommittedChainTemplates};
use tari_engine::{
    state_store::{
        StateWriter,
        memory::{MemoryStateStore, ReadOnlyMemoryStateStore},
        new_memory_store,
    },
    template::LoadedTemplate,
};
use tari_engine_types::{
    substate::Substate,
    virtual_substate::{VirtualSubstate, VirtualSubstateId, VirtualSubstates},
};
use tari_ootle_app_utilities::transaction_executor::TransactionExecutor;
use tari_ootle_common_types::{
    SubstateRequirement,
    VersionedSubstateId,
    services::template_provider::TemplateProvider,
};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{LockedEpoch, TransactionExecution, VersionedSubstateIdLockIntent},
};
use tari_ootle_template_provider::UncommittedTemplateCache;
use tari_ootle_transaction::Transaction;

use super::chain_template_provider::ChainTemplateProvider;
#[cfg(feature = "metrics")]
use crate::metrics::PrometheusExecutionMetrics;

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::block_transaction_executor";

/// Bounds the compiled templates held for templates published in uncommitted blocks.
const UNCOMMITTED_TEMPLATE_CACHE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct TariBlockTransactionExecutor<TExecutor, TTemplateProvider> {
    executor: TExecutor,
    committed_templates: TTemplateProvider,
    uncommitted_cache: UncommittedTemplateCache,
    #[cfg(feature = "metrics")]
    metrics: PrometheusExecutionMetrics,
}

impl<TExecutor, TTemplateProvider> TariBlockTransactionExecutor<TExecutor, TTemplateProvider> {
    /// `committed_templates` must resolve exactly the templates in durably committed state.
    pub fn new(
        executor: TExecutor,
        committed_templates: TTemplateProvider,
        #[cfg(feature = "metrics")] metrics: PrometheusExecutionMetrics,
    ) -> Self {
        Self {
            executor,
            committed_templates,
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

impl<TExecutor, TTemplateProvider, TStateStore> BlockTransactionExecutor<TStateStore>
    for TariBlockTransactionExecutor<TExecutor, TTemplateProvider>
where
    TStateStore: StateStore,
    TExecutor: TransactionExecutor<ReadOnlyMemoryStateStore>,
    TTemplateProvider: TemplateProvider<Template = LoadedTemplate>,
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

        let template_provider = ChainTemplateProvider::new(
            self.committed_templates.clone(),
            self.uncommitted_cache.clone(),
            templates.clone(),
        );

        // Execute the transaction and get the result
        let exec_output = self
            .executor
            .execute(
                transaction,
                template_provider,
                state_db.into_read_only(),
                virtual_substates,
                burn_rate,
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
    use std::{convert::Infallible, sync::Arc, time::Duration};

    use tari_common_types::types::PrivateKey;
    use tari_engine::wasm::DiskCachedWasmTemplateProvider;
    use tari_engine_types::{
        commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
        fees::{ExhaustBurnRate, FeeReceipt},
        published_template::PublishedTemplate,
        substate::SubstateDiff,
    };
    use tari_ootle_app_utilities::transaction_executor::ExecutionOutput;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_template_provider::{MemoryCacheTemplateProvider, TemplateConfig};
    use tari_template_builtin::{ACCOUNT_TEMPLATE_ADDRESS, get_template_builtin};
    use tari_template_lib::types::{Hash32, TemplateAddress};
    use tempfile::TempDir;

    use super::*;

    type TestStateStore = tari_state_store_rocksdb::RocksDbStateStore<String>;
    type CommittedTemplates = MemoryCacheTemplateProvider<DiskCachedWasmTemplateProvider<TestStateStore>>;

    const CALLED: TemplateAddress = TemplateAddress::from_array([7; 32]);

    /// Stands in for the engine: a transaction commits if `CALLED` resolves through the template provider it is
    /// given, and is rejected otherwise.
    struct CallingExecutor;

    impl TransactionExecutor<ReadOnlyMemoryStateStore> for CallingExecutor {
        type Error = Infallible;

        fn execute<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
            &self,
            _transaction: &Transaction,
            template_provider: TTemplateProvider,
            _state_store: ReadOnlyMemoryStateStore,
            _virtual_substates: VirtualSubstates,
            _burn_rate: ExhaustBurnRate,
        ) -> Result<ExecutionOutput, Self::Error> {
            let finalize = if template_provider.get_template(&CALLED).unwrap().is_some() {
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

    /// A block executor over the validator's committed template stack, with nothing committed.
    fn executor() -> (
        TariBlockTransactionExecutor<CallingExecutor, CommittedTemplates>,
        TempDir,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let store = TestStateStore::open(dir.path().join("db"), Default::default()).unwrap();
        let disk = DiskCachedWasmTemplateProvider::open(store, dir.path().join("wasm_cache"), u64::MAX).unwrap();
        let executor = TariBlockTransactionExecutor::new(
            CallingExecutor,
            MemoryCacheTemplateProvider::new(disk, &TemplateConfig::default()),
            #[cfg(feature = "metrics")]
            PrometheusExecutionMetrics::register(&mut Default::default()),
        );
        (executor, dir)
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

    fn execute(publishes: bool) -> TransactionExecution {
        let (executor, _dir) = executor();
        let transaction = Transaction::builder_localnet(Epoch(1)).build_and_seal(&PrivateKey::from(1u64));
        BlockTransactionExecutor::<TestStateStore>::execute(
            &executor,
            &transaction,
            LockedEpoch::new(Epoch(1), Default::default(), Default::default()),
            &HashMap::new(),
            &chain(publishes),
        )
        .unwrap()
    }

    #[test]
    fn a_template_an_uncommitted_ancestor_publishes_resolves() {
        assert!(execute(true).decision().is_commit());
    }

    #[test]
    fn a_template_the_chain_does_not_publish_is_not_found() {
        assert!(!execute(false).decision().is_commit());
    }
}
