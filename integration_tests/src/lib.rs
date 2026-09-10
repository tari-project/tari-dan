//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use base_node::BaseNodeProcess;
use cucumber::gherkin::Scenario;
use http_server::MockHttpServer;
use indexer::IndexerProcess;
use indexmap::IndexMap;
use miner::MinerProcess;
use tari_common::configuration::Network as L1Network;
use tari_common_types::{
    tari_address::{TariAddress, TariAddressFeatures},
    types::{CompressedPublicKey, PrivateKey},
};
use tari_consensus::consensus_constants::ConsensusConstants;
use tari_crypto::keys::SecretKey;
use tari_engine_types::substate::SubstateId;
use tari_ootle_app_utilities::consensus_constants_file::{ConsensusConstantsFile, load_consensus_constants};
use tari_ootle_common_types::SubstateRequirement;
use tari_ootle_transaction::Network;
use tari_ootle_wallet_sdk::models::AccountWithAddress;
use tari_template_lib_types::Amount;
use tari_transaction_components::{
    consensus::ConsensusManager,
    key_manager::{KeyManager, TariKeyId, TransactionKeyManagerInterface},
};
use template::RegisteredTemplate;
use validator_node::ValidatorNodeProcess;
use wallet::WalletProcess;
use wallet_daemon::TariWalletDaemonProcess;

use crate::{
    claim_proof::CucumberClaimProof,
    logging::{get_base_dir, get_base_dir_for_scenario},
};

pub mod base_node;
pub mod claim_proof;
pub mod helpers;
pub mod http_server;
pub mod indexer;
pub mod logging;
pub mod miner;
pub mod template;
pub mod util;
pub mod validator_node;
pub mod validator_node_client;
pub mod wallet;
pub mod wallet_daemon;
pub mod wallet_daemon_client;

#[derive(cucumber::World)]
#[world(init = Self::init)]
pub struct TariWorld {
    pub consensus_constants: ConsensusConstants,
    pub base_nodes: IndexMap<String, BaseNodeProcess>,
    pub wallets: IndexMap<String, WalletProcess>,
    pub validator_nodes: IndexMap<String, ValidatorNodeProcess>,
    pub indexers: IndexMap<String, IndexerProcess>,
    pub vn_seeds: IndexMap<String, ValidatorNodeProcess>,
    pub miners: IndexMap<String, MinerProcess>,
    pub templates: IndexMap<String, RegisteredTemplate>,
    pub outputs: IndexMap<String, IndexMap<String, SubstateRequirement>>,
    pub http_server: Option<MockHttpServer>,
    pub template_mock_server_port: Option<u16>,
    pub current_scenario_name: Option<String>,
    /// The consensus constants file every node in this scenario is pointed at. Consensus constants
    /// have to agree across a network, so a scenario authors one file and hands it to each node it
    /// spawns rather than configuring them one by one.
    pub consensus_constants_file: Option<PathBuf>,
    pub claim_proofs: HashMap<String, CucumberClaimProof>,
    /// The TARI balance each account had when a scenario last checked it, so a later step can
    /// assert how much a balance moved rather than where it landed.
    pub last_checked_balances: HashMap<String, Amount>,
    pub substate_ids: IndexMap<String, SubstateId>,
    pub num_databases_saved: usize,
    pub key_manager: KeyManager,
    pub wallet_accounts: IndexMap<String, AccountWithAddress>,
    pub wallet_daemons: IndexMap<String, TariWalletDaemonProcess>,
    /// Used for all one-sided coinbase payments
    pub minotari_wallet_private_key: PrivateKey,
    /// A receiver wallet address that is used for default one-sided coinbase payments
    pub default_payment_address: TariAddress,
    pub consensus_manager: ConsensusManager,
}

