//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_consensus::messages::{HotstuffMessage, ProposalMessage, VoteMessage};
use tari_dan_common_types::committee::Committee;
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, TransactionPool},
    StateStore,
    StateStoreReadTransaction,
};
use tari_shutdown::Shutdown;
use tari_state_store_sqlite::SqliteStateStore;
use tokio::{sync::mpsc, task::JoinHandle, time::timeout};

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    SelectedIndexLeaderStrategy,
    ValidatorBuilder,
};

pub struct Validator {
    pub address: TestAddress,

    pub tx_new_transactions: mpsc::Sender<ExecutedTransaction>,
    pub tx_hs_message: mpsc::Sender<(TestAddress, HotstuffMessage)>,
    pub rx_broadcast: mpsc::Receiver<(Committee<TestAddress>, HotstuffMessage)>,
    pub rx_leader: mpsc::Receiver<(TestAddress, HotstuffMessage)>,

    pub state_store: SqliteStateStore,
    pub epoch_manager: TestEpochManager,
    pub leader_strategy: SelectedIndexLeaderStrategy,
    pub shutdown: Shutdown,

    pub handle: JoinHandle<()>,
}

impl Validator {
    pub fn builder() -> ValidatorBuilder {
        ValidatorBuilder::new()
    }

    pub async fn send_transaction(&self, tx: &ExecutedTransaction) {
        self.tx_new_transactions.send(tx.clone()).await.unwrap();
    }

    pub async fn assert_proposal_broadcast(&mut self) -> ProposalMessage {
        let (_, hs_message) = recv_timeout(&mut self.rx_broadcast)
            .await
            .unwrap_or_else(|| panic!("{}(leader) Did not broadcast proposal", self.address));
        match hs_message {
            HotstuffMessage::Proposal(msg) => msg,
            msg => panic!("Unexpected msg {:?}", msg),
        }
    }

    pub async fn send_proposal(&self, from: TestAddress, msg: &ProposalMessage) {
        self.tx_hs_message
            .send((from, HotstuffMessage::Proposal(msg.clone())))
            .await
            .unwrap();
    }

    pub async fn assert_vote_sent(&mut self, leader: &TestAddress) -> VoteMessage {
        let (dest, hs_message) = recv_timeout(&mut self.rx_leader)
            .await
            .unwrap_or_else(|| panic!("{} No vote sent to {}", self.address, leader));
        match hs_message {
            HotstuffMessage::Vote(msg) => {
                assert_eq!(
                    dest, *leader,
                    "Vote was sent to {} but expected it to be sent to leader {}",
                    dest, leader
                );
                msg
            },
            msg => panic!("Unexpected msg {:?}", msg),
        }
    }

    pub async fn send_votes(&self, votes: Vec<(TestAddress, VoteMessage)>) {
        for (from, vote) in votes {
            self.tx_hs_message
                .send((from, HotstuffMessage::Vote(vote)))
                .await
                .unwrap();
        }
    }

    pub fn leader_strategy(&self) -> &SelectedIndexLeaderStrategy {
        &self.leader_strategy
    }

    pub fn uncommitted_transaction_count(&self) -> usize {
        self.state_store
            .with_read_tx(|tx| tx.transaction_pools_count(TransactionPool::All))
            .unwrap()
    }
}

async fn recv_timeout<T>(rx: &mut mpsc::Receiver<T>) -> Option<T> {
    timeout(Duration::from_secs(10), rx.recv()).await.ok()?
}
