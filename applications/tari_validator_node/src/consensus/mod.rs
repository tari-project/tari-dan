//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_common_types::types::PublicKey;
use tari_comms::{types::CommsPublicKey, NodeIdentity};
use tari_consensus::{
    hotstuff::{HotstuffEvent, HotstuffWorker},
    messages::HotstuffMessage,
};
use tari_dan_common_types::committee::Committee;
use tari_dan_p2p::{Message, OutboundService};
use tari_dan_storage::consensus_models::TransactionPool;
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::{
    consensus::{
        leader_selection::RoundRobinLeaderStrategy,
        signature_service::TariSignatureService,
        spec::TariConsensusSpec,
        state_manager::TariStateManager,
    },
    event_subscription::EventSubscription,
    p2p::services::{mempool::MempoolHandle, messaging::OutboundMessaging},
};

mod leader_selection;
mod signature_service;
mod spec;
mod state_manager;

pub async fn spawn(
    store: SqliteStateStore<PublicKey>,
    node_identity: Arc<NodeIdentity>,
    epoch_manager: EpochManagerHandle,
    rx_new_transactions: mpsc::Receiver<TransactionId>,
    rx_hs_message: mpsc::Receiver<(CommsPublicKey, HotstuffMessage<PublicKey>)>,
    outbound_messaging: OutboundMessaging,
    mempool: MempoolHandle,
    shutdown_signal: ShutdownSignal,
) -> (JoinHandle<Result<(), anyhow::Error>>, EventSubscription<HotstuffEvent>) {
    let (tx_broadcast, rx_broadcast) = mpsc::channel(10);
    let (tx_leader, rx_leader) = mpsc::channel(10);
    let (tx_mempool, rx_mempool) = mpsc::unbounded_channel();

    let validator_addr = node_identity.public_key().clone();
    let signing_service = TariSignatureService::new(node_identity);
    let leader_strategy = RoundRobinLeaderStrategy::new();
    let transaction_pool = TransactionPool::new();
    let state_manager = TariStateManager::new();
    let (tx_events, _) = broadcast::channel(100);

    let epoch_events = epoch_manager.subscribe().await.unwrap();

    let handle = HotstuffWorker::<TariConsensusSpec>::new(
        validator_addr,
        rx_new_transactions,
        rx_hs_message,
        store,
        epoch_events,
        epoch_manager,
        leader_strategy,
        signing_service,
        state_manager,
        transaction_pool,
        tx_broadcast,
        tx_leader,
        tx_events.clone(),
        tx_mempool,
        shutdown_signal,
    )
    .spawn();

    ConsensusWorker {
        rx_broadcast,
        rx_leader,
        rx_mempool,
        outbound_messaging,
        mempool,
    }
    .spawn();

    (handle, EventSubscription::new(tx_events))
}

struct ConsensusWorker {
    rx_broadcast: mpsc::Receiver<(Committee<CommsPublicKey>, HotstuffMessage<CommsPublicKey>)>,
    rx_leader: mpsc::Receiver<(CommsPublicKey, HotstuffMessage<CommsPublicKey>)>,
    rx_mempool: mpsc::UnboundedReceiver<Transaction>,
    outbound_messaging: OutboundMessaging,
    mempool: MempoolHandle,
}

impl ConsensusWorker {
    fn spawn(mut self) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some((committee, msg)) = self.rx_broadcast.recv() => {
                        self.outbound_messaging
                            .broadcast(committee.members(), Message::Consensus(msg))
                            .await
                            .ok();
                    },
                    Some((dest, msg)) = self.rx_leader.recv() => {
                        self.outbound_messaging
                            .send(dest, Message::Consensus(msg))
                            .await
                            .ok();
                    },
                    Some(tx) = self.rx_mempool.recv() => {
                        self.mempool.submit_transaction_without_propagating(tx).await.ok();
                    },
                    else => break,
                }
            }
        });
    }
}
