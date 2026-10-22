//  Copyright 2023. The Tari Project
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

use std::{net::SocketAddr, path::PathBuf, time::Duration};

use config::Config;
use ootle_byte_type::ToByteType;
use serde::{Deserialize, Serialize};
use tari_common::{
    ConfigurationError,
    DefaultConfigLoader,
    SubConfigPath,
    configuration::{CommonConfig, serializers},
};
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_indexer_lib::cached_substate_manager::DEFAULT_NEGATIVE_CACHE_TTL;
use tari_ootle_app_utilities::{
    epoch_oracle_config::EpochOracleConfig,
    p2p_config::{P2pConfig, PeerSeedsConfig},
};
use tari_ootle_template_provider::TemplateConfig;
use tari_ootle_transaction::Network;
use tari_template_lib_types::{TemplateAddress, crypto::RistrettoPublicKeyBytes};

use crate::{network_state_sync::EventFilter, rest_api::RefillRate};

#[derive(Debug, Clone)]
pub struct ApplicationConfig {
    pub common: CommonConfig,
    pub indexer: IndexerConfig,
    pub peer_seeds: PeerSeedsConfig,
    pub epoch_oracle: EpochOracleConfig,
    pub network: Network,
}

impl ApplicationConfig {
    pub fn load_from(cfg: &Config) -> Result<Self, ConfigurationError> {
        let config = Self {
            common: CommonConfig::load_from(cfg)?,
            indexer: IndexerConfig::load_from(cfg)?,
            peer_seeds: PeerSeedsConfig::load_from(cfg)?,
            epoch_oracle: EpochOracleConfig::load_from(cfg)?,
            network: cfg.get("network")?,
        };
        Ok(config)
    }

    pub fn to_identity_file_path(&self) -> PathBuf {
        if self.indexer.identity_file.is_absolute() {
            return self.indexer.identity_file.clone();
        }

        self.common.base_path.join(&self.indexer.identity_file)
    }

    pub fn to_data_dir(&self) -> PathBuf {
        if self.indexer.data_dir.is_absolute() {
            return self.indexer.data_dir.clone();
        }

        self.common.base_path.join(&self.indexer.data_dir)
    }

    pub fn state_db_path(&self) -> PathBuf {
        self.to_data_dir().join("state.db")
    }

    /// The configured consensus constants file, resolved against the data directory.
    pub fn localnet_consensus_constants_path(&self) -> Option<PathBuf> {
        let path = self.indexer.localnet_consensus_constants_file.as_ref()?;
        if path.is_absolute() {
            return Some(path.clone());
        }

        Some(self.to_data_dir().join(path))
    }

    /// Where a consensus constants file is picked up from when none is configured.
    pub fn default_localnet_consensus_constants_path(&self) -> PathBuf {
        self.to_data_dir().join("consensus_constants.toml")
    }

