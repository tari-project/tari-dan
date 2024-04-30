//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use indexmap::IndexSet;
use log::info;
use tari_consensus::traits::{
    BlockTransactionExecutor,
    BlockTransactionExecutorBuilder,
    BlockTransactionExecutorError,
};
use tari_dan_app_utilities::transaction_executor::TransactionExecutor;
use tari_dan_common_types::SubstateAddress;
use tari_dan_engine::{
    bootstrap_state,
    state_store::{memory::MemoryStateStore, AtomicDb, StateWriter},
};
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, SubstateLockFlag, SubstateRecord, VersionedSubstateIdLockIntent},
    StateStore,
};
use tari_engine_types::{substate::SubstateId, virtual_substate::VirtualSubstates};
use tari_epoch_manager::EpochManagerReader;
use tari_transaction::{Transaction, VersionedSubstateId};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::block_transaction_executor";

#[derive(Debug, Clone)]
pub struct TariDanBlockTransactionExecutorBuilder<TEpochManager: EpochManagerReader, TExecutor: TransactionExecutor> {
    epoch_manager: TEpochManager,
    executor: TExecutor,
}

impl<TEpochManager, TExecutor> TariDanBlockTransactionExecutorBuilder<TEpochManager, TExecutor>
where
    TEpochManager: EpochManagerReader,
    TExecutor: TransactionExecutor,
{
    pub fn new(epoch_manager: TEpochManager, executor: TExecutor) -> Self {
        Self {
            epoch_manager,
            executor,
        }
    }
}

impl<TExecutor, TStateStore, TEpochManager> BlockTransactionExecutorBuilder<TStateStore>
    for TariDanBlockTransactionExecutorBuilder<TEpochManager, TExecutor>
where
    TStateStore: StateStore,
    TExecutor: TransactionExecutor + Clone + 'static,
    TEpochManager: EpochManagerReader + Clone + 'static,
{
    type Executor = TariDanBlockTransactionExecutor<TEpochManager, TExecutor>;

    fn build(&self) -> Self::Executor {
        TariDanBlockTransactionExecutor::new(self.epoch_manager.clone(), self.executor.clone())
    }
}

#[derive(Debug, Clone)]
pub struct TariDanBlockTransactionExecutor<TEpochManager, TExecutor> {
    // TODO: we will need the epoch manager for virtual substates and other operations in the future
    #[allow(dead_code)]
    epoch_manager: TEpochManager,
    executor: TExecutor,
    // "cache" hashmap of updated outputs from previous transactions in the same block
    // so each consecutive transaction gets the most updated version for their inputs
    // TODO: store also the substate content and not only the version
    output_versions: HashMap<SubstateId, u32>,
}

impl<TEpochManager, TExecutor, TStateStore> BlockTransactionExecutor<TStateStore>
    for TariDanBlockTransactionExecutor<TEpochManager, TExecutor>
where
    TStateStore: StateStore,
    TExecutor: TransactionExecutor,
{
    fn execute(
        &mut self,
        transaction: Transaction,
        db_tx: &mut <TStateStore as StateStore>::ReadTransaction<'_>,
    ) -> Result<ExecutedTransaction, BlockTransactionExecutorError> {
        let id: tari_transaction::TransactionId = *transaction.id();

        // Get the latest input substates
        let inputs = self.resolve_substates::<TStateStore>(&transaction, db_tx)?;
        info!(target: LOG_TARGET, "Transaction {} executing. Inputs: {:?}", id, inputs);

        // Create a memory db with all the input substates, needed for the transaction execution
        let state_db = Self::new_state_db();
        self.add_substates_to_memory_db::<TStateStore>(db_tx, &inputs, &state_db)?;

        // TODO: create the virtual substates for execution
        let virtual_substates = VirtualSubstates::new();

        // Execute the transaction and get the result
        let mut executed = self
            .executor
            .execute(transaction, state_db, virtual_substates)
            .map_err(|e| BlockTransactionExecutorError::ExecutionThreadFailure(e.to_string()))?;

        // Store the output versions for future concurrent transactions in the same block
        if let Some(diff) = executed.result().finalize.accept() {
            diff.up_iter().for_each(|s| {
                self.output_versions.insert(s.0.clone(), s.1.version());
            });
        }

        // Update the resolved inputs to set the specific version, as we know it after execution
        let mut resolved_inputs = IndexSet::new();
        if let Some(diff) = executed.result().finalize.accept() {
            resolved_inputs = inputs
                .into_iter()
                .map(|versioned_id| {
                    let lock_flag = if diff.down_iter().any(|(id, _)| *id == versioned_id.substate_id) {
                        // Update all inputs that were DOWNed to be write locked
                        SubstateLockFlag::Write
                    } else {
                        // Any input not downed, gets a read lock
                        SubstateLockFlag::Read
                    };
                    VersionedSubstateIdLockIntent::new(versioned_id, lock_flag)
                })
                .collect::<IndexSet<_>>();
        }

        executed.set_resolved_inputs(resolved_inputs);

        info!(target: LOG_TARGET, "Transaction {} executed. result: {:?}", id, executed);
        Ok(executed)
    }
}

