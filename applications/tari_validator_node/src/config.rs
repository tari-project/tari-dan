//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use config::Config;
use serde::{Deserialize, Serialize};
use tari_common::{
    ConfigurationError,
    DefaultConfigLoader,
    SubConfigPath,
    configuration::{CommonConfig, ConfigList},
};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_ootle_app_utilities::{
    epoch_oracle_config::EpochOracleConfig,
    genesis_governance::{GenesisCouncil, GenesisCouncilError},
    p2p_config::{P2pConfig, PeerSeedsConfig, RpcConfig},
};
use tari_ootle_common_types::diagnostics::DiagnosticLevel;
use tari_ootle_storage::DiagnosticRetention;
use tari_ootle_template_provider::TemplateConfig;
use tari_ootle_transaction::Network;
use tari_state_store_rocksdb::DatabaseOptions;
use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;

#[derive(Debug, Clone)]
pub struct ApplicationConfig {
    pub common: CommonConfig,
    pub validator_node: ValidatorNodeConfig,
    pub epoch_oracle: EpochOracleConfig,
    pub peer_seeds: PeerSeedsConfig,
    pub network: Network,
}

impl ApplicationConfig {
    pub fn load_from(cfg: &Config) -> Result<Self, ConfigurationError> {
        let mut config = Self {
            common: CommonConfig::load_from(cfg)?,
            validator_node: ValidatorNodeConfig::load_from(cfg)?,
            epoch_oracle: EpochOracleConfig::load_from(cfg)?,
            peer_seeds: PeerSeedsConfig::load_from(cfg)?,
            network: cfg.get("network")?,
        };
        config.validator_node.set_base_path(config.common.base_path());
        Ok(config)
    }

