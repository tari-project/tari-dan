//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::{
    config::InstanceType,
    process_manager::{AllocatedPorts, Instance, InstanceId},
};

type Reply<T> = oneshot::Sender<anyhow::Result<T>>;

pub enum ProcessManagerRequest {
    CreateInstance {
        name: String,
        instance_type: InstanceType,
        settings: HashMap<String, String>,
        reply: Reply<InstanceId>,
    },
    ListInstances {
        by_type: Option<InstanceType>,
        reply: Reply<Vec<InstanceInfo>>,
    },
    StartInstance {
        instance_id: InstanceId,
        reply: Reply<()>,
    },
    StopInstance {
        instance_id: InstanceId,
        reply: Reply<()>,
    },
    DeleteInstanceData {
        instance_id: InstanceId,
        reply: Reply<()>,
    },
    MineBlocks {
        blocks: u64,
        reply: Reply<()>,
    },
    RegisterValidatorNode {
        instance_id: InstanceId,
        reply: Reply<()>,
    },
    ExitValidatorNode {
        instance_id: InstanceId,
        reply: Reply<()>,
    },
    BurnFunds {
        amount: u64,
        wallet_instance_id: InstanceId,
        account_name: String,
        out_path: PathBuf,
        reply: Reply<PathBuf>,
    },
    GetMinotariNodeDetails {
        instance_id: InstanceId,
        reply: Reply<MinotariNodeDetails>,
    },
}

#[derive(Debug, Clone)]
pub struct MinotariNodeDetails {
    pub instance_info: InstanceInfo,
    pub height: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct InstanceInfo {
    pub id: InstanceId,
    pub name: String,
    pub ports: AllocatedPorts,
    pub base_path: PathBuf,
    /// Where the daemon captures this process's stdout and stderr. The parent of `base_path`, which points at the
    /// network subdirectory the process writes its own logs into.
    pub stdout_log_path: PathBuf,
    pub stderr_log_path: PathBuf,
    pub instance_type: InstanceType,
    pub settings: HashMap<String, String>,
    pub is_running: bool,
    /// True if the config file has been modified since this instance was last started.
    /// The user should restart the instance to apply the new configuration.
    pub is_config_dirty: bool,
}

impl InstanceInfo {
    pub fn get_public_web_url(&self) -> Url {
        match self.settings.get("public_web_url") {
            Some(url) => url
                .parse()
                .expect("Invalid web URL. Please check `public_web_url` in your config."),
            None => {
                let public_ip = self
                    .settings
                    .get("public_ip")
                    .map(|s| {
                        // Required for webauthn
                        if s == "127.0.0.1" {
                            return "localhost";
                        }
                        s.as_str()
                    })
                    .unwrap_or("localhost");
                let web_port = self.ports.get("web").expect("web port not found");
                format!("http://{public_ip}:{web_port}")
                    .parse()
                    .expect("Invalid web URL")
            },
        }
    }

    pub fn get_public_json_rpc_url(&self) -> Url {
        match self.settings.get("public_json_rpc_url") {
            Some(url) => url
                .parse()
                .expect("Invalid JSON RPC URL. Please check `public_json_rpc_url` in your config."),
            None => {
                let public_ip = self
                    .settings
                    .get("public_ip")
                    .map(|s| {
                        // Required for webauthn
                        if s == "127.0.0.1" {
                            return "localhost";
                        }
                        s.as_str()
                    })
                    .unwrap_or("localhost");
                let web_port = self.ports.get("jrpc").expect("jrpc port not found");
                format!("http://{public_ip}:{web_port}/json_rpc")
                    .parse()
                    .expect("Invalid JSON-RPC URL")
            },
        }
    }

    pub fn get_public_api_url(&self) -> Url {
        match self.settings.get("public_api_url") {
            Some(url) => url
                .parse()
                .expect("Invalid API URL. Please check `public_api_url` in your config."),
            None => {
                let public_ip = self
                    .settings
                    .get("public_ip")
                    .map(|s| {
                        // Required for webauthn
                        if s == "127.0.0.1" {
                            return "localhost";
                        }
                        s.as_str()
                    })
                    .unwrap_or("localhost");
                let web_port = self.ports.get("api").expect("api port not found");
                format!("http://{public_ip}:{web_port}")
                    .parse()
                    .expect("Invalid api URL")
            },
        }
    }

