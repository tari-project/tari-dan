//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::traits::{ConsensusSpec, hooks::CompositeHook};
use tari_engine::state_store::memory::ReadOnlyMemoryStateStore;
use tari_epoch_manager::service::EpochManagerHandle;
use tari_ootle_app_utilities::transaction_executor::TariTransactionProcessor;
use tari_ootle_p2p::PeerAddress;
use tari_ootle_template_provider::MemoryCacheTemplateProvider;
use tari_rpc_state_sync::RpcStateSyncClientProtocol;

#[cfg(feature = "metrics")]
use crate::metrics::PrometheusConsensusMetrics;
use crate::{
    consensus::{
        TariBlockTransactionExecutor,
        TariBlockTransactionValidator,
        leader_selection::RoundRobinLeaderStrategy,
        signer_service::TariSignatureService,
        // template_metadata_hooks::TemplateMetadataHooks,
    },
    diagnostics::DiagnosticHooks,
    p2p::{
        NopLogger,
        services::messaging::{ConsensusInboundMessaging, ConsensusOutboundMessaging},
    },
    template_prewarm::TemplatePrewarmHooks,
};

pub type ValidatorNodeStateStore = tari_state_store_rocksdb::RocksDbStateStore<PeerAddress>;
pub type ValidatorTemplateProvider =
    MemoryCacheTemplateProvider<tari_engine::wasm::DiskCachedWasmTemplateProvider<ValidatorNodeStateStore>>;
pub type ValidatorTransactionProcessor = TariTransactionProcessor<ReadOnlyMemoryStateStore>;
#[derive(Clone)]
pub struct TariConsensusSpec;

impl ConsensusSpec for TariConsensusSpec {
    type Addr = PeerAddress;
    type EpochManager = EpochManagerHandle<Self::Addr>;
    // Template metadata is now persisted from execution artifacts during block commit/propose
    #[cfg(not(feature = "metrics"))]
    type Hooks =
        CompositeHook<TemplatePrewarmHooks, CompositeHook<tari_consensus::traits::hooks::NoopHooks, DiagnosticHooks>>;
    #[cfg(feature = "metrics")]
    type Hooks = CompositeHook<TemplatePrewarmHooks, CompositeHook<PrometheusConsensusMetrics, DiagnosticHooks>>;
    type InboundMessaging = ConsensusInboundMessaging<NopLogger>;
    type LeaderStrategy = RoundRobinLeaderStrategy;
    type OutboundMessaging = ConsensusOutboundMessaging<NopLogger>;
    type SignerService = TariSignatureService;
    type StateStore = ValidatorNodeStateStore;
    type SyncManager = RpcStateSyncClientProtocol<Self>;
    type TransactionExecutor = TariBlockTransactionExecutor<ValidatorTransactionProcessor, ValidatorTemplateProvider>;
    type TransactionValidator = TariBlockTransactionValidator;
}