impl<TEpochManager, TExecutor> TariDanBlockTransactionExecutor<TEpochManager, TExecutor> {
    pub fn new(epoch_manager: TEpochManager, executor: TExecutor) -> Self {
        Self {
            epoch_manager,
            executor,
            output_versions: HashMap::new(),
        }
    }

    fn resolve_substates<TStateStore: StateStore>(
        &self,
        transaction: &Transaction,
        db_tx: &mut <TStateStore as StateStore>::ReadTransaction<'_>,
    ) -> Result<IndexSet<VersionedSubstateId>, BlockTransactionExecutorError> {
        let mut resolved_substates = IndexSet::with_capacity(transaction.num_unique_inputs());
        for input in transaction.all_inputs_iter() {
            let address = match input.version() {
                Some(version) => VersionedSubstateId::new(input.substate_id, version),
                None => {
                    // We try to fetch each input from the block "cache", and only hit the DB if the input has not been
                    // used in the block before
                    match self.output_versions.get(input.substate_id()) {
                        Some(version) => VersionedSubstateId::new(input.substate_id, *version),
                        None => self.resolve_local_substate::<TStateStore>(input.substate_id, db_tx)?,
                    }
                },
            };
            resolved_substates.insert(address);
        }
        // TODO: we assume local only transactions, we need to implement multi-shard transactions.
        //       Suggest once we have pledges for foreign substates, we add them to a temporary pledge store and use
        //       that to resolve inputs.
        Ok(resolved_substates)
    }

    fn resolve_local_substate<TStateStore: StateStore>(
        &self,
        id: SubstateId,
        db_tx: &mut <TStateStore as StateStore>::ReadTransaction<'_>,
    ) -> Result<VersionedSubstateId, BlockTransactionExecutorError> {
        let version = Self::get_last_substate_version::<TStateStore>(db_tx, &id)?.ok_or_else(|| {
            BlockTransactionExecutorError::UnableToResolveSubstateId {
                substate_id: id.clone(),
            }
        })?;
        Ok(VersionedSubstateId::new(id, version))
    }

    fn add_substates_to_memory_db<TStateStore: StateStore>(
        &self,
        db_tx: &mut <TStateStore as StateStore>::ReadTransaction<'_>,
        inputs: &IndexSet<VersionedSubstateId>,
        out: &MemoryStateStore,
    ) -> Result<(), BlockTransactionExecutorError> {
        let mut access = out
            .write_access()
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        for input in inputs {
            let address = input.to_substate_address();
            let substate = SubstateRecord::get(db_tx, &address)?;
            access
                .set_state(&input.substate_id, substate.into_substate())
                .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;
        }
        access
            .commit()
            .map_err(|e| BlockTransactionExecutorError::StateStoreError(e.to_string()))?;

        Ok(())
    }

    fn get_last_substate_version<TStateStore: StateStore>(
        db_tx: &mut <TStateStore as StateStore>::ReadTransaction<'_>,
        substate_id: &SubstateId,
    ) -> Result<Option<u32>, BlockTransactionExecutorError> {
        // TODO: add a DB query to fetch the latest version
        let mut version = 0;
        loop {
            let address = SubstateAddress::from_address(substate_id, version);
            info!(target: LOG_TARGET, "get_last_substate_version {}:{}, address: {}", substate_id, version, address);
            if SubstateRecord::exists(db_tx, &address)? {
                version += 1;
            } else if version > 0 {
                return Ok(Some(version - 1));
            } else {
                return Ok(None);
            }
        }
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
