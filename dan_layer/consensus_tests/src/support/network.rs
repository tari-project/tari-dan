//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{atomic::AtomicUsize, Arc},
};

use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use itertools::Itertools;
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::shard::Shard;
use tari_dan_storage::{
    consensus_models::{ExecutedTransaction, TransactionPool},
    StateStore,
};
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::{
    mpsc::{self},
    watch,
    RwLock,
};

use crate::support::{address::TestAddress, ValidatorChannels};

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
                (c.bucket, c.tx_new_transactions.clone(), c.state_store.clone()),
            )
        })
        .collect();
    let tx_hs_message = channels
        .iter()
        .map(|c| (c.address.clone(), c.tx_hs_message.clone()))
        .collect();
    let (rx_broadcast, rx_leader, rx_mempool) = channels
        .into_iter()
        .map(|c| {
            (
                (c.address.clone(), c.rx_broadcast),
                (c.address.clone(), c.rx_leader),
                (c.address, c.rx_mempool),
            )
        })
        .multiunzip();
    let (tx_new_transaction, rx_new_transaction) = mpsc::channel(100);
    let (tx_network_status, network_status) = watch::channel(NetworkStatus::Paused);
    let (tx_on_message, rx_on_message) = watch::channel(None);
    let num_sent_messages = Arc::new(AtomicUsize::new(0));
    let num_filtered_messages = Arc::new(AtomicUsize::new(0));

    let offline_destinations = Arc::new(RwLock::new(Vec::new()));

    TestNetworkWorker {
        network_status,
        rx_new_transaction: Some(rx_new_transaction),
        tx_new_transactions,
        tx_hs_message,
        rx_broadcast: Some(rx_broadcast),
        rx_leader: Some(rx_leader),
        rx_mempool: Some(rx_mempool),
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
    tx_new_transaction: mpsc::Sender<(TestNetworkDestination, ExecutedTransaction)>,
    network_status: watch::Sender<NetworkStatus>,
    offline_destinations: Arc<RwLock<Vec<TestNetworkDestination>>>,
    num_sent_messages: Arc<AtomicUsize>,
    num_filtered_messages: Arc<AtomicUsize>,
    _on_message: watch::Receiver<Option<HotstuffMessage>>,
}

impl TestNetwork {
    pub fn start(&self) {
        self.network_status.send(NetworkStatus::Started).unwrap();
    }

