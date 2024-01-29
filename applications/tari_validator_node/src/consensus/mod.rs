//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_common::configuration::Network;
use tari_consensus::hotstuff::{ConsensusWorker, ConsensusWorkerContext, HotstuffWorker};
use tari_dan_storage::consensus_models::TransactionPool;
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
        signature_service::TariSignatureService,
        spec::TariConsensusSpec,
        state_manager::TariStateManager,
    },
    event_subscription::EventSubscription,
};

mod handle;
mod leader_selection;
mod signature_service;
mod spec;
mod state_manager;

pub use handle::*;
use sqlite_message_logger::SqliteMessageLogger;
use tari_dan_app_utilities::keypair::RistrettoKeypair;
use tari_dan_common_types::PeerAddress;
use tari_rpc_state_sync::RpcStateSyncManager;

use crate::p2p::services::messaging::{ConsensusInboundMessaging, ConsensusOutboundMessaging};

pub async fn spawn(
    network: Network,
    store: SqliteStateStore<PeerAddress>,
    keypair: RistrettoKeypair,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    rx_new_transactions: mpsc::Receiver<(TransactionId, usize)>,
    inbound_messaging: ConsensusInboundMessaging<SqliteMessageLogger>,
    outbound_messaging: ConsensusOutboundMessaging<SqliteMessageLogger>,
    client_factory: TariValidatorNodeRpcClientFactory,
    shutdown_signal: ShutdownSignal,
) -> (
    JoinHandle<Result<(), anyhow::Error>>,
    ConsensusHandle,
    mpsc::UnboundedReceiver<Transaction>,
) {
    let (tx_mempool, rx_mempool) = mpsc::unbounded_channel();

    let validator_addr = PeerAddress::from(keypair.public_key().clone());
    let signing_service = TariSignatureService::new(keypair);
    let leader_strategy = RoundRobinLeaderStrategy::new();
    let transaction_pool = TransactionPool::new();
    let state_manager = TariStateManager::new();
    let (tx_hotstuff_events, _) = broadcast::channel(100);

    let hotstuff_worker = HotstuffWorker::<TariConsensusSpec>::new(
        validator_addr,
        network,
        inbound_messaging,
        outbound_messaging,
        rx_new_transactions,
        store.clone(),
        epoch_manager.clone(),
        leader_strategy,
        signing_service,
        state_manager,
        transaction_pool,
        tx_hotstuff_events.clone(),
        tx_mempool,
        shutdown_signal.clone(),
    );

    let (tx_current_state, rx_current_state) = watch::channel(Default::default());
    let context = ConsensusWorkerContext {
        epoch_manager: epoch_manager.clone(),
        hotstuff: hotstuff_worker,
        state_sync: RpcStateSyncManager::new(network, epoch_manager, store, leader_strategy, client_factory),
        tx_current_state,
    };

    let handle = ConsensusWorker::new(shutdown_signal).spawn(context);

    (
        handle,
        ConsensusHandle::new(rx_current_state, EventSubscription::new(tx_hotstuff_events)),
        rx_mempool,
    )
}