impl TariWorld {
    async fn init() -> Self {
        let wallet_private_key = PrivateKey::random(&mut rand::rng());
        let default_payment_address = TariAddress::new_single_address(
            CompressedPublicKey::from_secret_key(&wallet_private_key),
            L1Network::LocalNet,
            TariAddressFeatures::create_interactive_and_one_sided(),
        )
        .unwrap();
        Self {
            consensus_constants: ConsensusConstants::from(Network::LocalNet),
            base_nodes: IndexMap::new(),
            wallets: IndexMap::new(),
            validator_nodes: IndexMap::new(),
            indexers: IndexMap::new(),
            vn_seeds: IndexMap::new(),
            miners: IndexMap::new(),
            templates: IndexMap::new(),
            outputs: IndexMap::new(),
            http_server: None,
            template_mock_server_port: None,
            current_scenario_name: None,
            consensus_constants_file: None,
            claim_proofs: HashMap::new(),
            last_checked_balances: HashMap::new(),
            substate_ids: IndexMap::new(),
            num_databases_saved: 0,
            key_manager: KeyManager::new_random().unwrap(),
            wallet_accounts: IndexMap::new(),
            wallet_daemons: IndexMap::new(),
            minotari_wallet_private_key: wallet_private_key,
            default_payment_address,
            consensus_manager: ConsensusManager::builder(L1Network::LocalNet).build(),
        }
    }

    pub fn mark_point_in_logs(&self, point_name: &str) {
        fn write_point(file_name: &str, point_name: &str) {
            let base_dir = get_base_dir();
            if !base_dir.exists() {
                fs::create_dir_all(&base_dir).unwrap();
            }
            if base_dir.join(file_name).exists() {
                let log_file = fs::read_to_string(base_dir.join(file_name)).unwrap();
                fs::write(
                    base_dir.join(file_name),
                    format!(
                        "{}\n\n------------------------------------------------\n\n{}\n\\
                         n----------------------------------------------------------\n\n",
                        log_file, point_name
                    ),
                )
                .unwrap();
            }
        }

        write_point("base_layer.log", point_name);
        write_point("ootle.log", point_name);
        write_point("wallet.log", point_name);
        write_point("network.log", point_name);
        write_point("wallet_daemon.log", point_name);
    }

    /// Writes `overrides` to a file shared by every node this scenario spawns, and adopts the
    /// resulting constants as the world's own so that assertions and nodes cannot disagree.
    pub fn set_consensus_constants(&mut self, overrides: &ConsensusConstantsFile) {
        let dir = get_base_dir_for_scenario("network", self.get_current_scenario_name(), "consensus_constants");
        fs::create_dir_all(&dir).expect("Failed to create consensus constants directory");
        let path = dir.join("consensus_constants.toml");
        fs::write(
            &path,
            toml::to_string(overrides).expect("Failed to serialize consensus constants"),
        )
        .expect("Failed to write consensus constants file");

        self.consensus_constants = load_consensus_constants(Network::LocalNet, Some(&path), &path)
            .expect("Failed to load consensus constants");
        self.consensus_constants_file = Some(path);
    }

    pub fn get_current_scenario_name(&self) -> &str {
        self.current_scenario_name.as_deref().expect("No current scenario")
    }

    pub fn get_mock_server(&self) -> &MockHttpServer {
        self.http_server.as_ref().unwrap()
    }

    pub fn get_miner(&self, name: &str) -> &MinerProcess {
        self.miners
            .get(name)
            .unwrap_or_else(|| panic!("Miner {} not found", name))
    }

    pub fn get_wallet(&self, name: &str) -> &WalletProcess {
        self.wallets
            .get(name)
            .unwrap_or_else(|| panic!("Wallet {} not found", name))
    }

    pub fn get_wallet_daemon(&self, name: &str) -> &TariWalletDaemonProcess {
        self.wallet_daemons
            .get(name)
            .unwrap_or_else(|| panic!("Wallet daemon {} not found", name))
    }

