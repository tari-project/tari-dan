//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::{hotstuff::HotstuffEvent, messages::HotstuffMessage};
use tari_dan_common_types::{committee::Committee, Epoch, ShardId};
use tari_dan_storage::{
    consensus_models::{BlockId, ExecutedTransaction, LeafBlock},
    StateStore,
    StateStoreReadTransaction,
};
use tari_epoch_manager::EpochManagerEvent;
use tari_shutdown::Shutdown;
use tari_state_store_sqlite::SqliteStateStore;
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    SelectedIndexLeaderStrategy,
    ValidatorBuilder,
};

pub struct ValidatorChannels {
    pub address: TestAddress,

    pub tx_new_transactions: mpsc::Sender<ExecutedTransaction>,
    pub tx_hs_message: mpsc::Sender<(TestAddress, HotstuffMessage)>,
    pub rx_broadcast: mpsc::Receiver<(Committee<TestAddress>, HotstuffMessage)>,
    pub rx_leader: mpsc::Receiver<(TestAddress, HotstuffMessage)>,
}

pub struct Validator {
    pub address: TestAddress,
    pub shard: ShardId,

    pub state_store: SqliteStateStore,
    pub epoch_manager: TestEpochManager,
    pub leader_strategy: SelectedIndexLeaderStrategy,
    pub shutdown: Shutdown,
    pub events: broadcast::Receiver<HotstuffEvent>,
    pub tx_epoch_events: broadcast::Sender<EpochManagerEvent>,

    pub handle: JoinHandle<()>,
}

impl Validator {
    pub fn builder() -> ValidatorBuilder {
        ValidatorBuilder::new()
    }

    #[allow(dead_code)]
    pub fn leader_strategy(&self) -> &SelectedIndexLeaderStrategy {
        &self.leader_strategy
    }

    pub fn uncommitted_transaction_count(&self) -> usize {
        self.state_store
            .with_read_tx(|tx| tx.transaction_pool_count(None, None))
            .unwrap()
    }

    #[allow(dead_code)]
    pub async fn on_block_committed(&mut self) -> BlockId {
        loop {
            let event = self.events.recv().await.unwrap();
            #[allow(clippy::single_match)]
            match event {
                HotstuffEvent::BlockCommitted { block_id } => break block_id,
                _ => {},
            }
        }
    }

    pub fn get_leaf_block(&self) -> LeafBlock {
        self.state_store
            .with_read_tx(|tx| LeafBlock::get(tx, Epoch(0)))
            .unwrap()
    }
}
