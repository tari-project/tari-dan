//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_common::configuration::Network;
use tari_consensus::{
    hotstuff::{ConsensusWorker, ConsensusWorkerContext, HotstuffConfig, HotstuffWorker},
    traits::ConsensusSpec,
};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_dan_app_utilities::{
    consensus_constants::ConsensusConstants,
    template_manager::implementation::TemplateManager,
    transaction_executor::TariDanTransactionProcessor,
};
use tari_dan_common_types::PeerAddress;
use tari_dan_storage::consensus_models::TransactionPool;
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_rpc_state_sync::RpcStateSyncManager;
use tari_shutdown::ShutdownSignal;
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::Transaction;
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;
use tokio::{
    sync::{broadcast, mpsc, watch},
    task::JoinHandle,
};

use crate::{
    consensus::{leader_selection::RoundRobinLeaderStrategy, spec::TariConsensusSpec},
    event_subscription::EventSubscription,
    p2p::services::messaging::{ConsensusInboundMessaging, ConsensusOutboundMessaging},
    transaction_validators::{
        ClaimFeeTransactionValidator,
        EpochRangeValidator,
        FeeTransactionValidator,
        HasInputs,
        TemplateExistsValidator,
        TransactionSignatureValidator,
        TransactionValidationError,
    },
    validator::{BoxedValidator, Validator},
    ValidatorNodeConfig,
};

mod block_transaction_executor;
mod gossip;
mod handle;
mod leader_selection;
#[cfg(feature = "metrics")]
pub mod metrics;
mod signature_service;
mod spec;

pub use block_transaction_executor::*;
pub use gossip::*;
pub use handle::*;
pub use signature_service::*;

use crate::{p2p::NopLogger, transaction_validators::WithContext};

pub type ConsensusTransactionValidator = BoxedValidator<ValidationContext, Transaction, TransactionValidationError>;

pub async fn spawn(
    network: Network,
    sidechain_id: Option<RistrettoPublicKey>,
    store: SqliteStateStore<PeerAddress>,
    local_addr: PeerAddress,
    signing_service: TariSignatureService,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    inbound_messaging: ConsensusInboundMessaging<NopLogger>,
    outbound_messaging: ConsensusOutboundMessaging<NopLogger>,
    client_factory: TariValidatorNodeRpcClientFactory,
    hooks: <TariConsensusSpec as ConsensusSpec>::Hooks,
    shutdown_signal: ShutdownSignal,
    transaction_executor: TariDanBlockTransactionExecutor<
        TariDanTransactionProcessor<TemplateManager<PeerAddress>>,
        ConsensusTransactionValidator,
    >,
    consensus_constants: ConsensusConstants,
) -> (JoinHandle<Result<(), anyhow::Error>>, ConsensusHandle) {
    let (tx_new_transaction, rx_new_transactions) = mpsc::channel(10);

    let leader_strategy = RoundRobinLeaderStrategy::new();
    let transaction_pool = TransactionPool::new();
    let (tx_hotstuff_events, _) = broadcast::channel(100);

    let hs_config = HotstuffConfig {
        network,
        sidechain_id,
        max_base_layer_blocks_behind: consensus_constants.max_base_layer_blocks_behind,
        max_base_layer_blocks_ahead: consensus_constants.max_base_layer_blocks_ahead,
        num_preshards: consensus_constants.num_preshards,
        pacemaker_max_base_time: consensus_constants.pacemaker_max_base_time,
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
        signing_service,
        transaction_pool,
        transaction_executor,
        tx_hotstuff_events.clone(),
        hooks,
        shutdown_signal.clone(),
    );
    let current_view = hotstuff_worker.pacemaker().current_view().clone();

    let (tx_current_state, rx_current_state) = watch::channel(Default::default());
    let context = ConsensusWorkerContext {
        epoch_manager: epoch_manager.clone(),
        hotstuff: hotstuff_worker,
        state_sync: RpcStateSyncManager::new(epoch_manager, store, client_factory),
        tx_current_state,
    };

    let join_handle = ConsensusWorker::new(shutdown_signal).spawn(context);

    let consensus_handle = ConsensusHandle::new(
        rx_current_state,
        EventSubscription::new(tx_hotstuff_events),
        current_view,
        tx_new_transaction,
    );

    (join_handle, consensus_handle)
}

pub fn create_transaction_validator(
    config: &ValidatorNodeConfig,
    template_manager: TemplateManager<PeerAddress>,
) -> ConsensusTransactionValidator {
    let mut validator = WithContext::<ValidationContext, _, _>::new()
        .map_context(
            |_| (),
            TransactionSignatureValidator.and_then(TemplateExistsValidator::new(template_manager)),
        )
        .map_context(
            |c| c.current_epoch,
            EpochRangeValidator::new().and_then(ClaimFeeTransactionValidator::new()),
        )
        .boxed();
    if !config.no_fees {
        // A transaction without fee payment may have 0 inputs.
        validator = WithContext::<ValidationContext, _, _>::new()
            .map_context(|_| (), HasInputs::new())
            .and_then(validator)
            .map_context(|_| (), FeeTransactionValidator)
            .boxed();
    }
    validator
}