    pub fn global_db_path(&self) -> PathBuf {
        self.to_data_dir().join("global_storage.sqlite")
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct IndexerConfig {
    override_from: Option<String>,
    /// A path to the file that stores your node identity and secret key
    pub identity_file: PathBuf,
    /// The relative path to store persistent data
    pub data_dir: PathBuf,
    /// An absolute or relative (to data_dir) path to a file of consensus constant overrides, read
    /// once at startup and only on LocalNet. Setting it says the file is expected, so an indexer that
    /// cannot find it refuses to start. Leave it unset to pick up
    /// `<data_dir>/consensus_constants.toml` if it happens to be there.
    #[serde(default)]
    pub localnet_consensus_constants_file: Option<PathBuf>,
    /// The p2p configuration settings
    pub p2p: P2pConfig,
    /// Listening address for the indexer API server
    pub api_listen_address: Option<SocketAddr>,
    /// Listening address for the Prometheus metrics endpoint (`/_metrics`). This is served on a dedicated listener,
    /// separate from the API server, so that metrics can be bound to a private interface while the API is public.
    /// Only used when the `metrics` feature is enabled (default = "127.0.0.1:18302"); `None` disables the listener.
    pub metrics_listen_address: Option<SocketAddr>,
    /// The address of the Web UI
    pub web_ui_address: Option<SocketAddr>,
    /// The publicly-accessible URL that the UI uses to connect to the API.
    /// If this is None, then the api_listen_address will be used.
    pub web_ui_public_api_url: Option<String>,
    /// How long a shard group waits before reopening its state sync stream after the stream fails,
    /// or after a validator that does not follow its tip closes it. Also how often sync statistics
    /// are reported.
    #[serde(with = "serializers::seconds")]
    pub state_scanning_interval: Duration,
    /// The longest a validator holds a state sync stream open without a transition to send. The
    /// stream stays open past the validator's tip, streaming transitions as they commit, and is
    /// reopened once this passes at the cost of one completion marker per shard. While it is open
    /// and quiet the validator's keepalives are what show it is still there, so this only sets how
    /// often a quiet stream is reopened.
    #[serde(default = "default_state_sync_stream_deadline", with = "serializers::seconds")]
    pub state_sync_stream_deadline: Duration,
    /// How often a validator is asked to show it is still there while the state sync stream has
    /// nothing to send. Each keepalive re-confirms every shard the stream has caught up, which is
    /// what keeps the substate cache serving a quiet shard. Must be well under
    /// `state_sync_stream_deadline`. A validator serves no shorter an interval than its own minimum
    /// (5s by default), and the stream is given up after several missed keepalives.
    #[serde(default = "default_state_sync_keepalive_interval", with = "serializers::seconds")]
    pub state_sync_keepalive_interval: Duration,
    /// The sidechain to listen on. Also identifies this chain for L1 burn-claim binding.
    pub sidechain_id: Option<RistrettoPublicKey>,
    /// Cache TTL for substates fetched during dry run transaction processing.
    /// A shorter TTL reduces the chance of stale fee estimates.
    #[serde(with = "serializers::seconds")]
    pub dry_run_cache_ttl: Duration,
    /// How many dry runs may execute at once. A dry run is unpaid, and one can spend up to the
    /// per-transaction execution ceilings in CPU, so this bounds the share of the node that dry runs
    /// take, however many clients send them. A dry run that waits too long for a slot is refused
    /// with a 503. Defaults to half the available cores, at least 1.
    #[serde(default = "default_dry_run_max_concurrent_executions")]
    pub dry_run_max_concurrent_executions: usize,
    /// Start even when the binary's schema activation schedule disagrees with the one this node has
    /// already run under. Doing so re-hashes committed state and breaks the substate proofs this
    /// indexer serves, so this exists only for a node whose state is being discarded.
    #[serde(default)]
    pub allow_past_protocol_activation: bool,
    /// How long after a shard was last confirmed level with its committee the substate cache keeps
    /// serving entries for it. A cached substate is served on the argument that every commit which
    /// could supersede or destroy it has already reached this indexer through that shard's transition
    /// stream, which only holds while the stream is being kept up with.
    ///
    /// A shard that sees no transition is re-confirmed by each keepalive on its stream, every
    /// `state_sync_keepalive_interval`. A validator that goes quiet is given up on after several
    /// missed keepalives and the stream reopened from another, which takes a minute or two, so this
    /// must comfortably exceed that or the cache closes for every shard of a group whose validator
    /// went away. It is also the only bound on a validator that has stopped serving transitions:
    /// until it expires, values that the withheld transitions would have retracted are still
    /// served. Ordinary staleness is bounded by how promptly the validator streams its commits,
    /// not by this.
    #[serde(default = "default_substate_cache_max_serve_lag", with = "serializers::seconds")]
    pub substate_cache_max_serve_lag: Duration,
    /// Maximum substates held in the cache - one entry each, its head version. Beyond this the oldest
    /// are evicted, at the cost of one validator round trip each to fetch again.
    #[serde(default = "default_substate_cache_max_entries")]
    pub substate_cache_max_entries: usize,
    /// How long the cache serves a substate it has established does not exist. A creation retracts
    /// that through the transition stream, so ordinary staleness is bounded by the sync round and
    /// this covers only what that stream cannot correct.
    ///
    /// What it cannot correct is an answer that was already wrong when it was cached. The committee
    /// is chosen from the epoch manager's current epoch, independently of the watermark that gates
    /// the write, so a lagging view of a shard group split can put the question to a committee that
    /// honestly no longer holds the substate and agrees it does not exist. Uncached that misleads
    /// one caller; cached it is served to every caller until this expires.
    ///
    /// Held shorter than the other entries because it is the one a caller feels as an absence: a
    /// substate being waited on stays missing until this expires. Lowering it towards zero narrows
    /// that window to sub-second - an entry cached within the current second still answers - at
    /// `f + 1` committee round trips for every nonexistent lookup.
    #[serde(default = "default_substate_cache_negative_ttl", with = "serializers::seconds")]
    pub substate_cache_negative_ttl: Duration,
    /// How many epochs past its terminal epoch a stored transaction is retained before it is pruned.
    /// A transaction's terminal epoch is the epoch it committed in once its receipt has been
    /// indexed, and its `max_epoch` — the last epoch it could still be sequenced in — until then, so
    /// a transaction that is never sequenced ages out on the same schedule as one that commits.
    /// Write `"forever"` to retain transactions indefinitely; `0` keeps only those that can still
    /// commit or committed in the current epoch.
    ///
    /// Applies to every stored transaction, whether submitted here or observed on the gossip topic.
    /// Only the transaction body and its locally recorded rejection reason are pruned; transaction
    /// receipts synced from the network follow `transaction_receipt_retention_epochs`, which is kept
    /// longer, so a pruned transaction still resolves to its receipt-backed outcome. Set this well above the
    /// longest a client may take to poll for a result: once pruned, a transaction no longer appears in the
    /// recent-transactions listing or single transaction lookup, and a mempool rejection reason recorded for it is
    /// lost.
    ///
    /// Pruning bounds database growth but does not return disk to the filesystem: SQLite reuses the
    /// freed pages rather than shrinking the file.
    #[serde(default = "default_transaction_retention_epochs", with = "retention_epochs")]
    pub transaction_retention_epochs: Option<u64>,
    /// How many epochs past the epoch its transaction committed in a transaction receipt is retained
    /// before it is pruned. Write `"forever"` (the default) to retain receipts indefinitely.
    ///
    /// A pruned receipt no longer answers a result lookup or appears in the receipt listing, so set
    /// this well above the longest a client may take to fetch a result. A window not longer than
    /// `transaction_retention_epochs` is raised to one epoch longer, and receipts are retained
    /// indefinitely while transactions are; see [`IndexerConfig::effective_receipt_retention_epochs`].
    /// The network economic totals, including the receipt count, are accumulated as receipts are
    /// indexed and keep counting pruned ones.
    #[serde(default, with = "retention_epochs")]
    pub transaction_receipt_retention_epochs: Option<u64>,
    /// How many epochs past the epoch its transaction committed in an event is retained before it is
    /// pruned. Write `"forever"` (the default) to retain events indefinitely.
    ///
    /// A pruned event no longer answers event queries or the event stream's catch-up, so a client
    /// resuming from an older cursor misses it.
    #[serde(default, with = "retention_epochs")]
    pub event_retention_epochs: Option<u64>,
    /// Store transactions observed on the network-wide transaction gossip topic, not only those
    /// submitted directly to this indexer. When enabled the indexer joins the transaction mesh as a
    /// full participant: it validates what it receives and propagates it onward. Disabling it leaves
    /// the mesh entirely — no transaction is received, stored or forwarded.
    ///
    /// The gossip topic carries the whole network's transaction volume, so leaving this on while
    /// retaining forever grows the database without bound. Size retention against an adversary
    /// rather than against ordinary volume: validation deliberately omits the checks that depend on
    /// this node's view of runtime state, so correctly signed transactions that no validator will
    /// ever admit — an unknown template, a conflicting output — are still stored, and they are cheap
    /// to mint.
    #[serde(default = "default_index_gossiped_transactions")]
    pub index_gossiped_transactions: bool,
    /// Maximum total size of inbound transaction gossip awaiting storage. The gossip service drains
    /// this queue serially, so it absorbs bursts that arrive faster than validation and batched
    /// writes; once it is full, further messages are dropped rather than queued without limit. Sized
    /// in bytes because every message may be up to `gossip_sub_max_message_size`: at ordinary
    /// transaction sizes this admits a very deep backlog, while capping a flood of maximum-size
    /// messages.
    #[serde(default = "default_max_transaction_gossip_queue_bytes")]
    pub max_transaction_gossip_queue_bytes: usize,
    /// How long each pruner idles between passes once it has nothing left to prune. While a backlog
    /// remains it drains in back-to-back batches rather than waiting out this interval. Only used
    /// when a retention window is set.
    #[serde(
        default = "default_prune_interval",
        alias = "transaction_prune_interval",
        with = "serializers::seconds"
    )]
    pub prune_interval: Duration,
    /// The event filtering configuration
    pub event_filters: Vec<EventFilter>,
    /// Bounds on the compiled-template caches: the in-memory one, and the on-disk artifact cache
    /// shared by the template manager and dry runs.
    #[serde(default)]
    pub templates: TemplateConfig,
    /// Template addresses to watch for component creation/update events.
    /// Components created from these templates are tracked in a separate table for fast lookup.
    /// Defaults to the builtin liquidity pool template.
    #[serde(default = "default_watched_templates")]
    pub watched_templates: Vec<TemplateAddress>,
    /// When true (the default), substates fetched from validators to serve client reads must come
    /// with a proof that verifies against the shard group committee, or the read fails. Disabling
    /// trades verifiability for performance: values are served unverified, as fetched from a single
    /// (possibly byzantine or out-of-sync) validator.
    #[serde(default = "default_verify_substate_proofs")]
    pub verify_substate_proofs: bool,
    /// Rate-limiting configuration for the REST API endpoints
    pub rate_limits: IndexerRateLimitsConfig,
}