    pub fn get_layer_one_transaction_base_path(&self) -> PathBuf {
        if self.validator_node.layer_one_transaction_path.is_absolute() {
            return self.validator_node.layer_one_transaction_path.clone();
        }
        self.common
            .base_path()
            .join(&self.validator_node.layer_one_transaction_path)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct ValidatorNodeConfig {
    override_from: Option<String>,
    pub shard_key_file: PathBuf,
    /// A path to the file that stores your node identity and secret key
    pub identity_file: PathBuf,
    /// The relative path to store persistent data
    pub data_dir: PathBuf,
    /// An absolute or relative (to data_dir) path to the state database
    pub state_db_path: PathBuf,
    /// An absolute or relative (to data_dir) path to a file of consensus constant overrides, read
    /// once at startup and only on LocalNet. Setting it says the file is expected, so a node that
    /// cannot find it refuses to start. Leave it unset to pick up
    /// `<data_dir>/consensus_constants.toml` if it happens to be there.
    #[serde(default)]
    pub localnet_consensus_constants_file: Option<PathBuf>,
    /// Database config
    // pub database: tari_any_state_store::Config,
    /// The p2p configuration settings
    pub p2p: P2pConfig,
    /// P2P RPC configuration
    pub rpc: RpcConfig,
    /// JSON-RPC address of the validator node application. The Web UI is served from the same address.
    pub json_rpc_listener_address: Option<SocketAddr>,
    /// Listening address for the Prometheus metrics endpoint (`/_metrics`), served separately from the JSON-RPC.
    /// Only used when the `metrics` feature is enabled (default = "127.0.0.1:18201"); `None` disables the listener.
    pub metrics_listener_address: Option<SocketAddr>,
    /// Allow cross-origin browser requests to the JSON-RPC. Browsers on other origins, such as a Vite dev
    /// server or the swarm daemon UI, can then drive every JSON-RPC method, including signing layer-one
    /// transactions. Leave off unless the listener is only reachable by trusted pages.
    #[serde(default)]
    pub enable_permissive_cors: bool,
    /// Template config
    pub templates: TemplateConfig,
    /// Fee claim public key
    pub fee_claim_public_key: RistrettoPublicKey,
    /// The burn rate council this node writes into genesis. Genesis is part of what every node on a
    /// network agrees on, so every node must be configured with the same council, or none: with none
    /// the network's own council is seated, which is the only council MainNet accepts.
    #[serde(default)]
    pub genesis_council: GenesisCouncilConfig,
    /// Create identity file if not exists
    pub dont_create_id: bool,
    /// The (optional) sidechain to run this on. Identifies this chain for validator-node and
    /// template registration filtering, block-header validation, and L1 burn-claim binding.
    pub sidechain_id: Option<RistrettoPublicKey>,
    /// Retain the full transaction history instead of pruning finalized transaction records once
    /// they age out of the epoch retention window. Chain state (committed effects and transaction
    /// receipts) is retained regardless; this only controls the node-local transaction payloads,
    /// results and finalized markers used for queries and history.
    #[serde(default)]
    pub keep_transaction_history: bool,
    /// The path to store layer-one transactions.
    pub layer_one_transaction_path: PathBuf,
    /// Consensus configuration
    pub consensus: ConsensusConfig,
    /// Start even when the binary schedules a schema activation at an epoch this node has already
    /// passed. Doing so re-hashes committed state and diverges from the network, so this exists only
    /// for a deliberate re-bootstrap of a node whose state is being discarded anyway.
    #[serde(default)]
    pub allow_past_protocol_activation: bool,
    /// Maximum total size of inbound transaction gossip awaiting mempool validation. The mempool
    /// drains this queue serially, so it absorbs bursts that arrive faster than validation; once it
    /// is full, further messages are dropped rather than queued without limit. Sized in bytes
    /// because every message may be up to `gossip_sub_max_message_size`: at ordinary transaction
    /// sizes this admits a very deep backlog, while capping a flood of maximum-size messages.
    #[serde(default = "default_max_transaction_gossip_queue_bytes")]
    pub max_transaction_gossip_queue_bytes: usize,
    /// Maximum total size of inbound consensus gossip awaiting processing.
    ///
    /// The topic carries only `HotstuffMessage::ForeignProposalNotification` — a block id, an epoch
    /// and a shard group list. The proposal itself is requested over the messaging protocol, so
    /// legitimate traffic here is a few hundred bytes per message and nowhere near this budget.
    ///
    /// The budget is not sized for legitimate traffic. It is sized for what a flood can queue before
    /// the node starts dropping, and the topic is open to anyone: a message is only known to be
    /// undecodable or invalid after it has been drained. It is set above the transaction topic's
    /// because a dropped notification delays a foreign proposal the local committee is waiting on,
    /// whereas a dropped transaction can be re-gossiped.
    ///
    /// No measurement supports this particular figure. Now that the legitimate traffic on this topic
    /// is known to be small, it is the queue most likely to be over-provisioned.
    #[serde(default = "default_max_consensus_gossip_queue_bytes")]
    pub max_consensus_gossip_queue_bytes: usize,
    /// Maximum total size of inbound direct consensus messages awaiting processing. Carries
    /// intra-committee `HotStuffMessage`s. Reaching this queue requires an established connection
    /// rather than a topic publish, so it is harder to flood than the gossip topics, but it is
    /// equally liveness-critical.
    #[serde(default = "default_max_consensus_messaging_queue_bytes")]
    pub max_consensus_messaging_queue_bytes: usize,
    /// Total memory the state store may hold across its block cache and memtables, shared by every
    /// column family. Half is given to memtables and the rest stays available to cache reads.
    /// Larger trades memory for fewer disk reads and less frequent flushing; it is the largest
    /// single line in the node's memory budget, which is logged at startup.
    #[serde(default = "default_state_store_memory_budget_bytes")]
    pub state_store_memory_budget_bytes: usize,
    /// Local diagnostic event log
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
}

fn default_max_transaction_gossip_queue_bytes() -> usize {
    128 * 1024 * 1024
}

fn default_max_consensus_gossip_queue_bytes() -> usize {
    256 * 1024 * 1024
}

fn default_max_consensus_messaging_queue_bytes() -> usize {
    128 * 1024 * 1024
}

fn default_state_store_memory_budget_bytes() -> usize {
    tari_state_store_rocksdb::DEFAULT_MEMORY_BUDGET_BYTES
}

/// The validator's local record of abnormal events (leader failures, consensus errors, sync and
/// state machine transitions, crashes). Read and cleared over JSON-RPC and shown in the web UI.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Maximum number of events retained. The oldest are dropped first.
    #[serde(default = "default_diagnostics_max_events")]
    pub max_events: usize,
    /// Maximum age of a retained event, in seconds.
    #[serde(default = "default_diagnostics_max_age_secs")]
    pub max_age_secs: u64,
    /// Events below this level are never recorded.
    #[serde(default)]
    pub min_level: DiagnosticLevel,
}

impl DiagnosticsConfig {
    pub fn retention(&self) -> DiagnosticRetention {
        DiagnosticRetention {
            max_events: self.max_events,
            max_age: Duration::from_secs(self.max_age_secs),
        }
    }
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_events: default_diagnostics_max_events(),
            max_age_secs: default_diagnostics_max_age_secs(),
            min_level: DiagnosticLevel::Info,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_diagnostics_max_events() -> usize {
    10_000
}

fn default_diagnostics_max_age_secs() -> u64 {
    7 * 24 * 60 * 60
}

impl ValidatorNodeConfig {
    /// Database options for the state store this node opens.
    ///
    /// The memory budget is one capacity covering the block cache and memtables together, split so
    /// that memtables take half and the rest stays available to cache reads — RocksDB's own
    /// guidance, and the split the startup memory budget assumes.
    pub fn state_store_options(&self) -> DatabaseOptions {
        DatabaseOptions::default()
            // TODO: just enable it always for now, later make it configurable and default to true for testnets
            .with_debugging_data(true)
            .with_prune_transaction_history(!self.keep_transaction_history)
            .with_memory_budget_bytes(self.state_store_memory_budget_bytes)
            .with_memtable_budget_bytes(self.state_store_memory_budget_bytes / 2)
    }

