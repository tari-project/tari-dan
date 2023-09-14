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

use std::time::{Duration, Instant};

use base_node::BaseNodeProcess;
use cucumber::gherkin::Scenario;
use http_server::MockHttpServer;
use indexer::IndexerProcess;
use indexmap::IndexMap;
use miner::MinerProcess;
use tari_common_types::types::PublicKey;
use tari_crypto::ristretto::{RistrettoComSig, RistrettoSecretKey};
use tari_validator_node_cli::versioned_substate_address::VersionedSubstateAddress;
use template::RegisteredTemplate;
use validator_node::ValidatorNodeProcess;
use wallet::WalletProcess;
use wallet_daemon::DanWalletDaemonProcess;

pub mod base_node;
pub mod helpers;
pub mod http_server;
pub mod indexer;
pub mod logging;
pub mod miner;
pub mod template;
pub mod validator_node;
pub mod validator_node_cli;
pub mod wallet;
pub mod wallet_daemon;
pub mod wallet_daemon_cli;

#[derive(Debug, Default, cucumber::World)]
pub struct TariWorld {
    pub base_nodes: IndexMap<String, BaseNodeProcess>,
    pub wallets: IndexMap<String, WalletProcess>,
    pub validator_nodes: IndexMap<String, ValidatorNodeProcess>,
    pub indexers: IndexMap<String, IndexerProcess>,
    pub vn_seeds: IndexMap<String, ValidatorNodeProcess>,
    pub miners: IndexMap<String, MinerProcess>,
    pub templates: IndexMap<String, RegisteredTemplate>,
    pub outputs: IndexMap<String, IndexMap<String, VersionedSubstateAddress>>,
    pub http_server: Option<MockHttpServer>,
    pub template_mock_server_port: Option<u16>,
    pub current_scenario_name: Option<String>,
    pub commitments: IndexMap<String, Vec<u8>>,
    pub commitment_ownership_proofs: IndexMap<String, RistrettoComSig>,
    pub rangeproofs: IndexMap<String, Vec<u8>>,
    pub addresses: IndexMap<String, String>,
    pub num_databases_saved: usize,
    pub account_keys: IndexMap<String, (RistrettoSecretKey, PublicKey)>,
    /// Key name -> key index
    pub wallet_keys: IndexMap<String, u64>,
    pub claim_public_keys: IndexMap<String, PublicKey>,
    pub wallet_daemons: IndexMap<String, DanWalletDaemonProcess>,
    pub fees_enabled: bool,
}

impl TariWorld {
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

    pub fn get_wallet_daemon(&self, name: &str) -> &DanWalletDaemonProcess {
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

    pub fn get_base_node(&self, name: &str) -> &BaseNodeProcess {
        self.base_nodes
            .get(name)
            .unwrap_or_else(|| panic!("Base node {} not found", name))
    }

    pub fn get_account_component_address(&self, name: &str) -> Option<VersionedSubstateAddress> {
        let all_components = self
            .outputs
            .get(name)
            .unwrap_or_else(|| panic!("Account component address {} not found", name));
        all_components.get("components/Account").cloned()
    }

    pub fn after(&mut self, _scenario: &Scenario) {
        let _drop = self.http_server.take();

        for (name, mut p) in self.indexers.drain(..) {
            println!("Shutting down indexer {}", name);
            p.shutdown.trigger();
        }

        for (name, mut p) in self.validator_nodes.drain(..) {
            println!("Shutting down validator node {}", name);
            p.shutdown.trigger();
        }

        for (name, mut p) in self.wallets.drain(..) {
            println!("Shutting down wallet {}", name);
            p.shutdown.trigger();
        }
        for (name, mut p) in self.base_nodes.drain(..) {
            println!("Shutting down base node {}", name);
            // You have explicitly trigger the shutdown now because of the change to use Arc/Mutex in tari_shutdown
            p.shutdown.trigger();
        }
        for (name, mut p) in self.wallet_daemons.drain(..) {
            println!("Shutting down wallet daemon {}", name);
            // You have explicitly trigger the shutdown now because of the change to use Arc/Mutex in tari_shutdown
            p.shutdown.trigger();
        }
        self.outputs.clear();
        self.commitments.clear();
        self.commitment_ownership_proofs.clear();
        self.miners.clear();
        self.fees_enabled = false;
    }

    pub async fn wait_until_base_nodes_have_transaction_in_mempool(&self, min_tx_count: usize, timeout: Duration) {
        let timer = Instant::now();
        'outer: loop {
            for bn in self.base_nodes.values() {
                let mut client = bn.create_client();
                let tx_count = client.get_mempool_transaction_count().await.unwrap();

                if tx_count < min_tx_count {
                    // println!(
                    //     "Waiting for {} to have {} transaction(s) in mempool (currently has {})",
                    //     bn.name, min_tx_count, tx_count
                    // );
                    if timer.elapsed() > timeout {
                        println!(
                            "Timed out waiting for base node {} to have {} transactions in mempool",
                            bn.name, min_tx_count
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
}