fn default_verify_substate_proofs() -> bool {
    true
}

fn default_dry_run_max_concurrent_executions() -> usize {
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
}

fn default_state_sync_stream_deadline() -> Duration {
    Duration::from_secs(600)
}

fn default_state_sync_keepalive_interval() -> Duration {
    Duration::from_secs(10)
}

fn default_substate_cache_max_serve_lag() -> Duration {
    Duration::from_secs(300)
}

fn default_substate_cache_max_entries() -> usize {
    100_000
}

fn default_substate_cache_negative_ttl() -> Duration {
    DEFAULT_NEGATIVE_CACHE_TTL
}

impl IndexerConfig {
    /// The receipt retention window in force: the configured one, raised to at least one epoch longer
    /// than `transaction_retention_epochs`. A stored transaction reports its outcome from its receipt,
    /// so the receipt must outlive it, and the two pruners run independently, so equal windows would
    /// race at each epoch boundary. `None` retains receipts indefinitely.
    pub fn effective_receipt_retention_epochs(&self) -> Option<u64> {
        let receipts = self.transaction_receipt_retention_epochs?;
        let transactions = self.transaction_retention_epochs?;
        Some(receipts.max(transactions.saturating_add(1)))
    }
}

/// The subset of an indexer's configuration that is published over its API, as it affects what
/// clients see. Built once at startup: the API must expose exactly these values and nothing else
/// from `IndexerConfig`, which also holds local paths and listen addresses.
#[derive(Debug, Clone)]
pub struct PublishedIndexerConfig {
    pub sidechain_id: Option<RistrettoPublicKeyBytes>,
    pub transaction_retention_epochs: Option<u64>,
    pub transaction_receipt_retention_epochs: Option<u64>,
    pub event_retention_epochs: Option<u64>,
    pub index_gossiped_transactions: bool,
    pub verify_substate_proofs: bool,
    pub substate_cache_max_serve_lag: Duration,
    pub indexes_all_events: bool,
}

