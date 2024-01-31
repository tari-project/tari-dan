//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_engine::state_store::memory::MemoryStateStore;
use tari_dan_storage::consensus_models::ExecutedTransaction;
use tari_engine_types::virtual_substate::VirtualSubstates;
use tari_transaction::Transaction;

pub trait TransactionExecutor {
    type Error: std::error::Error + Send + Sync + 'static;

    fn execute(
        &self,
        transaction: Transaction,
        state_store: MemoryStateStore,
        virtual_substates: VirtualSubstates,
    ) -> Result<ExecutedTransaction, Self::Error>;
}