    pub async fn go_offline(&self, destination: TestNetworkDestination) -> &Self {
        if destination.is_bucket() {
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

    pub async fn send_transaction(&self, destination: TestNetworkDestination, tx: ExecutedTransaction) {
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
pub enum TestNetworkDestination {
    All,
    Address(TestAddress),
    Bucket(u32),
}

impl TestNetworkDestination {
    pub fn is_for(&self, addr: &TestAddress, bucket: Shard) -> bool {
        match self {
            TestNetworkDestination::All => true,
            TestNetworkDestination::Address(a) => a == addr,
            TestNetworkDestination::Bucket(b) => *b == bucket,
        }
    }

    pub fn is_bucket(&self) -> bool {
        matches!(self, TestNetworkDestination::Bucket(_))
    }
}

pub struct TestNetworkWorker {
    rx_new_transaction: Option<mpsc::Receiver<(TestNetworkDestination, ExecutedTransaction)>>,
    #[allow(clippy::type_complexity)]
    tx_new_transactions: HashMap<
        TestAddress,
        (
            Shard,
            mpsc::Sender<(TransactionId, usize)>,
            SqliteStateStore<TestAddress>,
        ),
    >,
    tx_hs_message: HashMap<TestAddress, mpsc::Sender<(TestAddress, HotstuffMessage)>>,
    #[allow(clippy::type_complexity)]
    rx_broadcast: Option<HashMap<TestAddress, mpsc::Receiver<(Vec<TestAddress>, HotstuffMessage)>>>,
    #[allow(clippy::type_complexity)]
    rx_leader: Option<HashMap<TestAddress, mpsc::Receiver<(TestAddress, HotstuffMessage)>>>,
    rx_mempool: Option<HashMap<TestAddress, mpsc::UnboundedReceiver<Transaction>>>,
    network_status: watch::Receiver<NetworkStatus>,
    on_message: watch::Sender<Option<HotstuffMessage>>,
    num_sent_messages: Arc<AtomicUsize>,
    num_filtered_messages: Arc<AtomicUsize>,
    transaction_store: Arc<RwLock<HashMap<TransactionId, ExecutedTransaction>>>,

    offline_destinations: Arc<RwLock<Vec<TestNetworkDestination>>>,
    shutdown_signal: ShutdownSignal,
    message_filter: Option<MessageFilter>,
}

impl TestNetworkWorker {
    pub fn spawn(self) {
        tokio::spawn(self.run());
    }

    async fn run(mut self) {
        let mut rx_broadcast = self.rx_broadcast.take().unwrap();
        let mut rx_leader = self.rx_leader.take().unwrap();
        let mut rx_mempool = self.rx_mempool.take().unwrap();

        let mut rx_new_transaction = self.rx_new_transaction.take().unwrap();
        let tx_new_transactions = self.tx_new_transactions.clone();
        let transaction_store = self.transaction_store.clone();

        // Handle transactions that come in from the test. This behaves like a mempool.
        tokio::spawn(async move {
            while let Some((dest, executed)) = rx_new_transaction.recv().await {
                transaction_store
                    .write()
                    .await
                    .insert(*executed.transaction().id(), executed.clone());
                for (addr, (bucket, tx_new_transaction_to_consensus, state_store)) in &tx_new_transactions {
                    if dest.is_for(addr, *bucket) {
                        state_store
                            .with_write_tx(|tx| {
                                executed.upsert(tx)?;
                                let atom = executed.to_atom();
                                let pool = TransactionPool::<SqliteStateStore<TestAddress>>::new();
                                if !pool.exists(tx, &atom.id)? {
                                    pool.insert(tx, atom)?;
                                }
                                Ok::<_, anyhow::Error>(())
                            })
                            .unwrap();
                        log::info!("🐞 New transaction {}", executed.id());
                        tx_new_transaction_to_consensus.send((*executed.id(), 0)).await.unwrap();
                    }
                }
            }
        });

        if self.network_status.borrow().is_paused() {
            loop {
                self.network_status.changed().await.unwrap();
                if let NetworkStatus::Started = *self.network_status.borrow() {
                    break;
                }
            }
        }

        loop {
            let mut rx_broadcast = rx_broadcast
                .iter_mut()
                .map(|(from, rx)| rx.recv().map(|r| (from.clone(), r)))
                .collect::<FuturesUnordered<_>>();
            let mut rx_leader = rx_leader
                .iter_mut()
                .map(|(from, rx)| rx.recv().map(|r| (from.clone(), r)))
                .collect::<FuturesUnordered<_>>();

            let mut rx_mempool = rx_mempool
                .iter_mut()
                .map(|(from, rx)| rx.recv().map(|r| (from.clone(), r)))
                .collect::<FuturesUnordered<_>>();

            tokio::select! {
                biased;

                  _ = self.shutdown_signal.wait() => {
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
                Some((from, Some(msg))) = rx_mempool.next() => self.handle_mempool(from, msg).await,
            }
        }
    }

    pub async fn handle_broadcast(&mut self, from: TestAddress, to: Vec<TestAddress>, msg: HotstuffMessage) {
        log::debug!("🌎️ Broadcast {} from {} to {}", msg, from, to.iter().join(", "));
        for vn in to {
            if let Some(message_filter) = &self.message_filter {
                if !message_filter(&from, &vn, &msg) {
                    self.num_filtered_messages
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
            }
            // TODO: support for taking a whole committee bucket offline
            if vn != from && self.is_offline_destination(&vn, u32::MAX.into()).await {
                continue;
            }

            self.tx_hs_message
                .get(&vn)
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
        if from != to && self.is_offline_destination(&from, u32::MAX.into()).await {
            return;
        }
        self.on_message.send(Some(msg.clone())).unwrap();
        self.num_sent_messages
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.tx_hs_message.get(&to).unwrap().send((from, msg)).await.unwrap();
    }

    async fn is_offline_destination(&self, addr: &TestAddress, bucket: Shard) -> bool {
        let lock = self.offline_destinations.read().await;
        lock.iter().any(|d| d.is_for(addr, bucket))
    }

    /// Handles transactions that come in from missing transactions
    async fn handle_mempool(&mut self, from: TestAddress, msg: Transaction) {
        let (_, sender, state_store) = self
            .tx_new_transactions
            .get(&from)
            .unwrap_or_else(|| panic!("No new transaction channel for {}", from));

        // In the normal case, we need to provide the same execution results to consensus. In future we could add code
        // here to make a local decision to ABORT.
        let existing_executed_tx = self.transaction_store.read().await.get(msg.id()).unwrap().clone();
        state_store
            .with_write_tx(|tx| {
                existing_executed_tx.upsert(tx)?;
                let pool = TransactionPool::<SqliteStateStore<TestAddress>>::new();
                if !pool.exists(tx, existing_executed_tx.id())? {
                    pool.insert(tx, existing_executed_tx.to_atom())?;
                }
                Ok::<_, anyhow::Error>(())
            })
            .unwrap();

        sender.send((*existing_executed_tx.id(), 0)).await.unwrap();
    }
}