    pub fn get_validator_node(&self, name: &str) -> &ValidatorNodeProcess {
        self.validator_nodes
            .get(name)
            .or_else(|| self.vn_seeds.get(name))
            .unwrap_or_else(|| panic!("Validator node {} not found", name))
    }

    pub fn get_validator_node_mut(&mut self, name: &str) -> &mut ValidatorNodeProcess {
        self.validator_nodes
            .get_mut(name)
            .or_else(|| self.vn_seeds.get_mut(name))
            .unwrap_or_else(|| panic!("Validator node {} not found", name))
    }

    pub fn all_running_validators_iter(&self) -> impl Iterator<Item = &ValidatorNodeProcess> + Clone {
        self.validator_nodes
            .values()
            .chain(self.vn_seeds.values())
            .filter(|vn| !vn.handle.is_finished())
    }

    pub fn get_indexer(&self, name: &str) -> &IndexerProcess {
        self.indexers
            .get(name)
            .unwrap_or_else(|| panic!("Indexer {} not found", name))
    }

    pub fn get_output<T: AsRef<str>>(&self, output_name: &str, discriminator: T) -> &SubstateRequirement {
        let outputs = self.outputs.get(output_name).unwrap_or_else(|| {
            self.print();
            panic!("Output {} not found", output_name)
        });
        outputs.get(discriminator.as_ref()).unwrap_or_else(|| {
            self.print();
            panic!(
                "Output {output_name} with discriminator {} not found",
                discriminator.as_ref()
            )
        })
    }

    pub fn get_output_fq<T: AsRef<str>>(&self, fq_output_name: T) -> &SubstateRequirement {
        let (output_name, discriminator) = fq_output_name.as_ref().trim().split_once("/").unwrap_or_else(|| {
            panic!(
                "Fully qualified output name must be in the format <output_name>/<discriminator>, got {}",
                fq_output_name.as_ref()
            )
        });
        self.get_output(output_name, discriminator)
    }

    pub fn get_base_node(&self, name: &str) -> &BaseNodeProcess {
        self.base_nodes
            .get(name)
            .unwrap_or_else(|| panic!("Base node {} not found", name))
    }

    pub fn after(&mut self, _scenario: &Scenario) {
        let _drop = self.http_server.take();

        for (name, mut p) in self.indexers.drain(..) {
            cucumber_log!("Shutting down indexer {}", name);
            p.shutdown.trigger();
        }

        for (name, mut p) in self.validator_nodes.drain(..) {
            cucumber_log!("Shutting down validator node {}", name);
            p.shutdown.trigger();
        }

        for (name, mut p) in self.vn_seeds.drain(..) {
            cucumber_log!("Shutting down validator node seed {}", name);
            p.shutdown.trigger();
        }

        for (name, mut p) in self.wallets.drain(..) {
            cucumber_log!("Shutting down wallet {}", name);
            p.shutdown.trigger();
        }
        for (name, mut p) in self.base_nodes.drain(..) {
            cucumber_log!("Shutting down base node {}", name);
            // You have explicitly trigger the shutdown now because of the change to use Arc/Mutex in tari_shutdown
            p.shutdown.trigger();
        }
        for (name, mut p) in self.wallet_daemons.drain(..) {
            cucumber_log!("Shutting down wallet daemon {}", name);
            // You have explicitly trigger the shutdown now because of the change to use Arc/Mutex in tari_shutdown
            p.shutdown.trigger();
        }
        self.outputs.clear();
        self.claim_proofs.clear();
        self.miners.clear();
    }

    pub async fn wait_until_base_nodes_have_transaction_in_mempool(&self, min_tx_count: usize, timeout: Duration) {
        let timer = Instant::now();
        'outer: loop {
            for bn in self.base_nodes.values() {
                let mut client = bn.create_client();
                let tx_count = client.get_mempool_transaction_count().await.unwrap();

                if tx_count < min_tx_count {
                    // cucumber_log!(
                    //     "Waiting for {} to have {} transaction(s) in mempool (currently has {})",
                    //     bn.name, min_tx_count, tx_count
                    // );
                    if timer.elapsed() > timeout {
                        cucumber_log!(
                            "Timed out waiting for base node {} to have {} transactions in mempool",
                            bn.name,
                            min_tx_count
                        );
                        panic!(
                            "Timed out waiting for base node {} to have {} transactions in mempool",
                            bn.name, min_tx_count
                        );
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue 'outer;
                }
            }

            break;
        }
    }

