//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::{
    hotstuff::{ConsensusCurrentState, HotstuffEvent},
    messages::HotstuffMessage,
};
use tari_dan_common_types::{shard::Shard, SubstateAddress};
use tari_dan_storage::{consensus_models::LeafBlock, StateStore, StateStoreReadTransaction};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    RoundRobinLeaderStrategy,
    ValidatorBuilder,
};

pub struct ValidatorChannels {
    pub address: TestAddress,
    pub bucket: Shard,
    pub state_store: SqliteStateStore<TestAddress>,

    pub tx_new_transactions: mpsc::Sender<(TransactionId, usize)>,
    pub tx_hs_message: mpsc::Sender<(TestAddress, HotstuffMessage)>,
    pub rx_broadcast: mpsc::Receiver<(Vec<TestAddress>, HotstuffMessage)>,
    pub rx_leader: mpsc::Receiver<(TestAddress, HotstuffMessage)>,
    pub rx_mempool: mpsc::UnboundedReceiver<Transaction>,
}

pub struct Validator {
    pub address: TestAddress,
    pub substate_address: SubstateAddress,

    pub state_store: SqliteStateStore<TestAddress>,
    pub epoch_manager: TestEpochManager,
    pub leader_strategy: RoundRobinLeaderStrategy,
    pub events: broadcast::Receiver<HotstuffEvent>,
    pub current_state_machine_state: watch::Receiver<ConsensusCurrentState>,

    pub handle: JoinHandle<()>,
}

impl Validator {
    pub fn builder() -> ValidatorBuilder {
        ValidatorBuilder::new()
    }

    #[allow(dead_code)]
    pub fn leader_strategy(&self) -> &RoundRobinLeaderStrategy {
        &self.leader_strategy
    }

    pub fn state_store(&self) -> &SqliteStateStore<TestAddress> {
        &self.state_store
    }

    pub fn get_transaction_pool_count(&self) -> usize {
        self.state_store
            .with_read_tx(|tx| tx.transaction_pool_count(None, None, None))
            .unwrap()
    }

    pub fn current_state_machine_state(&self) -> ConsensusCurrentState {
        *self.current_state_machine_state.borrow()
    }

    pub fn get_leaf_block(&self) -> LeafBlock {
        self.state_store.with_read_tx(|tx| LeafBlock::get(tx)).unwrap()
    }

    pub fn has_committed_substates(&self) -> bool {
        let tx = self.state_store().create_read_tx().unwrap();
        assert_eq!(
            tx.transaction_pool_count(None, None, None).unwrap(),
            0,
            "Transaction pool is not empty in {}",
            self.address
        );

        tx.substates_count().unwrap() > 0
    }
}