    pub fn set_base_path<P: AsRef<Path>>(&mut self, base_path: P) {
        if !self.shard_key_file.is_absolute() {
            self.shard_key_file = base_path.as_ref().join(&self.shard_key_file);
        }
        if !self.identity_file.is_absolute() {
            self.identity_file = base_path.as_ref().join(&self.identity_file);
        }
        if !self.data_dir.is_absolute() {
            self.data_dir = base_path.as_ref().join(&self.data_dir);
        }
        if !self.state_db_path.is_absolute() {
            self.state_db_path = self.data_dir.join(&self.state_db_path);
        }
        if let Some(path) = self
            .localnet_consensus_constants_file
            .as_ref()
            .filter(|p| !p.is_absolute())
        {
            self.localnet_consensus_constants_file = Some(self.data_dir.join(path));
        }
        // if !self.database.rocks_db.path.is_absolute() {
        //     self.database.rocks_db.path = self.data_dir.as_ref().join(&self.database.rocks_db.path);
        // }
        // if !self.database.sqlite.path.is_absolute() {
        //     self.database.sqlite.path = self.data_dir.as_ref().join(&self.database.sqlite.path);
        // }
    }

    /// Where a consensus constants file is picked up from when none is configured.
    pub fn default_localnet_consensus_constants_file(&self) -> PathBuf {
        self.data_dir.join("consensus_constants.toml")
    }

