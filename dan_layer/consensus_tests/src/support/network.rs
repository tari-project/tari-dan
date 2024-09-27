//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{atomic::AtomicUsize, Arc},
};

use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use itertools::Itertools;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::ShardGroup;
use tari_dan_storage::consensus_models::TransactionRecord;
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::{
    sync::{
        mpsc::{self},
        watch,
        RwLock,
    },
    task,
};

use crate::support::{
    address::TestAddress,
    committee_number_to_shard_group,
    Validator,
    ValidatorChannels,
    TEST_NUM_PRESHARDS,
};

pub type MessageFilter = Box<dyn Fn(&TestAddress, &TestAddress, &HotstuffMessage) -> bool + Sync + Send + 'static>;

pub fn spawn_network(
    channels: Vec<ValidatorChannels>,
    shutdown_signal: ShutdownSignal,
    message_filter: Option<MessageFilter>,
) -> TestNetwork {
    let tx_new_transactions = channels
        .iter()
        .map(|c| {
            (
                c.address.clone(),
                (
                    c.shard_group,
                    c.num_committees,
                    c.tx_new_transactions.clone(),
                    c.state_store.clone(),
                ),
            )
        })
        .collect();
    let tx_hs_message = channels
        .iter()
        .map(|c| (c.address.clone(), c.tx_hs_message.clone()))
        .collect();
    let (rx_broadcast, rx_leader) = channels
        .into_iter()
        .map(|c| ((c.address.clone(), c.rx_broadcast), (c.address.clone(), c.rx_leader)))
        .multiunzip();
    let (tx_new_transaction, rx_new_transaction) = mpsc::channel(100);
    let (tx_network_status, network_status) = watch::channel(NetworkStatus::Paused);
    let (tx_on_message, rx_on_message) = watch::channel(None);
    let num_sent_messages = Arc::new(AtomicUsize::new(0));
    let num_filtered_messages = Arc::new(AtomicUsize::new(0));

    let offline_destinations = Arc::new(RwLock::new(Vec::new()));

    let network_task_handle = TestNetworkWorker {
        network_status,
        rx_new_transaction: Some(rx_new_transaction),
        tx_new_transactions,
        tx_hs_message,
        rx_broadcast: Some(rx_broadcast),
        rx_leader: Some(rx_leader),
        on_message: tx_on_message,
        num_sent_messages: num_sent_messages.clone(),
        num_filtered_messages: num_filtered_messages.clone(),
        transaction_store: Arc::new(Default::default()),
        offline_destinations: offline_destinations.clone(),
        shutdown_signal,
        message_filter,
    }
    .spawn();

    TestNetwork {
        network_task_handle,
        tx_new_transaction,
        network_status: tx_network_status,
        offline_destinations,
        num_sent_messages,
        num_filtered_messages,
        _on_message: rx_on_message,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkStatus {
    Started,
    Paused,
}

impl NetworkStatus {
    pub fn is_paused(self) -> bool {
        matches!(self, NetworkStatus::Paused)
    }
}

pub struct TestNetwork {
    network_task_handle: task::JoinHandle<()>,
    tx_new_transaction: mpsc::Sender<(TestVnDestination, TransactionRecord)>,
    network_status: watch::Sender<NetworkStatus>,
    offline_destinations: Arc<RwLock<Vec<TestVnDestination>>>,
    num_sent_messages: Arc<AtomicUsize>,
    num_filtered_messages: Arc<AtomicUsize>,
    _on_message: watch::Receiver<Option<HotstuffMessage>>,
}

impl TestNetwork {
    pub fn start(&self) {
        self.network_status.send(NetworkStatus::Started).unwrap();
    }

    pub fn task_handle(&self) -> &task::JoinHandle<()> {
        &self.network_task_handle
    }

    pub async fn go_offline(&self, destination: TestVnDestination) -> &Self {
        if destination.is_shard() {
            unimplemented!("Sorry :/ taking a bucket offline is not yet supported in the test harness");
        }
        self.offline_destinations.write().await.push(destination);
        self
    }

    #[allow(dead_code)]
    pub async fn on_message(&mut self) -> Option<HotstuffMessage> {
        self._on_message.changed().await.unwrap();
        self._on_message.borrow().clone()
    }

    #[allow(dead_code)]
    pub async fn pause(&self) {
        self.network_status.send(NetworkStatus::Paused).unwrap();
    }

    pub async fn send_transaction(&self, destination: TestVnDestination, tx: TransactionRecord) {
        self.tx_new_transaction.send((destination, tx)).await.unwrap();
    }

    pub fn total_messages_sent(&self) -> usize {
        self.num_sent_messages.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn total_messages_filtered(&self) -> usize {
        self.num_filtered_messages.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
pub enum TestVnDestination {
    All,
    Address(TestAddress),
    #[allow(dead_code)]
    Committee(u32),
}

impl TestVnDestination {
    pub fn is_for(&self, addr: &TestAddress, shard_group: ShardGroup, num_committees: u32) -> bool {
        match self {
            TestVnDestination::All => true,
            TestVnDestination::Address(a) => a == addr,
            TestVnDestination::Committee(b) => {
                committee_number_to_shard_group(TEST_NUM_PRESHARDS, *b, num_committees) == shard_group
            },
        }
    }

    pub fn is_for_vn(&self, vn: &Validator) -> bool {
        self.is_for(&vn.address, vn.shard_group, vn.num_committees)
    }

    pub fn is_shard(&self) -> bool {
        matches!(self, TestVnDestination::Committee(_))
    }
}

pub struct TestNetworkWorker {
    rx_new_transaction: Option<mpsc::Receiver<(TestVnDestination, TransactionRecord)>>,
    #[allow(clippy::type_complexity)]
    tx_new_transactions: HashMap<
        TestAddress,
        (
            ShardGroup,
            u32, // num_committees
            mpsc::Sender<(Transaction, usize)>,
            SqliteStateStore<TestAddress>,
        ),
    >,
    tx_hs_message: HashMap<TestAddress, mpsc::Sender<(TestAddress, HotstuffMessage)>>,
    #[allow(clippy::type_complexity)]
    rx_broadcast: Option<HashMap<TestAddress, mpsc::Receiver<(Vec<TestAddress>, HotstuffMessage)>>>,
    #[allow(clippy::type_complexity)]
    rx_leader: Option<HashMap<TestAddress, mpsc::Receiver<(TestAddress, HotstuffMessage)>>>,
    network_status: watch::Receiver<NetworkStatus>,
    on_message: watch::Sender<Option<HotstuffMessage>>,
    num_sent_messages: Arc<AtomicUsize>,
    num_filtered_messages: Arc<AtomicUsize>,
    transaction_store: Arc<RwLock<HashMap<TransactionId, TransactionRecord>>>,

    offline_destinations: Arc<RwLock<Vec<TestVnDestination>>>,
    shutdown_signal: ShutdownSignal,
    message_filter: Option<MessageFilter>,
}

impl TestNetworkWorker {
    pub fn spawn(self) -> task::JoinHandle<()> {
        tokio::spawn(self.run())
    }

    async fn run(mut self) {
        let mut rx_broadcast = self.rx_broadcast.take().unwrap();
        let mut rx_leader = self.rx_leader.take().unwrap();

        let mut rx_new_transaction = self.rx_new_transaction.take().unwrap();
        let tx_new_transactions = self.tx_new_transactions.clone();
        let transaction_store = self.transaction_store.clone();

        if self.network_status.borrow().is_paused() {
            loop {
                self.network_status.changed().await.unwrap();
                if let NetworkStatus::Started = *self.network_status.borrow() {
                    break;
                }
            }
        }

        log::info!("🚀 Network started");

        // Handle transactions that come in from the test. This behaves like a mempool.
        let mut mempool_task = tokio::spawn(async move {
            while let Some((dest, tx_record)) = rx_new_transaction.recv().await {
                let remaining = rx_new_transaction.len();
                transaction_store
                    .write()
                    .await
                    .insert(*tx_record.transaction().id(), tx_record.clone());

                for (addr, (shard_group, num_committees, tx_new_transaction_to_consensus, _)) in &tx_new_transactions {
                    if dest.is_for(addr, *shard_group, *num_committees) {
                        tx_new_transaction_to_consensus
                            .send((tx_record.transaction().clone(), remaining))
                            .await
                            .unwrap();
                        log::info!("🐞 New transaction {} for vn {}", tx_record.id(), addr);
                    } else {
                        log::debug!(
                            "ℹ️🐞 New transaction {} not destined for vn {} (dest = {:?})",
                            tx_record.id(),
                            addr,
                            dest
                        );
                    }
                }
            }
            log::info!("🛑 Mempool task stopped");
        });

        loop {
            let mut rx_broadcast = rx_broadcast
                .iter_mut()
                .map(|(from, rx)| rx.recv().map(|r| (from.clone(), r)))
                .collect::<FuturesUnordered<_>>();
            let mut rx_leader = rx_leader
                .iter_mut()
                .map(|(from, rx)| rx.recv().map(|r| (from.clone(), r)))
                .collect::<FuturesUnordered<_>>();

            tokio::select! {
                biased;

                  _ = self.shutdown_signal.wait() => {
                    break;
                }
                result = &mut mempool_task => {
                    result.expect("Test Mempool task failed");
                    break;
                }

                Ok(_) = self.network_status.changed() => {
                    if let NetworkStatus::Started = *self.network_status.borrow() {
                        continue;
                    }
                    loop{
                        self.network_status.changed().await.unwrap();
                        if let NetworkStatus::Started = *self.network_status.borrow() {
                            break;
                        }
                    }
                }

                Some((from, Some((to, msg)))) = rx_broadcast.next() => self.handle_broadcast(from, to, msg).await,
                Some((from, Some((to, msg)))) = rx_leader.next() => self.handle_leader(from, to, msg).await,
            }
        }

        log::info!("🛑 Network stopped");
    }

    pub async fn handle_broadcast(&mut self, from: TestAddress, to_addrs: Vec<TestAddress>, msg: HotstuffMessage) {
        log::debug!("✉️ Broadcast {} from {} to {}", msg, from, to_addrs.iter().join(", "));
        for to in to_addrs {
            if let Some(message_filter) = &self.message_filter {
                if !message_filter(&from, &to, &msg) {
                    self.num_filtered_messages
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
            }
            // TODO: support for taking a whole committee bucket offline
            if to != from &&
                self.is_offline_destination(&from, &to, ShardGroup::all_shards(TEST_NUM_PRESHARDS))
                    .await
            {
                continue;
            }

            self.tx_hs_message
                .get(&to)
                .unwrap()
                .send((from.clone(), msg.clone()))
                .await
                .unwrap();
            self.num_sent_messages
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        self.on_message.send(Some(msg.clone())).unwrap();
    }

    async fn handle_leader(&mut self, from: TestAddress, to: TestAddress, msg: HotstuffMessage) {
        if let Some(message_filter) = &self.message_filter {
            if !message_filter(&from, &to, &msg) {
                self.num_filtered_messages
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        }
        log::debug!("✉️ Message {} from {} to {}", msg, from, to);
        if from != to &&
            self.is_offline_destination(&from, &to, ShardGroup::all_shards(TEST_NUM_PRESHARDS))
                .await
        {
            log::info!("🗑️ Discarding message {msg}. Leader {from} is offline");
            return;
        }
        self.on_message.send(Some(msg.clone())).unwrap();
        self.num_sent_messages
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.tx_hs_message.get(&to).unwrap().send((from, msg)).await.unwrap();
    }

    async fn is_offline_destination(&self, from: &TestAddress, to: &TestAddress, shard: ShardGroup) -> bool {
        let lock = self.offline_destinations.read().await;
        // 99999 is not used TODO: support for taking entire shard group offline
        lock.iter()
            .any(|d| d.is_for(from, shard, 99999) || d.is_for(to, shard, 99999))
    }
}