impl From<&IndexerConfig> for PublishedIndexerConfig {
    fn from(config: &IndexerConfig) -> Self {
        Self {
            sidechain_id: config.sidechain_id.as_ref().map(|pk| pk.to_byte_type()),
            transaction_retention_epochs: config.transaction_retention_epochs,
            transaction_receipt_retention_epochs: config.effective_receipt_retention_epochs(),
            event_retention_epochs: config.event_retention_epochs,
            index_gossiped_transactions: config.index_gossiped_transactions,
            verify_substate_proofs: config.verify_substate_proofs,
            substate_cache_max_serve_lag: config.substate_cache_max_serve_lag,
            indexes_all_events: config.event_filters.is_empty() ||
                config.event_filters.iter().any(EventFilter::is_match_all),
        }
    }
}

fn default_transaction_retention_epochs() -> Option<u64> {
    Some(50)
}

mod retention_epochs {
    //! A retention window accepts a number of epochs or the string `"forever"`. An
    //! omitted key takes the default and TOML has no null literal, so without an explicit spelling
    //! for it, retaining indefinitely would be unreachable from a config file.

    use std::fmt;

    use serde::{
        Deserializer,
        Serializer,
        de::{self, Visitor},
    };

    const FOREVER: &str = "forever";

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where D: Deserializer<'de> {
        deserializer.deserialize_any(RetentionEpochsVisitor)
    }

    pub fn serialize<S>(value: &Option<u64>, s: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        match value {
            Some(epochs) => s.serialize_u64(*epochs),
            None => s.serialize_str(FOREVER),
        }
    }

    struct RetentionEpochsVisitor;

    impl<'de> Visitor<'de> for RetentionEpochsVisitor {
        type Value = Option<u64>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a number of epochs or the string \"{FOREVER}\"")
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            u64::try_from(v)
                .map(Some)
                .map_err(|_| E::custom(format!("a retention window cannot be negative (got {v})")))
        }

        // The config layer hands every value through as a string, so a numeric literal in the file
        // arrives here rather than at `visit_u64`.
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            if v.eq_ignore_ascii_case(FOREVER) {
                return Ok(None);
            }
            v.parse::<u64>().map(Some).map_err(|_| {
                E::custom(format!(
                    "expected a number of epochs or \"{FOREVER}\" for a retention window, got '{v}'"
                ))
            })
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(self)
        }
    }
}

