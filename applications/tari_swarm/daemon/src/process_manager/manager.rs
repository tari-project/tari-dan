//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, path::PathBuf, time::Duration};

use anyhow::anyhow;
use log::info;
use minotari_node_grpc_client::grpc;
use tari_common::configuration::Network;
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::TemplateAddress;
use tari_shutdown::ShutdownSignal;
use tokio::sync::mpsc;

use crate::{
    config::{InstanceType, ProcessesConfig},
    process_manager::{
        executables::ExecutableManager,
        handle::{ProcessManagerHandle, ProcessManagerRequest},
        instances::InstanceManager,
        TemplateData,
    },
};

pub struct ProcessManager {
    executable_manager: ExecutableManager,
    instance_manager: InstanceManager,
    rx_request: mpsc::Receiver<ProcessManagerRequest>,
    shutdown_signal: ShutdownSignal,
}

impl ProcessManager {
    pub fn new(
        base_dir: PathBuf,
        config: ProcessesConfig,
        network: Network,
        shutdown_signal: ShutdownSignal,
    ) -> (Self, ProcessManagerHandle) {
        let (tx_request, rx_request) = mpsc::channel(1);
        let this = Self {
            executable_manager: ExecutableManager::new(config.executables, config.always_compile),
            instance_manager: InstanceManager::new(base_dir, network, config.instances),
            rx_request,
            shutdown_signal,
        };
        (this, ProcessManagerHandle::new(tx_request))
    }

    pub async fn start(mut self) -> anyhow::Result<()> {
        log::info!("Starting process manager");
        let executables = self.executable_manager.prepare().await?;
        self.instance_manager.fork_all(executables).await?;

        self.register_all_validator_nodes().await?;

        loop {
            tokio::select! {
                Some(req) = self.rx_request.recv() => {
                    if let Err(err) = self.handle_request(req).await {
                        log::error!("Error handling request: {:?}", err);
                    }
                }

                // res = self.instance_manager.wait() => {
                //     res?;
                // }

                _ = self.shutdown_signal.wait() => {
                    log::info!("Shutting down process manager");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn handle_request(&mut self, req: ProcessManagerRequest) -> anyhow::Result<()> {
        match req {
            ProcessManagerRequest::CreateInstance {
                name,
                instance_type,
                args,
                reply,
            } => {
                let executable = self.executable_manager.get_executable(instance_type).ok_or_else(|| {
                    anyhow!(
                        "No configuration for instance type '{instance_type}'. Please add this to the configuration",
                    )
                })?;
                let instance_id = self
                    .instance_manager
                    .fork(executable, instance_type, name, &args)
                    .await?;
                if reply.send(Ok(instance_id)).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            },
            ProcessManagerRequest::DestroyInstance => {
                // self.instance_manager.kill_all().await?;
                unimplemented!();
            },
            ProcessManagerRequest::ListInstances { by_type, reply } => {
                let instances = self
                    .instance_manager
                    .instances()
                    .filter(|i| by_type.is_none() || i.instance_type() == by_type.unwrap())
                    .map(Into::into)
                    .collect();

                if reply.send(Ok(instances)).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            },
            ProcessManagerRequest::MineBlocks { blocks, reply } => {
                let result = self.mine(blocks).await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            },
            ProcessManagerRequest::RegisterTemplate { data, reply } => {
                let result = self.register_template(data).await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            },
        }

        Ok(())
    }

    async fn register_all_validator_nodes(&mut self) -> anyhow::Result<()> {
        for vn in self.instance_manager.validator_nodes_mut() {
            if !vn.instance_mut().is_running() {
                log::error!(
                    "Skipping registration for validator node {}: {} since it is not running",
                    vn.instance().id(),
                    vn.instance().name()
                );
                continue;
            }
        }

        let wallet = self.instance_manager.minotari_wallets().next().ok_or_else(|| {
            anyhow!(
                "No MinoTariConsoleWallet instances found. Please start a wallet before registering validator nodes"
            )
        })?;

        for vn in self.instance_manager.validator_nodes() {
            if let Err(err) = vn.wait_for_startup(Duration::from_secs(10)).await {
                log::error!(
                    "Skipping registration for validator node {}: {} since it is not responding",
                    vn.instance().id(),
                    err
                );
                continue;
            }

            let reg_info = vn.get_registration_info().await?;
            wallet.register_validator_node(reg_info).await?;
        }
        self.mine(20).await?;
        Ok(())
    }

    async fn mine(&mut self, blocks: u64) -> anyhow::Result<()> {
        let executable = self
            .executable_manager
            .get_executable(InstanceType::MinoTariMiner)
            .ok_or_else(|| {
                anyhow!("No executable configuration for 'MinoTariMiner'. Please add this to the configuration")
            })?;

        let args = HashMap::from([("max_blocks".to_string(), blocks.to_string())]);
        let id = self
            .instance_manager
            .fork(executable, InstanceType::MinoTariMiner, "miner".to_string(), &args)
            .await?;

        let status = self.instance_manager.wait(id).await?;
        if !status.success() {
            return Err(anyhow!("Failed to mine blocks. Process exited with {status}"));
        }

        Ok(())
    }

    async fn register_template(&mut self, data: TemplateData) -> anyhow::Result<()> {
        let wallet = self.instance_manager.minotari_wallets().next().ok_or_else(|| {
            anyhow!("No MinoTariConsoleWallet instances found. Please start a wallet before uploading a template")
        })?;

        let mut client = wallet.connect_client().await?;
        let resp = client
            .create_template_registration(grpc::CreateTemplateRegistrationRequest {
                fee_per_gram: 10,
                template_name: data.name,
                template_version: data.version,
                template_type: Some(grpc::TemplateType {
                    template_type: Some(grpc::template_type::TemplateType::Wasm(grpc::WasmInfo {
                        abi_version: 0,
                    })),
                }),
                build_info: Some(grpc::BuildInfo {
                    repo_url: "".to_string(),
                    commit_hash: vec![],
                }),
                binary_sha: data.contents_hash.to_vec(),
                binary_url: data.contents_url.to_string(),
                sidechain_deployment_key: vec![],
            })
            .await?
            .into_inner();
        let template_address = TemplateAddress::try_from_vec(resp.template_address).unwrap();
        info!("🟢 Registered template {}. Mining some blocks", template_address);
        self.mine(10).await?;

        Ok(())
    }
}
