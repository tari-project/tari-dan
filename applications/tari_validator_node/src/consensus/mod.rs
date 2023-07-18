//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_comms::{types::CommsPublicKey, NodeIdentity};
use tari_consensus::{
    hotstuff::{HotstuffEvent, HotstuffWorker},
    messages::HotstuffMessage,
};
use tari_dan_common_types::committee::Committee;
use tari_dan_p2p::{DanMessage, OutboundService};
use tari_dan_storage::consensus_models::{ExecutedTransaction, TransactionPool};
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::{
    consensus::{
        leader_selection::RandomDeterministicLeaderStrategy,
        signature_service::TariSignatureService,
        spec::TariConsensusSpec,
        state_manager::TariStateManager,
    },
    event_subscription::EventSubscription,
    p2p::services::messaging::OutboundMessaging,
};

mod leader_selection;
mod signature_service;
mod spec;
mod state_manager;

pub async fn spawn(
    store: SqliteStateStore,
    node_identity: Arc<NodeIdentity>,
    epoch_manager: EpochManagerHandle,
    rx_new_transactions: mpsc::Receiver<ExecutedTransaction>,
    rx_hs_message: mpsc::Receiver<(CommsPublicKey, HotstuffMessage)>,
    outbound_messaging: OutboundMessaging,
    shutdown_signal: ShutdownSignal,
) -> (JoinHandle<Result<(), anyhow::Error>>, EventSubscription<HotstuffEvent>) {
    let (tx_broadcast, rx_broadcast) = mpsc::channel(10);
    let (tx_leader, rx_leader) = mpsc::channel(10);

    let validator_addr = node_identity.public_key().clone();
    let signing_service = TariSignatureService::new(node_identity);
    let leader_strategy = RandomDeterministicLeaderStrategy::new();
    let transaction_pool = TransactionPool::new();
    let noop_state_manager = TariStateManager::new();
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
        noop_state_manager,
        transaction_pool,
        tx_broadcast,
        tx_leader,
        tx_events.clone(),
        shutdown_signal,
    )
    .spawn();

    ConsensusWorker {
        rx_broadcast,
        rx_leader,
        outbound_messaging,
    }
    .spawn();

    (handle, EventSubscription::new(tx_events))
}

struct ConsensusWorker {
    rx_broadcast: mpsc::Receiver<(Committee<CommsPublicKey>, HotstuffMessage)>,
    rx_leader: mpsc::Receiver<(CommsPublicKey, HotstuffMessage)>,
    outbound_messaging: OutboundMessaging,
}

impl ConsensusWorker {
    fn spawn(mut self) {
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some((committee, msg)) = self.rx_broadcast.recv() => {
                        self.outbound_messaging
                            .broadcast(committee.members(), DanMessage::HotStuffMessage(Box::new(msg)))
                            .await
                            .ok();
                    },
                    Some((dest, msg)) = self.rx_leader.recv() => {
                        self.outbound_messaging
                            .send(dest, DanMessage::HotStuffMessage(Box::new(msg)))
                            .await
                            .ok();
                    },
                    else => break,
                }
            }
        });
    }
}