fn default_index_gossiped_transactions() -> bool {
    true
}

fn default_max_transaction_gossip_queue_bytes() -> usize {
    128 * 1024 * 1024
}

fn default_prune_interval() -> Duration {
    Duration::from_secs(60 * 60)
}

fn default_watched_templates() -> Vec<TemplateAddress> {
    vec![tari_template_builtin::LIQUIDITY_POOL_TEMPLATE_ADDRESS]
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            override_from: None,
            identity_file: PathBuf::from("indexer_id.json"),
            data_dir: PathBuf::from("data/indexer"),
            localnet_consensus_constants_file: None,
            p2p: P2pConfig::default(),
            api_listen_address: Some("127.0.0.1:18300".parse().unwrap()),
            metrics_listen_address: Some("127.0.0.1:18302".parse().unwrap()),
            web_ui_address: Some("127.0.0.1:15000".parse().unwrap()),
            web_ui_public_api_url: None,
            state_scanning_interval: Duration::from_secs(60),
            state_sync_stream_deadline: default_state_sync_stream_deadline(),
            state_sync_keepalive_interval: default_state_sync_keepalive_interval(),
            sidechain_id: None,
            dry_run_cache_ttl: Duration::from_secs(10),
            dry_run_max_concurrent_executions: default_dry_run_max_concurrent_executions(),
            allow_past_protocol_activation: false,
            substate_cache_max_serve_lag: default_substate_cache_max_serve_lag(),
            substate_cache_max_entries: default_substate_cache_max_entries(),
            substate_cache_negative_ttl: default_substate_cache_negative_ttl(),
            transaction_retention_epochs: default_transaction_retention_epochs(),
            transaction_receipt_retention_epochs: None,
            event_retention_epochs: None,
            index_gossiped_transactions: default_index_gossiped_transactions(),
            max_transaction_gossip_queue_bytes: default_max_transaction_gossip_queue_bytes(),
            prune_interval: default_prune_interval(),
            event_filters: vec![],
            templates: TemplateConfig::default(),
            watched_templates: default_watched_templates(),
            verify_substate_proofs: default_verify_substate_proofs(),
            rate_limits: IndexerRateLimitsConfig::default(),
        }
    }
}

