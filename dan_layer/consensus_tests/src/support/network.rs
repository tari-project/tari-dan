//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use tari_consensus::messages::HotstuffMessage;
use tari_dan_common_types::committee::Committee;
use tari_dan_storage::consensus_models::ExecutedTransaction;
use tokio::sync::{mpsc, watch};

use crate::support::{address::TestAddress, ValidatorChannels};

pub fn spawn_network(channels: Vec<ValidatorChannels>) -> TestNetwork {
    let tx_new_transactions = channels
        .iter()
        .map(|c| (c.address, c.tx_new_transactions.clone()))
        .collect();
    let tx_hs_message = channels.iter().map(|c| (c.address, c.tx_hs_message.clone())).collect();
    let (rx_broadcast, rx_leader) = channels
        .into_iter()
        .map(|c| ((c.address, c.rx_broadcast), (c.address, c.rx_leader)))
        .unzip();
    let (tx_new_transaction, rx_new_transaction) = mpsc::channel(100);
    let (tx_network_status, network_status) = watch::channel(NetworkStatus::Paused);
    let (tx_on_message, rx_on_message) = watch::channel(None);

    TestNetworkWorker {
        network_status,
        rx_new_transaction: Some(rx_new_transaction),
        tx_new_transactions,
        tx_hs_message,
        rx_broadcast: Some(rx_broadcast),
        rx_leader: Some(rx_leader),
        on_message: tx_on_message,
    }
    .spawn();

    TestNetwork {
        tx_new_transaction,
        network_status: tx_network_status,
        on_message: rx_on_message,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkStatus {
    Started,
    Paused,
}

impl NetworkStatus {
    pub fn is_started(self) -> bool {
        matches!(self, NetworkStatus::Started)
    }

    pub fn is_paused(self) -> bool {
        matches!(self, NetworkStatus::Paused)
    }
}

pub struct TestNetwork {
    tx_new_transaction: mpsc::Sender<ExecutedTransaction>,
    network_status: watch::Sender<NetworkStatus>,
    on_message: watch::Receiver<Option<HotstuffMessage>>,
}

impl TestNetwork {
    pub fn start(&self) {
        self.network_status.send(NetworkStatus::Started).unwrap();
    }

    pub async fn on_message(&mut self) -> Option<HotstuffMessage> {
        self.on_message.changed().await.unwrap();
        self.on_message.borrow().clone()
    }

    pub async fn pause(&self) {
        self.network_status.send(NetworkStatus::Paused).unwrap();
    }

    pub async fn send_transaction(&self, tx: ExecutedTransaction) {
        self.tx_new_transaction.send(tx).await.unwrap();
    }
}

pub struct TestNetworkWorker {
    rx_new_transaction: Option<mpsc::Receiver<ExecutedTransaction>>,
    tx_new_transactions: HashMap<TestAddress, mpsc::Sender<ExecutedTransaction>>,
    tx_hs_message: HashMap<TestAddress, mpsc::Sender<(TestAddress, HotstuffMessage)>>,
    #[allow(clippy::type_complexity)]
    rx_broadcast: Option<HashMap<TestAddress, mpsc::Receiver<(Committee<TestAddress>, HotstuffMessage)>>>,
    rx_leader: Option<HashMap<TestAddress, mpsc::Receiver<(TestAddress, HotstuffMessage)>>>,
    network_status: watch::Receiver<NetworkStatus>,
    on_message: watch::Sender<Option<HotstuffMessage>>,
}

impl TestNetworkWorker {
    pub fn spawn(self) {
        tokio::spawn(self.run());
    }

    async fn run(mut self) {
        let mut rx_broadcast = self.rx_broadcast.take().unwrap();
        let mut rx_leader = self.rx_leader.take().unwrap();

        let mut rx_new_transaction = self.rx_new_transaction.take().unwrap();
        let mut tx_new_transactions = self.tx_new_transactions.clone();

        tokio::spawn(async move {
            while let Some(tx) = rx_new_transaction.recv().await {
                for tx_new_transaction in tx_new_transactions.values_mut() {
                    tx_new_transaction.send(tx.clone()).await.unwrap();
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
                .map(|(from, rx)| rx.recv().map(|r| (*from, r.unwrap())))
                .collect::<FuturesUnordered<_>>();
            let mut rx_leader = rx_leader
                .iter_mut()
                .map(|(from, rx)| rx.recv().map(|r| (*from, r.unwrap())))
                .collect::<FuturesUnordered<_>>();

            tokio::select! {
                Some((from, (to, msg))) = rx_broadcast.next() => self.handle_broadcast(from, to, msg).await,
                Some((from, (to, msg))) = rx_leader.next() => self.handle_leader(from,to, msg).await,

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
                else => break,
            }
        }
    }

    pub async fn handle_broadcast(&mut self, from: TestAddress, to: Committee<TestAddress>, msg: HotstuffMessage) {
        for vn in to {
            self.tx_hs_message
                .get(&vn)
                .unwrap()
                .send((from, msg.clone()))
                .await
                .unwrap();
        }
        self.on_message.send(Some(msg.clone())).unwrap();
    }

    pub async fn handle_leader(&mut self, from: TestAddress, to: TestAddress, msg: HotstuffMessage) {
        self.on_message.send(Some(msg.clone())).unwrap();
        self.tx_hs_message.get(&to).unwrap().send((from, msg)).await.unwrap();
    }
}
