//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_comms_rpc_state_sync::CommsRpcStateSyncManager;
use tari_consensus::{
    hotstuff::{ConsensusWorker, ConsensusWorkerContext, HotstuffWorker},
    messages::HotstuffMessage,
};
use tari_dan_common_types::committee::Committee;
use tari_dan_p2p::{Message, OutboundService};
use tari_dan_storage::consensus_models::{ForeignReceiveCounters, TransactionPool};
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::{Transaction, TransactionId};
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};

use crate::{
    consensus::{
        leader_selection::RoundRobinLeaderStrategy,
        state_manager::TariStateManager,
    },
    event_subscription::EventSubscription,
};

mod handle;
mod leader_selection;
mod spec;
pub use spec::TariConsensusSpec;
mod state_manager;

pub use handle::*;
use sqlite_message_logger::SqliteMessageLogger;
use tari_dan_app_utilities::{keypair::RistrettoKeypair, signature_service::TariSignatureService};
use tari_dan_common_types::PeerAddress;

use crate::p2p::services::message_dispatcher::OutboundMessaging;

pub async fn spawn(
    store: SqliteStateStore<PeerAddress>,
    keypair: RistrettoKeypair,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    rx_new_transactions: mpsc::Receiver<TransactionId>,
    rx_hs_message: mpsc::Receiver<(PeerAddress, HotstuffMessage)>,
    outbound_messaging: OutboundMessaging<PeerAddress, SqliteMessageLogger>,
    client_factory: TariValidatorNodeRpcClientFactory,
    foreign_receive_counter: ForeignReceiveCounters,
    shutdown_signal: ShutdownSignal,
) -> (
    JoinHandle<Result<(), anyhow::Error>>,
    ConsensusHandle,
    mpsc::UnboundedReceiver<Transaction>,
) {
    let (tx_broadcast, rx_broadcast) = mpsc::channel(10);
    let (tx_leader, rx_leader) = mpsc::channel(10);
    let (tx_mempool, rx_mempool) = mpsc::unbounded_channel();

    let validator_addr = PeerAddress::from(keypair.public_key().clone());
    let signing_service = TariSignatureService::new(keypair);
    let leader_strategy = RoundRobinLeaderStrategy::new();
    let transaction_pool = TransactionPool::new();
    let state_manager = TariStateManager::new();
    let (tx_hotstuff_events, _) = broadcast::channel(100);

    let hotstuff_worker = HotstuffWorker::<TariConsensusSpec>::new(
        validator_addr,
        rx_new_transactions,
        rx_hs_message,
        store.clone(),
        epoch_manager.clone(),
        leader_strategy,
        signing_service,
        state_manager,
        transaction_pool,
        tx_broadcast,
        tx_leader,
        tx_hotstuff_events.clone(),
        tx_mempool,
        foreign_receive_counter,
        shutdown_signal.clone(),
    );

    let (tx_current_state, rx_current_state) = watch::channel(Default::default());
    let context = ConsensusWorkerContext {
        epoch_manager: epoch_manager.clone(),
        hotstuff: hotstuff_worker,
        state_sync: CommsRpcStateSyncManager::new(epoch_manager, store, client_factory),
        tx_current_state,
    };

    let handle = ConsensusWorker::new(shutdown_signal).spawn(context);

    ConsensusMessageWorker {
        rx_broadcast,
        rx_leader,
        outbound_messaging,
    }
    .spawn();

    (
        handle,
        ConsensusHandle::new(rx_current_state, EventSubscription::new(tx_hotstuff_events)),
        rx_mempool,
    )
}

struct ConsensusMessageWorker {
    rx_broadcast: mpsc::Receiver<(Committee<PeerAddress>, HotstuffMessage)>,
    rx_leader: mpsc::Receiver<(PeerAddress, HotstuffMessage)>,
    outbound_messaging: OutboundMessaging<PeerAddress, SqliteMessageLogger>,
}

impl ConsensusMessageWorker {
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

                    else => break,
                }
            }
        });
    }
}
