//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeMap, HashMap};

use futures::{future::join_all, FutureExt};
use tari_consensus::{
    messages::{ProposalMessage, VoteMessage},
    traits::{EpochManager, LeaderStrategy},
};
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::{
    consensus_models::{BlockId, Decision, TransactionPool},
    StateStore,
    StateStoreReadTransaction,
};

use crate::support::{
    address::TestAddress,
    epoch_manager::TestEpochManager,
    transaction::build_transaction,
    validator::Validator,
    SelectedIndexLeaderStrategy,
};

pub struct Test {
    validators: BTreeMap<TestAddress, Validator>,
    leader_strategy: SelectedIndexLeaderStrategy,
    epoch_manager: TestEpochManager,
    messages_sent: HashMap<DanMessageType, usize>,
}

impl Test {
    pub fn builder() -> TestBuilder {
        TestBuilder::new()
    }

    pub fn total_messages_sent(&self) -> usize {
        self.messages_sent.values().sum()
    }

    pub async fn do_hotstuff_round(&mut self, round: u64) {
        let (leader, proposal) = self.assert_proposal_broadcast().await;
        self.send_proposal_to_all(leader, &proposal).await;

        self.forward_all_votes_to_leader().await;
        self.wait_for_all_to_have_leaf_height(NodeHeight(round)).await;
    }

    pub async fn send_all_transaction(&self, decision: Decision, fee: u64) {
        let tx = build_transaction(decision, fee);
        for validator in self.validators.values() {
            validator.send_transaction(&tx).await;
        }
    }

    pub async fn assert_proposal_broadcast(&mut self) -> (TestAddress, ProposalMessage) {
        let leader = self.get_leader();
        let msg = self.get_validator_mut(&leader).assert_proposal_broadcast().await;
        *self.messages_sent.entry(DanMessageType::Proposal).or_default() += 1;
        (leader, msg)
    }

    pub fn get_leader(&self) -> TestAddress {
        *self
            .leader_strategy
            .get_leader(&self.validators.keys().copied().collect(), &BlockId::genesis(), 0)
    }

    pub fn get_validator_mut(&mut self, addr: &TestAddress) -> &mut Validator {
        self.validators.get_mut(addr).unwrap()
    }

    pub async fn send_proposal_to_all(&self, from: TestAddress, msg: &ProposalMessage) {
        for validator in self.validators.values() {
            validator.send_proposal(from, msg).await;
        }
    }

    pub async fn forward_all_votes_to_leader(&mut self) -> Vec<(TestAddress, VoteMessage)> {
        let votes = self.assert_votes_sent().await;
        let leader = self.get_leader();
        self.get_validator_mut(&leader).send_votes(votes.clone()).await;
        votes
    }

    pub async fn assert_votes_sent(&mut self) -> Vec<(TestAddress, VoteMessage)> {
        let leader = self.get_leader();
        let votes = join_all(self.validators.values_mut().map(|v| {
            let address = v.address;
            v.assert_vote_sent(&leader).map(move |vote| (address, vote))
        }))
        .await;
        *self.messages_sent.entry(DanMessageType::Vote).or_default() += votes.len();

        votes
    }

    pub fn is_all_transactions_committed(&self) -> bool {
        self.validators.values().all(|v| {
            let c = v.uncommitted_transaction_count();
            log::info!("{} has {} unfinalized transactions", v.address, c);
            c == 0
        })
    }

    pub fn with_all_validators(&self, f: impl FnMut(&Validator)) {
        self.validators.values().for_each(f);
    }

    pub async fn wait_until_new_pool_count(&self, count: usize) {
        self.wait_all_for_predicate(format!("new pool count to be {}", count), |v| {
            v.state_store
                .with_read_tx(|tx| tx.new_transaction_pool_get_many_ready(count))
                .unwrap()
                .len() >=
                count
        })
        .await;
    }

    pub async fn wait_for_all_to_have_leaf_height(&self, height: NodeHeight) {
        let epoch = self.epoch_manager.current_epoch().await.unwrap();
        self.wait_all_for_predicate(format!("leaf height to be {}", height), |v| {
            let leaf_height = v
                .state_store
                .with_read_tx(|tx| tx.leaf_block_get(epoch))
                .unwrap()
                .height;

            leaf_height >= height
        })
        .await;
    }

    pub async fn wait_all_have_at_least_n_transactions_in_pool(&self, n: usize, pool: TransactionPool) {
        self.wait_all_for_predicate(format!("waiting for {} transactions in pool {:?}", n, pool), |v| {
            let pool_count = v
                .state_store
                .with_read_tx(|tx| tx.transaction_pools_count(pool))
                .unwrap();

            pool_count >= n
        })
        .await;
    }

    async fn wait_all_for_predicate<P: FnMut(&Validator) -> bool>(&self, description: String, mut predicate: P) {
        let mut complete = vec![];
        let mut remaining_loops = 100usize; // ~10 seconds
        loop {
            self.with_all_validators(|v| {
                if complete.contains(&v.address) {
                    return;
                }
                if predicate(v) {
                    complete.push(v.address);
                } else if remaining_loops == 0 {
                    panic!("Timed out waiting for {}", description);
                } else {
                    // 📎
                }
            });

            remaining_loops -= 1;

            if complete.len() == self.validators.len() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

pub struct TestBuilder {
    addresses: Vec<&'static str>,
    sql_address: String,
}

impl TestBuilder {
    pub fn new() -> Self {
        Self {
            addresses: vec!["1"],
            sql_address: ":memory:".to_string(),
        }
    }

    pub fn with_sql_url<T: Into<String>>(&mut self, sql_address: T) -> &mut Self {
        self.sql_address = sql_address.into();
        self
    }

    pub fn with_addresses(&mut self, addresses: Vec<&'static str>) -> &mut Self {
        self.addresses = addresses;
        self
    }

    fn build_validators(
        &self,
        leader_strategy: &SelectedIndexLeaderStrategy,
        epoch_manager: &TestEpochManager,
    ) -> BTreeMap<TestAddress, Validator> {
        self.addresses
            .iter()
            .enumerate()
            .map(|(i, addr)| {
                let address = TestAddress(addr);
                let leader_strategy = leader_strategy.clone();
                let sql_address = self.sql_address.replace("{}", &format!("{}", i + 1));
                let validator = Validator::builder()
                    .with_sql_url(sql_address)
                    .with_address(address)
                    .with_epoch_manager(epoch_manager.copy_for(address))
                    .with_leader_strategy(leader_strategy)
                    .spawn();
                (address, validator)
            })
            .collect()
    }

    pub async fn start(&self) -> Test {
        let leader_strategy = SelectedIndexLeaderStrategy::new(0);
        let epoch_manager = TestEpochManager::new();
        epoch_manager
            .set_committee(0, self.addresses.clone().into_iter().map(TestAddress).collect())
            .await;

        Test {
            validators: self.build_validators(&leader_strategy, &epoch_manager),
            leader_strategy,
            messages_sent: Default::default(),
            epoch_manager,
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum DanMessageType {
    Proposal,
    Vote,
    NewView,
}
