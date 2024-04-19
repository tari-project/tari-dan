//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_core::transactions::transaction_components::ValidatorNodeSignature;
use tari_validator_node_client::ValidatorNodeClient;
use tokio::{fs, time::sleep};
use url::Url;

use crate::process_manager::Instance;

pub struct ValidatorNodeProcess {
    instance: Instance,
}

impl ValidatorNodeProcess {
    pub fn new(instance: Instance) -> Self {
        Self { instance }
    }

    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }

    pub fn connect_client(&self) -> anyhow::Result<ValidatorNodeClient> {
        let client = ValidatorNodeClient::connect(self.json_rpc_address())?;
        Ok(client)
    }

    pub async fn is_json_rpc_listening(&self) -> bool {
        let mut client = self
            .connect_client()
            .expect("Validator node client is infallible unless using TLS backend");
        match client.get_identity().await {
            Ok(_) => true,
            Err(err) => {
                log::error!("Failed to connect to validator node: {}", err);
                false
            },
        }
    }

    pub async fn wait_for_startup(&self, timeout: Duration) -> anyhow::Result<()> {
        let mut attempts = 0usize;
        loop {
            if attempts * 1000 > timeout.as_millis() as usize {
                return Err(anyhow!(
                    "Validator node {} ({}) did not start up within {} seconds",
                    self.instance().id(),
                    self.instance().name(),
                    timeout.as_secs()
                ));
            }
            if self.is_json_rpc_listening().await {
                return Ok(());
            }

            log::info!(
                "Waiting for validator node {} ({}) to start up (attempt {})...",
                self.instance().id(),
                self.instance().name(),
                attempts
            );
            sleep(Duration::from_secs(1)).await; // wait for the validator node to start up before continuing
            attempts += 1;
        }
    }

    pub fn json_rpc_address(&self) -> Url {
        let jrpc_port = self.instance().allocated_ports().get("jrpc").unwrap();
        Url::parse(&format!("http://localhost:{jrpc_port}/json_rpc")).unwrap()
    }

    pub async fn get_registration_info(&self) -> anyhow::Result<ValidatorRegistrationInfo> {
        let reg_file = self.instance.base_path().join("registration.json");
        let info = fs::read_to_string(reg_file)
            .await
            .context("Failed to load registration.json")?;
        let reg = json5::from_str(&info)?;
        Ok(reg)
    }
}

#[derive(Serialize, Deserialize)]
pub struct ValidatorRegistrationInfo {
    pub signature: ValidatorNodeSignature,
    pub public_key: PublicKey,
    pub claim_fees_public_key: PublicKey,
}
