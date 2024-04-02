//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::{hotstuff::HotstuffEvent, messages::HotstuffMessage};
use tari_dan_common_types::{shard::Shard, SubstateAddress};
use tari_dan_storage::{consensus_models::LeafBlock, StateStore, StateStoreReadTransaction};
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    RoundRobinLeaderStrategy,
    TestStateManager,
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
    pub shard: SubstateAddress,

    pub state_store: SqliteStateStore<TestAddress>,
    pub epoch_manager: TestEpochManager,
    pub leader_strategy: RoundRobinLeaderStrategy,
    pub events: broadcast::Receiver<HotstuffEvent>,
    pub state_manager: TestStateManager,

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

    pub fn state_manager(&self) -> &TestStateManager {
        &self.state_manager
    }

    pub fn get_transaction_pool_count(&self) -> usize {
        self.state_store
            .with_read_tx(|tx| tx.transaction_pool_count(None, None, None))
            .unwrap()
    }

    pub fn get_leaf_block(&self) -> LeafBlock {
        self.state_store.with_read_tx(|tx| LeafBlock::get(tx)).unwrap()
    }
}
