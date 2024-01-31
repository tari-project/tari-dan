//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_dan_common_types::Epoch;
use tari_dan_engine::state_store::memory::MemoryStateStore;
use tari_engine_types::virtual_substate::VirtualSubstates;
use tari_transaction::Transaction;

#[async_trait]
pub trait SubstateResolver {
    type Error: Send + Sync + 'static;

    async fn resolve(&self, transaction: &Transaction, out: &MemoryStateStore) -> Result<(), Self::Error>;

    async fn resolve_virtual_substates(
        &self,
        transaction: &Transaction,
        current_epoch: Epoch,
    ) -> Result<VirtualSubstates, Self::Error>;
}
