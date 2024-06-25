//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf};

use tari_common_types::types::FixedHash;
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
        args: HashMap<String, String>,
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
    RegisterTemplate {
        data: TemplateData,
        reply: Reply<()>,
    },
    RegisterValidatorNode {
        instance_id: InstanceId,
        reply: Reply<()>,
    },
}

pub struct TemplateData {
    pub name: String,
    pub version: u32,
    pub contents_hash: FixedHash,
    pub contents_url: Url,
}

pub struct InstanceInfo {
    pub id: InstanceId,
    pub name: String,
    pub ports: AllocatedPorts,
    pub base_path: PathBuf,
    pub instance_type: InstanceType,
    pub is_running: bool,
}

impl From<&Instance> for InstanceInfo {
    fn from(instance: &Instance) -> Self {
        Self {
            id: instance.id(),
            name: instance.name().to_string(),
            ports: instance.allocated_ports().clone(),
            base_path: instance.base_path().clone(),
            instance_type: instance.instance_type(),
            is_running: instance.is_running(),
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
        args: A,
    ) -> anyhow::Result<InstanceId> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::CreateInstance {
                name,
                instance_type,
                args: args.into(),
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

    pub async fn register_template(&self, data: TemplateData) -> anyhow::Result<()> {
        let (tx_reply, rx_reply) = oneshot::channel();
        self.tx_request
            .send(ProcessManagerRequest::RegisterTemplate { data, reply: tx_reply })
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

    pub(crate) async fn stop_all(&self) -> anyhow::Result<usize> {
        let instances = self.list_instances(None).await?;
        let num_instances = instances.len();
        for instance in instances {
            self.stop_instance(instance.id).await?;
        }
        Ok(num_instances)
    }
}
