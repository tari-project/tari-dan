//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::StateManager;
use tari_dan_common_types::{committee::CommitteeShard, SubstateAddress};
use tari_dan_storage::{
    consensus_models::{Block, ExecutedTransaction, SubstateRecord},
    StateStore,
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
        local_committee_shard: &CommitteeShard,
    ) -> Result<(), Self::Error> {
        let Some(diff) = transaction.result().finalize.result.accept() else {
            // We should only commit accepted transactions, might want to change this API to reflect that
            return Ok(());
        };

        let down_shards = diff
            .down_iter()
            .map(|(addr, version)| SubstateAddress::from_address(addr, *version))
            .filter(|shard| local_committee_shard.includes_substate_address(shard));
        SubstateRecord::destroy_many(
            tx,
            down_shards,
            block.epoch(),
            block.id(),
            block.justify().id(),
            transaction.id(),
            true,
        )?;

        let to_up = diff.up_iter().filter_map(|(addr, substate)| {
            let address = SubstateAddress::from_address(addr, substate.version());
            // Commit all substates included in this shard. Every involved validator commits the transaction receipt.
            if local_committee_shard.includes_substate_address(&address) || addr.is_transaction_receipt() {
                Some(SubstateRecord::new(
                    addr.clone(),
                    substate.version(),
                    substate.substate_value().clone(),
                    block.epoch(),
                    block.height(),
                    *block.id(),
                    *transaction.id(),
                    *block.justify().id(),
                ))
            } else {
                None
            }
        });

        for up in to_up {
            up.create(tx)?;
        }

        Ok(())
    }
}