    pub fn get_global_db_path(&self) -> PathBuf {
        self.data_dir.join("global_storage.sqlite")
    }
}

impl Default for ValidatorNodeConfig {
    fn default() -> Self {
        Self {
            override_from: None,
            shard_key_file: PathBuf::from("shard_key.json"),
            identity_file: PathBuf::from("validator_node_id.json"),
            data_dir: PathBuf::from("data/validator_node"),
            state_db_path: PathBuf::from("rocksdb"),
            localnet_consensus_constants_file: None,
            // database: tari_any_state_store::Config {
            //     database_type: AnyDatabaseType::Sqlite,
            //     rocks_db: RocksConfig { path: "rocksdb".into() },
            //     sqlite: SqliteConfig {
            //         path: "state.db".into(),
            //     },
            // },
            p2p: P2pConfig::default(),
            rpc: RpcConfig::default(),
            json_rpc_listener_address: Some("127.0.0.1:18200".parse().unwrap()),
            metrics_listener_address: Some("127.0.0.1:18201".parse().unwrap()),
            enable_permissive_cors: false,
            templates: TemplateConfig::default(),
            // Burn your fees
            fee_claim_public_key: RistrettoPublicKey::default(),
            genesis_council: GenesisCouncilConfig::default(),
            dont_create_id: false,
            sidechain_id: None,
            keep_transaction_history: false,
            layer_one_transaction_path: PathBuf::from("data/layer_one_transactions"),
            consensus: ConsensusConfig::default(),
            allow_past_protocol_activation: false,
            max_transaction_gossip_queue_bytes: default_max_transaction_gossip_queue_bytes(),
            max_consensus_gossip_queue_bytes: default_max_consensus_gossip_queue_bytes(),
            max_consensus_messaging_queue_bytes: default_max_consensus_messaging_queue_bytes(),
            state_store_memory_budget_bytes: default_state_store_memory_budget_bytes(),
            diagnostics: DiagnosticsConfig::default(),
        }
    }
}

/// A council named in configuration: `threshold` of `members` must sign to move the burn rate.
///
/// Left at its default, no council is configured and the network's own is seated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenesisCouncilConfig {
    #[serde(default)]
    pub threshold: u16,
    /// Hex-encoded public keys, as a list or comma-separated.
    #[serde(default)]
    pub members: ConfigList<RistrettoPublicKeyBytes>,
}

impl GenesisCouncilConfig {
    /// The configured council, or `None` when nothing was configured.
    pub fn to_council(&self) -> Result<Option<GenesisCouncil>, GenesisCouncilError> {
        if self.threshold == 0 && self.members.is_empty() {
            return Ok(None);
        }
        Ok(Some(GenesisCouncil {
            threshold: self.threshold,
            members: self.members.iter().copied().collect(),
        }))
    }
}

impl SubConfigPath for ValidatorNodeConfig {
    fn main_key_prefix() -> &'static str {
        "validator_node"
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ConsensusConfig {
    /// Skip the state sync check on startup and go directly into consensus. This makes `check_sync` always report
    /// up-to-date, so the node will never enter the syncing state. Intended for local development and recovery
    /// scenarios — running with this enabled against a network where the node is actually behind will cause
    /// consensus to misbehave.
    #[serde(default)]
    pub skip_sync: bool,
    /// The number of state sync runs a peer that served invalid state is left out of.
    #[serde(default = "default_state_sync_peer_skip_runs")]
    pub state_sync_peer_skip_runs: u32,
    /// The longest, in seconds, a peer's state sync stream may go without committing or proving a state version
    /// before the attempt is abandoned and the peer is skipped. 0 sets no limit.
    #[serde(default = "default_state_sync_progress_timeout_secs")]
    pub state_sync_progress_timeout_secs: u64,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            skip_sync: false,
            state_sync_peer_skip_runs: default_state_sync_peer_skip_runs(),
            state_sync_progress_timeout_secs: default_state_sync_progress_timeout_secs(),
        }
    }
}

fn default_state_sync_peer_skip_runs() -> u32 {
    tari_rpc_state_sync::DEFAULT_SYNC_PEER_SKIP_RUNS
}

fn default_state_sync_progress_timeout_secs() -> u64 {
    tari_rpc_state_sync::DEFAULT_SYNC_PROGRESS_TIMEOUT.as_secs()
}