impl SubConfigPath for IndexerConfig {
    fn main_key_prefix() -> &'static str {
        "indexer"
    }
}

/// Rate-limiting configuration for the indexer REST API.
///
/// All `*_rate` values are **per IP address**. Each rate is a token-bucket
/// `(capacity, window)` pair: `capacity` is the burst size, and the bucket
/// refills at `capacity / window` tokens per second. A 10-second window keeps
/// the per-minute throughput intact while letting clients recover from a burst
/// in seconds rather than a full minute.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct IndexerRateLimitsConfig {
    /// On by default, since the API is public. Local networks such as the swarm turn it off.
    pub enabled: bool,
    /// POST /transactions – default 50 req / 10s burst (5/s sustained)
    pub transactions_submit_rate: RefillRate,
    /// POST /transactions/dry-run – default 30 req / 10s burst (3/s sustained). Below submit: a dry-run fetches
    /// every input and executes the transaction on this indexer.
    pub transactions_dry_run_submit_rate: RefillRate,
    /// /substates/* – default 300 req / 10s burst (30/s sustained)
    pub substates_rate: RefillRate,
    /// /utxos/* – default 300 req / 10s burst (30/s sustained)
    pub utxos_fetch_rate: RefillRate,
    /// GET /transactions/* read endpoints – default 200 req / 10s burst (20/s sustained)
    pub transactions_rate: RefillRate,
    /// GET /epoch-checkpoints/* – default 20 req / 10s burst (2/s sustained). Below the other reads: a
    /// listing page carries full checkpoints of tens of KB each.
    #[serde(default = "default_epoch_checkpoints_rate")]
    pub epoch_checkpoints_rate: RefillRate,
    /// GET /resources/*, /templates/* and /validators – default 200 req / 10s burst (20/s sustained)
    #[serde(default = "default_reads_rate")]
    pub reads_rate: RefillRate,
    /// Maximum concurrent SSE connections per IP (default: 10)
    pub sse_max_connections_per_ip: usize,
    /// Key rate limits on the last X-Forwarded-For entry, else X-Real-IP (default: false).
    /// Only enable when the indexer is behind a reverse proxy that appends to X-Forwarded-For.
    pub trust_proxy_headers: bool,
    /// Key rate limits on CF-Connecting-IP, ahead of X-Forwarded-For (default: false).
    /// Only enable when Cloudflare is in front of the indexer: any other proxy passes a
    /// client-supplied CF-Connecting-IP through, and the client picks its own limit.
    #[serde(default)]
    pub trust_cf_connecting_ip: bool,
}

impl Default for IndexerRateLimitsConfig {
    fn default() -> Self {
        let window = Duration::from_secs(10);
        Self {
            enabled: true,
            transactions_submit_rate: RefillRate::new(50.0, window).unwrap(),
            transactions_dry_run_submit_rate: RefillRate::new(30.0, window).unwrap(),
            substates_rate: RefillRate::new(300.0, window).unwrap(),
            utxos_fetch_rate: RefillRate::new(300.0, window).unwrap(),
            transactions_rate: RefillRate::new(200.0, window).unwrap(),
            epoch_checkpoints_rate: default_epoch_checkpoints_rate(),
            reads_rate: default_reads_rate(),
            sse_max_connections_per_ip: 10,
            trust_proxy_headers: false,
            trust_cf_connecting_ip: false,
        }
    }
}

