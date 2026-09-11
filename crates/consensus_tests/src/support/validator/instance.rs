//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::{
    hotstuff::{ConsensusCurrentState, CurrentView, HotstuffEvent},
    messages::HotstuffMessage,
};
use tari_consensus_types::{BlockId, LeafBlock};
use tari_ootle_common_types::{NodeHeight, ShardGroup, SubstateAddress, VersionedSubstateIdRef, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StorageError,
    consensus_models::{BookkeepingModel, TransactionExecution, TransactionPool},
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};

use crate::support::{
    TestBlockTransactionProcessor,
    TestStore,
    ValidatorBuilder,
    address::TestAddress,
    epoch_manager::TestEpochManager,
};

pub struct ValidatorChannels {
    pub address: TestAddress,
    pub shard_group: ShardGroup,
    pub num_committees: u32,
    pub state_store: TestStore,

    pub tx_new_transactions: mpsc::Sender<(Transaction, usize)>,
    pub tx_hs_message: mpsc::Sender<(TestAddress, HotstuffMessage)>,
    pub rx_broadcast: mpsc::Receiver<(Vec<TestAddress>, HotstuffMessage)>,
    pub rx_leader: mpsc::Receiver<(TestAddress, HotstuffMessage)>,
}

pub struct Validator {
    pub address: TestAddress,
    pub _shard_address: SubstateAddress,
    pub shard_group: ShardGroup,
    pub num_committees: u32,

    pub _current_view: CurrentView,
    pub state_store: TestStore,
    pub transaction_executor: TestBlockTransactionProcessor,
    pub epoch_manager: TestEpochManager,
    pub events: broadcast::Receiver<HotstuffEvent>,
    pub current_state_machine_state: watch::Receiver<ConsensusCurrentState>,

    pub handle: JoinHandle<()>,
}

impl Validator {
    pub fn builder() -> ValidatorBuilder {
        ValidatorBuilder::new()
    }

    pub fn state_store(&self) -> &TestStore {
        &self.state_store
    }

    pub fn get_transaction_pool_count(&self) -> usize {
        self.state_store
            .with_read_tx(|tx| tx.transaction_pool_count(None, None, false))
            .unwrap()
    }

    /// Drops every transaction pool record, as consensus does on entering `Syncing` after a state sync.
    pub fn clear_transaction_pool(&self) -> usize {
        let pool = TransactionPool::<TestStore>::new();
        self.state_store
            .with_write_tx(|tx| {
                let recs = pool.get_all(&**tx, usize::MAX)?;
                let ids = recs.iter().map(|rec| rec.id()).collect::<Vec<_>>();
                pool.remove_all(tx, ids)
            })
            .unwrap()
            .len()
    }

    pub fn current_state_machine_state(&self) -> ConsensusCurrentState {
        *self.current_state_machine_state.borrow()
    }

    pub fn get_leaf_block(&self) -> LeafBlock {
        let epoch = self.epoch_manager.get_current_epoch();
        self.state_store
            .with_read_tx(|tx| LeafBlock::get(tx, epoch))
            .optional()
            .unwrap()
            .unwrap_or_else(|| LeafBlock {
                block_id: BlockId::zero(),
                height: NodeHeight::zero(),
                epoch,
                shard_group: self.shard_group,
            })
    }

    /// Returns true once the transaction has been received by this validator: either it is still in the
    /// transaction pool or it has already been finalized and removed from it.
    pub fn has_seen_transaction(&self, tx_id: &TransactionId) -> bool {
        self.state_store()
            .with_read_tx(|tx| {
                Ok::<_, StorageError>(tx.transaction_pool_exists(tx_id)? || tx.transactions_exists(tx_id)?)
            })
            .unwrap()
    }

    pub fn has_committed_substates(&self, tx_id: &TransactionId) -> bool {
        let tx = self.state_store().create_read_tx().unwrap();
        let Some(tx_rec) = tx.transactions_get(tx_id).optional().unwrap() else {
            return false;
        };
        let Some(exec_result) = tx_rec.get_finalized_execution(&tx).optional().unwrap() else {
            return false;
        };
        if let Some(diff) = exec_result.result().finalize.any_accept() {
            for (substate_id, substate) in diff.up_iter() {
                let id = VersionedSubstateIdRef::new(substate_id, substate.version());
                if tx.substates_any_exist(Some(id)).unwrap() {
                    return true;
                }
            }
        }
        false
    }

    pub fn get_transaction_execution(&self, transaction_id: &TransactionId) -> TransactionExecution {
        self.state_store
            .with_read_tx(|tx| TransactionExecution::get_finalized(tx, transaction_id))
            .unwrap()
    }
}
