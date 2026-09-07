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
};

use config::Config;
use serde::{Deserialize, Serialize};
use tari_common::{ConfigurationError, DefaultConfigLoader, SubConfigPath, configuration::CommonConfig};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_ootle_app_utilities::{
    epoch_oracle_config::EpochOracleConfig,
    p2p_config::{P2pConfig, PeerSeedsConfig, RpcConfig},
};
use tari_ootle_template_provider::TemplateConfig;
use tari_ootle_transaction::Network;

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
    /// JSON-RPC address of the validator node  application
    pub json_rpc_listener_address: Option<SocketAddr>,
    /// The public JSON-RPC url that the Web UI uses, if specified.
    pub json_rpc_public_url: Option<String>,
    /// The address to listen on for the Web UI
    pub web_ui_listener_address: Option<SocketAddr>,
    /// Template config
    pub templates: TemplateConfig,
    /// Fee claim public key
    pub fee_claim_public_key: RistrettoPublicKey,
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
    /// Maximum total size of inbound consensus gossip awaiting processing. This topic carries
    /// `HotStuffMessage`s between shard groups, including block-sized foreign proposals, and it
    /// feeds a short blocking channel into consensus — so this queue absorbs real bursts rather
    /// than sitting idle. Budgeted above transactions because a dropped proposal or vote can cost a
    /// view, whereas a dropped transaction can be re-requested.
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

impl ValidatorNodeConfig {
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
            json_rpc_public_url: None,
            web_ui_listener_address: Some("127.0.0.1:5001".parse().unwrap()),
            templates: TemplateConfig::default(),
            // Burn your fees
            fee_claim_public_key: RistrettoPublicKey::default(),
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
        }
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
    /// Enable proposing evictions for inactive validators. If disabled, this validator will still vote on eviction
    /// proposals from other validators, including voting in the affirmative if applicable, but will never propose
    /// evictions itself.
    pub enable_eviction_proposal: bool,
    /// Skip the state sync check on startup and go directly into consensus. This makes `check_sync` always report
    /// up-to-date, so the node will never enter the syncing state. Intended for local development and recovery
    /// scenarios — running with this enabled against a network where the node is actually behind will cause
    /// consensus to misbehave.
    #[serde(default)]
    pub skip_sync: bool,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            enable_eviction_proposal: true,
            skip_sync: false,
        }
    }
}