fn default_epoch_checkpoints_rate() -> RefillRate {
    RefillRate::new(20.0, Duration::from_secs(10)).unwrap()
}

fn default_reads_rate() -> RefillRate {
    RefillRate::new(200.0, Duration::from_secs(10)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TOML has no null literal and an omitted key takes the default, so `"forever"` is the only
    /// way an operator can select unlimited retention. Losing it would strand anyone who relied on
    /// the old default with no way back to it.
    #[test]
    fn retention_epochs_accepts_forever_and_epoch_counts() {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            #[serde(default = "default_transaction_retention_epochs", with = "retention_epochs")]
            transaction_retention_epochs: Option<u64>,
        }

        let parse = |toml: &str| toml::from_str::<Wrapper>(toml).map(|w| w.transaction_retention_epochs);

        assert_eq!(parse("transaction_retention_epochs = 50").unwrap(), Some(50));
        assert_eq!(parse("transaction_retention_epochs = 0").unwrap(), Some(0));
        assert_eq!(parse(r#"transaction_retention_epochs = "forever""#).unwrap(), None);
        assert_eq!(parse(r#"transaction_retention_epochs = "FOREVER""#).unwrap(), None);
        // The config layer can hand a numeric literal through as a string.
        assert_eq!(parse(r#"transaction_retention_epochs = "50""#).unwrap(), Some(50));
        assert_eq!(parse("").unwrap(), default_transaction_retention_epochs());
        assert!(parse("transaction_retention_epochs = -1").is_err());
        assert!(parse(r#"transaction_retention_epochs = "never""#).is_err());
    }

    #[test]
    fn receipts_outlive_the_transactions_they_report_on() {
        let effective = |transactions, receipts| {
            IndexerConfig {
                transaction_retention_epochs: transactions,
                transaction_receipt_retention_epochs: receipts,
                ..Default::default()
            }
            .effective_receipt_retention_epochs()
        };

        assert_eq!(effective(Some(50), None), None);
        assert_eq!(effective(None, None), None);
        assert_eq!(effective(Some(50), Some(100)), Some(100));
        assert_eq!(effective(Some(50), Some(50)), Some(51));
        assert_eq!(effective(Some(50), Some(10)), Some(51));
        assert_eq!(effective(None, Some(100)), None);
        assert_eq!(effective(Some(u64::MAX), Some(1)), Some(u64::MAX));
    }

    #[test]
    fn no_event_filters_indexes_every_event() {
        let config = IndexerConfig::default();
        assert!(config.event_filters.is_empty());
        assert!(PublishedIndexerConfig::from(&config).indexes_all_events);
    }

    /// The shipped config template declares one `[[indexer.event_filters]]` section with no fields,
    /// which matches every event. That must not read as a filtered indexer.
    #[test]
    fn an_empty_filter_indexes_every_event() {
        let config = IndexerConfig {
            event_filters: vec![EventFilter::default()],
            ..Default::default()
        };
        assert!(PublishedIndexerConfig::from(&config).indexes_all_events);
    }

    #[test]
    fn a_filter_that_narrows_events_is_reported_as_filtered() {
        let config = IndexerConfig {
            event_filters: vec![EventFilter {
                topic: Some("std.vault.deposit".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(!PublishedIndexerConfig::from(&config).indexes_all_events);
    }

    /// A match-all filter alongside narrower ones still admits everything.
    #[test]
    fn a_match_all_filter_wins_over_narrower_ones() {
        let config = IndexerConfig {
            event_filters: vec![
                EventFilter {
                    topic: Some("std.vault.deposit".into()),
                    ..Default::default()
                },
                EventFilter::default(),
            ],
            ..Default::default()
        };
        assert!(PublishedIndexerConfig::from(&config).indexes_all_events);
    }
}