    pub fn get_public_graphql_url(&self) -> Url {
        match self.settings.get("public_graphql_url") {
            Some(url) => url
                .parse()
                .expect("Invalid GraphQL URL. Please check `public_graphql_url` in your config."),
            None => {
                let public_ip = self
                    .settings
                    .get("public_ip")
                    .map(|s| s.as_str())
                    .unwrap_or("127.0.0.1");
                let port = self
                    .ports
                    .get("graphql")
                    .expect("Graphql port must be allocated before calling get_graphql_url");
                format!("http://{public_ip}:{port}")
                    .parse()
                    .expect("Invalid GraphQL URL")
            },
        }
    }
}

impl From<&Instance> for InstanceInfo {
    fn from(instance: &Instance) -> Self {
        Self {
            id: instance.id(),
            name: instance.name().to_string(),
            ports: instance.allocated_ports().clone(),
            base_path: instance.base_path().clone(),
            stdout_log_path: instance.stdout_log_path(),
            stderr_log_path: instance.stderr_log_path(),
            instance_type: instance.instance_type(),
            settings: instance.settings().clone(),
            is_running: instance.is_running(),
            is_config_dirty: instance.is_config_dirty(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessManagerHandle {
    tx_request: mpsc::Sender<ProcessManagerRequest>,
}

impl ProcessManagerHandle {
    pub fn new(tx_request: mpsc::Sender<ProcessManagerRequest>) -> Self {
        Self { tx_request }
    }

    pub async fn create_instance<A: Into<HashMap<String, String>>>(
        &self,
        name: String,
        instance_type: InstanceType,
        settings: A,
    ) -> anyhow::Result<InstanceId> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::CreateInstance {
                name,
                instance_type,
                settings: settings.into(),
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub async fn list_instances(&self, by_type: Option<InstanceType>) -> anyhow::Result<Vec<InstanceInfo>> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::ListInstances {
                by_type,
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub async fn get_instance_by_name(&self, name: String) -> anyhow::Result<Option<InstanceInfo>> {
        let (tx_reply, rx_reply) = oneshot::channel();
        // TODO: consider optimizing this by adding a new request variant
        self.tx_request
            .send(ProcessManagerRequest::ListInstances {
                by_type: None,
                reply: tx_reply,
            })
            .await?;

        let intances = rx_reply.await??;
        Ok(intances.into_iter().find(|i| i.name == name))
    }

    pub async fn get_instance(&self, id: InstanceId) -> anyhow::Result<Option<InstanceInfo>> {
        let (tx_reply, rx_reply) = oneshot::channel();
        // TODO: consider optimizing this by adding a new request variant
        self.tx_request
            .send(ProcessManagerRequest::ListInstances {
                by_type: None,
                reply: tx_reply,
            })
            .await?;

        let intances = rx_reply.await??;
        Ok(intances.into_iter().find(|i| i.id == id))
    }

    pub async fn get_minotari_node_details(&self, id: InstanceId) -> anyhow::Result<MinotariNodeDetails> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::GetMinotariNodeDetails {
                instance_id: id,
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    // pub async fn list_minotari_nodes(&self) -> anyhow::Result<Vec<InstanceInfo>> {
    //     self.list_instances(Some(InstanceType::MinoTariNode)).await
    // }
    //
    // pub async fn list_minotari_console_wallets(&self) -> anyhow::Result<Vec<InstanceInfo>> {
    //     self.list_instances(Some(InstanceType::MinoTariConsoleWallet)).await
    // }

    pub async fn list_validator_nodes(&self) -> anyhow::Result<Vec<InstanceInfo>> {
        self.list_instances(Some(InstanceType::TariValidatorNode)).await
    }

    // pub async fn list_minotari_miners(&self) -> anyhow::Result<Vec<InstanceInfo>> {
    //     self.list_instances(Some(InstanceType::MinoTariMiner)).await
    // }

    pub async fn list_indexers(&self) -> anyhow::Result<Vec<InstanceInfo>> {
        self.list_instances(Some(InstanceType::TariIndexer)).await
    }

    pub async fn list_wallet_daemons(&self) -> anyhow::Result<Vec<InstanceInfo>> {
        self.list_instances(Some(InstanceType::TariWalletDaemon)).await
    }

    pub async fn mine_blocks(&self, blocks: u64) -> anyhow::Result<()> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::MineBlocks {
                blocks,
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub async fn start_instance(&self, instance_id: InstanceId) -> anyhow::Result<()> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::StartInstance {
                instance_id,
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub async fn stop_instance(&self, instance_id: InstanceId) -> anyhow::Result<()> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::StopInstance {
                instance_id,
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub async fn delete_instance_data(&self, instance_id: InstanceId) -> anyhow::Result<()> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::DeleteInstanceData {
                instance_id,
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub async fn register_validator_node(&self, instance_id: InstanceId) -> anyhow::Result<()> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::RegisterValidatorNode {
                instance_id,
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub async fn exit_validator_node(&self, instance_id: InstanceId) -> anyhow::Result<()> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::ExitValidatorNode {
                instance_id,
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub async fn burn_funds<P: AsRef<Path>>(
        &self,
        amount: u64,
        wallet_instance_id: InstanceId,
        account_name: String,
        out_path: P,
    ) -> anyhow::Result<PathBuf> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::BurnFunds {
                amount,
                wallet_instance_id,
                account_name,
                out_path: out_path.as_ref().to_path_buf(),
                reply: tx_reply,
            })
            .await?;

        rx_reply.await?
    }

    pub(crate) async fn stop_all(&self) -> anyhow::Result<usize> {
        let instances = self.list_instances(None).await?;
        log::info!("Stopping {} instances", instances.len());
        let num_instances = instances.len();
        for instance in instances {
            log::info!("Stopping instance: {}", instance.name);
            self.stop_instance(instance.id).await?;
        }
        log::info!("Stopped {} instances", num_instances);
        Ok(num_instances)
    }
}
