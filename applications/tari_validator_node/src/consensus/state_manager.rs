//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::StateManager;
use tari_dan_common_types::ShardId;
use tari_dan_storage::{
    consensus_models::{Block, ExecutedTransaction, SubstateRecord},
    StateStore,
    StateStoreWriteTransaction,
    StorageError,
};

pub struct TariStateManager;

impl TariStateManager {
    pub fn new() -> Self {
        Self
    }
}

impl<TStateStore: StateStore> StateManager<TStateStore> for TariStateManager {
    type Error = StorageError;

    fn commit_transaction(
        &self,
        tx: &mut TStateStore::WriteTransaction<'_>,
        block: &Block,
        transaction: &ExecutedTransaction,
    ) -> Result<(), Self::Error> {
        let Some(diff) =transaction.result().finalize.result.accept() else {
            // We should only commit accepted transactions, might want to change this API to reflect that
            return Ok(());
        };

        let down_shards = diff
            .down_iter()
            .map(|(addr, version)| ShardId::from_address(addr, *version));
        tx.substate_down_many(down_shards, block.epoch(), block.id(), transaction.transaction().id())?;

        let to_up = diff.up_iter().map(|(addr, substate)| {
            SubstateRecord::new(
                addr.clone(),
                substate.version(),
                substate.substate_value().clone(),
                block.epoch(),
                block.height(),
                *block.id(),
                *transaction.transaction().id(),
                *block.justify().id(),
            )
        });

        for up in to_up {
            up.create(tx)?;
        }

        Ok(())
    }
}