    pub fn script_key_id(&self) -> TariKeyId {
        self.key_manager
            .create_encrypted_key(self.minotari_wallet_private_key.clone(), None)
            .unwrap()
    }

    pub fn print(&self) {
        cucumber_log!("======================================");
        cucumber_log!("=========== CUCUMBER WORLD ===========");
        cucumber_log!("======================================");

        // base nodes
        for (name, node) in &self.base_nodes {
            cucumber_log!(
                "Base node \"{}\": grpc port \"{}\", temp dir path \"{}\"",
                name,
                node.grpc_port,
                node.temp_dir_path.display()
            );
        }

        // wallets
        for (name, node) in &self.wallets {
            cucumber_log!(
                "Wallet \"{}\": grpc port \"{}\", temp dir path \"{}\"",
                name,
                node.grpc_port,
                node.temp_dir_path.display()
            );
        }

        // vns
        for (name, node) in &self.validator_nodes {
            cucumber_log!(
                "Validator node \"{}\": json rpc port \"{}\", web ui port \"{}\", temp dir path \"{:?}\"",
                name,
                node.json_rpc_port,
                node.web_ui_port,
                node.temp_dir_path
            );
        }

        // indexes
        for (name, node) in &self.indexers {
            cucumber_log!(
                "Indexer \"{}\": json rpc port \"{}\", graphql port \"{}\", web ui port  \"{}\", temp dir path \"{}\"",
                name,
                node.api_port,
                node.graphql_port,
                node.web_ui_port,
                node.temp_dir_path
            );
        }

        // templates
        for (name, template) in &self.templates {
            cucumber_log!("Template \"{}\" with address \"{}\"", name, template.address);
        }

        // templates
        for (name, outputs) in &self.outputs {
            cucumber_log!("Outputs \"{}\"", name);
            for (name, addr) in outputs {
                cucumber_log!("  - {}: {}", name, addr);
            }
        }

        // wallet daemons
        for (name, daemon) in &self.wallet_daemons {
            cucumber_log!(
                "Wallet daemon \"{}\": json rpc port \"{}\", indexer api port \"{}\", temp dir path \"{:?}\"",
                name,
                daemon.json_rpc_port,
                daemon.indexer_api_port,
                daemon.temp_path_dir
            );
        }

        for (account_name, account) in &self.wallet_accounts {
            cucumber_log!(
                "Wallet account \"{}\" with address \"{}\"",
                account_name,
                account.address
            );
        }

        cucumber_log!("======================================");
    }
}

impl Debug for TariWorld {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TariWorld")
            .field("base_nodes", &self.base_nodes.keys())
            .field("wallets", &self.wallets.keys())
            .field("validator_nodes", &self.validator_nodes.keys())
            .field("indexers", &self.indexers.keys())
            .field("vn_seeds", &self.vn_seeds.keys())
            .field("miners", &self.miners.keys())
            .field("templates", &self.templates.keys())
            .field("outputs", &self.outputs.keys())
            .field("http_server", &self.http_server)
            .field("template_mock_server_port", &self.template_mock_server_port)
            .field("current_scenario_name", &self.current_scenario_name)
            .field("claim_proofs", &self.claim_proofs.keys())
            .field("addresses", &self.substate_ids.keys())
            .field("num_databases_saved", &self.num_databases_saved)
            .field("wallet_accounts", &self.wallet_accounts.keys())
            .field("wallet_daemons", &self.wallet_daemons.keys())
            .finish()
    }
}
