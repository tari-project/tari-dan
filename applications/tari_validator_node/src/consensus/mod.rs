//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use tari_consensus::{
    hotstuff::{ConsensusWorker, ConsensusWorkerContext, HotstuffConfig, HotstuffWorker},
    traits::ConsensusSpec,
};
use tari_epoch_manager::service::EpochManagerHandle;
use tari_ootle_storage::consensus_models::TransactionPool;
use tari_ootle_transaction::Network;
use tari_rpc_state_sync::RpcStateSyncClientProtocol;
use tari_shutdown::ShutdownSignal;
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};

use crate::{
    consensus::{leader_selection::RoundRobinLeaderStrategy, spec::TariConsensusSpec},
    event_subscription::EventSubscription,
    p2p::services::messaging::{ConsensusInboundMessaging, ConsensusOutboundMessaging},
};

mod block_transaction_executor;
mod block_transaction_validator;
mod handle;
mod leader_selection;
#[cfg(feature = "metrics")]
pub mod metrics;
mod signer_service;
pub mod spec;

pub use block_transaction_executor::*;
pub use block_transaction_validator::*;
pub use handle::*;
pub use signer_service::*;
use tari_consensus::{consensus_constants::ConsensusConstants, hotstuff::HotstuffEvent};
use tari_ootle_p2p::PeerAddress;
use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

use crate::{
    config::ConsensusConfig,
    consensus::spec::{ValidatorNodeStateStore, ValidatorTransactionProcessor},
    p2p::NopLogger,
};

pub async fn spawn(
    network: Network,
    consensus_config: &ConsensusConfig,
    sidechain_id: Option<RistrettoPublicKeyBytes>,
    store: ValidatorNodeStateStore,
    local_addr: PeerAddress,
    signing_service: TariSignatureService,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    inbound_messaging: ConsensusInboundMessaging<NopLogger>,
    outbound_messaging: ConsensusOutboundMessaging<NopLogger>,
    client_factory: TariValidatorNodeRpcClientFactory,
    hooks: <TariConsensusSpec as ConsensusSpec>::Hooks,
    shutdown_signal: ShutdownSignal,
    transaction_executor: TariBlockTransactionExecutor<ValidatorTransactionProcessor>,
    transaction_validator: TariBlockTransactionValidator,
    tx_hotstuff_events: broadcast::Sender<HotstuffEvent>,
    consensus_constants: ConsensusConstants,
) -> (JoinHandle<Result<(), anyhow::Error>>, ConsensusHandle) {
    let (tx_new_transaction, rx_new_transactions) = mpsc::channel(10);

    let leader_strategy = RoundRobinLeaderStrategy::new();
    let transaction_pool = TransactionPool::new();

    let hs_config = HotstuffConfig {
        network,
        sidechain_id,
        consensus_constants: consensus_constants.clone(),
        // TODO: make these configurable (defaults should probably be longer than 1 hour)
        state_tree_cleanup_interval: Duration::from_secs(60 * 60),
        epoch_gc_interval: Duration::from_secs(60 * 60),
        // NOTE: This value should be greater than the epoch oracle's scanning interval to avoid leader failure by race
        // condition.
        epoch_end_grace_period: Duration::from_secs(10),
        catch_up_request_timeout: Duration::from_secs(20),
    };

    let hotstuff_worker = HotstuffWorker::<TariConsensusSpec>::new(
        hs_config,
        local_addr,
        inbound_messaging,
        outbound_messaging,
        rx_new_transactions,
        store.clone(),
        epoch_manager.clone(),
        leader_strategy,
        signing_service.clone(),
        transaction_pool,
        transaction_executor,
        transaction_validator,
        tx_hotstuff_events.clone(),
        hooks,
        shutdown_signal.clone(),
    );
    let current_view = hotstuff_worker.pacemaker().current_view().clone();

    let (tx_current_state, rx_current_state) = watch::channel(Default::default());
    let context = ConsensusWorkerContext {
        epoch_manager: epoch_manager.clone(),
        hotstuff: hotstuff_worker,
        state_sync: RpcStateSyncClientProtocol::new(network, epoch_manager, store, client_factory, signing_service)
            .with_skip_sync(consensus_config.skip_sync),
        tx_current_state,
    };

    let join_handle = ConsensusWorker::new(shutdown_signal).spawn(context);

    let consensus_handle = ConsensusHandle::new(
        rx_current_state,
        EventSubscription::new(tx_hotstuff_events.downgrade()),
        current_view,
        tx_new_transaction,
    );

    (join_handle, consensus_handle)
}
