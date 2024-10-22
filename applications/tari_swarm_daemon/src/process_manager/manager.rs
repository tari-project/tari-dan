//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{
    config::{Config, InstanceType},
    process_manager::{
        executables::ExecutableManager,
        handle::{ProcessManagerHandle, ProcessManagerRequest},
        instances::InstanceManager,
        InstanceId,
        TemplateData,
    },
};
use anyhow::{anyhow, Context};
use log::info;
use minotari_node_grpc_client::grpc;
use std::str::FromStr;
use std::{collections::HashMap, fs::File, path::PathBuf, time::Duration};
use tari_common_types::types::FixedHash;
use tari_crypto::tari_utilities::{hex, ByteArray};
use tari_dan_engine::wasm::WasmModule;
use tari_engine_types::{calculate_template_binary_hash, TemplateAddress};
use tari_shutdown::ShutdownSignal;
use tari_validator_node_client::types::{GetTemplateResponse, GetTemplatesRequest, GetTemplatesResponse, TemplateMetadata};
use tokio::{sync::mpsc, time::sleep};
use url::Url;

pub struct ProcessManager {
    executable_manager: ExecutableManager,
    instance_manager: InstanceManager,
    rx_request: mpsc::Receiver<ProcessManagerRequest>,
    shutdown_signal: ShutdownSignal,
    skip_registration: bool,
    disable_template_auto_register: bool,
    base_dir: PathBuf,
    web_server_port: u16,
}

impl ProcessManager {
    pub fn new(config: &Config, shutdown_signal: ShutdownSignal) -> (Self, ProcessManagerHandle) {
        let (tx_request, rx_request) = mpsc::channel(1);
        let this = Self {
            skip_registration: config.skip_registration,
            executable_manager: ExecutableManager::new(
                config.processes.executables.clone(),
                config.processes.force_compile,
            ),
            instance_manager: InstanceManager::new(
                config.base_dir.clone(),
                config.network,
                config.processes.instances.clone(),
                config.start_port,
            ),
            rx_request,
            shutdown_signal,
            disable_template_auto_register: !config.auto_register_previous_templates,
            base_dir: config.base_dir.clone(),
            web_server_port: config.webserver.bind_address.port(),
        };
        (this, ProcessManagerHandle::new(tx_request))
    }

    async fn setup(&mut self) -> anyhow::Result<()> {
        info!("Starting process manager");
        let executables = self.executable_manager.prepare_all().await?;
        self.instance_manager.fork_all(executables).await?;

        // Wait some time for all instances to start
        sleep(Duration::from_secs(self.instance_manager.num_instances() as u64)).await;
        self.check_instances_running()?;

        if !self.skip_registration {
            let num_vns = self.instance_manager.num_validator_nodes();
            // Mine some initial funds, guessing 10 blocks to allow for coinbase maturity
            self.mine(num_vns + 10).await.context("mining failed")?;
            self.wait_for_wallet_funds(num_vns)
                .await
                .context("waiting for wallet funds")?;

            self.register_all_validator_nodes()
                .await
                .context("registering validator node via GRPC")?;
        }

        if !self.disable_template_auto_register {
            let registered_templates = self.registered_templates().await?;
            let registered_template_hashes: Vec<FixedHash> = registered_templates.iter()
                .map(|template_data| template_data.contents_hash)
                .collect();
            let fs_templates = self.file_system_templates().await?;
            for template_data in fs_templates.iter()
                .filter(|fs_template_data| {
                    !registered_template_hashes.contains(&fs_template_data.contents_hash)
                }) {
                info!("🟡 Register missing template from local file system: {}", template_data.name);
                self.register_template(TemplateData {
                    name: template_data.name.clone(),
                    version: template_data.version,
                    contents_hash: template_data.contents_hash,
                    contents_url: template_data.contents_url.clone(),
                }).await?;
            }
        }

        Ok(())
    }

    async fn file_system_templates(&self) -> anyhow::Result<Vec<TemplateData>> {
        let templates_dir = self.base_dir.join("templates");
        let mut templates_dir_content = tokio::fs::read_dir(templates_dir).await?;
        let mut result = vec![];
        while let Some(dir_entry) = templates_dir_content.next_entry().await? {
            if dir_entry.path().is_file() && dir_entry.path().ends_with(".wasm") {
                let file_name = dir_entry.file_name();
                let file_name = file_name.to_str().ok_or(anyhow!("Can't get file name!"))?;
                let file_content = tokio::fs::read(dir_entry.path()).await?;
                let loaded = WasmModule::load_template_from_code(file_content.as_slice())?;
                let name = loaded.template_def().template_name().to_string();
                let hash = calculate_template_binary_hash(&file_content);
                result.push(TemplateData {
                    name,
                    version: 0,
                    contents_hash: hash,
                    contents_url: Url::parse(&format!(
                        "http://localhost:{}/templates/{}",
                        self.web_server_port,
                        file_name
                    ))?,
                }
                )
            }
        }

        Ok(result)
    }

    async fn registered_templates(&self) -> anyhow::Result<Vec<TemplateData>> {
        let process = self.instance_manager.validator_nodes().next().ok_or_else(|| {
            anyhow!("No MinoTariConsoleWallet instances found. Please start a wallet before uploading a template")
        })?;

        let mut client = process.connect_client()?;
        Ok(
            client.get_active_templates(GetTemplatesRequest { limit: 10_000 }).await?
                .templates
                .iter()
                .map(|metadata| {
                    TemplateData {
                        name: metadata.name.clone(),
                        version: 0,
                        contents_hash: FixedHash::try_from(metadata.binary_sha.as_slice())
                            .unwrap_or_default(),
                        contents_url: Url::from_str(metadata.url.as_str()).unwrap(),
                    }
                })
                .collect()
        )
    }

    fn check_instances_running(&mut self) -> anyhow::Result<()> {
        for instance in self
            .instance_manager
            .instances_mut()
            .filter(|i| !i.instance_type().is_tari_node() && !i.instance_type().is_miner())
        {
            if let Some(status) = instance.check_running()? {
                return Err(anyhow!(
                    "Failed to start instance: {} {} {}",
                    instance.name(),
                    instance.instance_type(),
                    status
                ));
            }
        }
        Ok(())
    }

    pub async fn start(mut self) -> anyhow::Result<()> {
        let mut shutdown_signal = self.shutdown_signal.clone();

        tokio::select! {
            result = self.setup() => {
                 result?;
            },
            _ = shutdown_signal.wait() => {
                info!("Shutting down process manager");
                return Ok(());
            }
        }

        loop {
            tokio::select! {
                Some(req) = self.rx_request.recv() => {
                    if let Err(err) = self.handle_request(req).await {
                        log::error!("Error handling request: {:?}", err);
                    }
                }

                _ = self.shutdown_signal.wait() => {
                    info!("Shutting down process manager");
                    break;
                }
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_request(&mut self, req: ProcessManagerRequest) -> anyhow::Result<()> {
        use ProcessManagerRequest::*;
        match req {
            CreateInstance {
                name,
                instance_type,
                args,
                reply,
            } => {
                if self.instance_manager.instances().any(|i| i.name() == name) {
                    if reply
                        .send(Err(anyhow!(
                            "Instance with name '{name}' already exists. Please choose a different name",
                        )))
                        .is_err()
                    {
                        log::warn!("Request cancelled before response could be sent")
                    }
                    return Ok(());
                }

                let executable = self.executable_manager.get_executable(instance_type).ok_or_else(|| {
                    anyhow!(
                        "No configuration for instance type '{instance_type}'. Please add this to the configuration",
                    )
                })?;
                let result = self
                    .instance_manager
                    .fork_new(executable, instance_type, name, args)
                    .await;

                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
            ListInstances { by_type, reply } => {
                let instances = self
                    .instance_manager
                    .instances()
                    .filter(|i| by_type.is_none() || i.instance_type() == by_type.unwrap())
                    .map(Into::into)
                    .collect();

                if reply.send(Ok(instances)).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
            StartInstance { instance_id, reply } => {
                let executable = {
                    let instance = self
                        .instance_manager
                        .instances()
                        .find(|i| i.id() == instance_id)
                        .ok_or_else(|| anyhow!("Instance with ID '{}' not found", instance_id))?;
                    let instance_type = instance.instance_type();
                    self.executable_manager
                        .compile_executable_if_required(instance_type)
                        .await?
                };

                let result = self.instance_manager.start_instance(instance_id, executable).await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
            StopInstance { instance_id, reply } => {
                let result = self.instance_manager.stop_instance(instance_id).await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
            DeleteInstanceData { instance_id, reply } => {
                let result = self.instance_manager.delete_instance_data(instance_id).await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
            MineBlocks { blocks, reply } => {
                let result = self.mine(blocks).await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
            RegisterTemplate { data, reply } => {
                let result = self.register_template(data).await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
            RegisterValidatorNode { instance_id, reply } => {
                let result = self.register_validator_node(instance_id).await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
            BurnFunds {
                amount,
                wallet_instance_id,
                account_name,
                out_path,
                reply,
            } => {
                let result = self
                    .burn_funds_to_wallet_account(amount, wallet_instance_id, account_name, out_path)
                    .await;
                if reply.send(result).is_err() {
                    log::warn!("Request cancelled before response could be sent")
                }
            }
        }

        Ok(())
    }

    async fn burn_funds_to_wallet_account(
        &mut self,
        amount: u64,
        wallet_instance_id: InstanceId,
        account_name: String,
        out_path: PathBuf,
    ) -> anyhow::Result<PathBuf> {
        let wallet = self
            .instance_manager
            .get_wallet_daemon_mut(wallet_instance_id)
            .ok_or_else(|| {
                anyhow!(
                    "No DanTariConsoleWallet instances {wallet_instance_id} found. Please start a wallet before \
                     burning funds"
                )
            })?;
        let claim_public_key = wallet.get_account_public_key(account_name.clone()).await?;
        let wallet = self
            .instance_manager
            .minotari_wallets()
            .next()
            .ok_or_else(|| anyhow!("No MinoTariConsoleWallet instances found"))?;

        let proof = wallet.burn_funds(amount, &claim_public_key).await?;

        let file_name = PathBuf::from(format!("burn_proof-{}.json", proof.tx_id));
        let path = out_path.join(&file_name);
        let mut file = File::create(path)?;
        serde_json::to_writer_pretty(&mut file, &proof)?;

        info!("🔥 Burned {amount} Tari to account {account_name}");
        self.mine(10).await?;
        Ok(file_name)
    }

    async fn register_all_validator_nodes(&mut self) -> anyhow::Result<()> {
        let mut skip = vec![];
        for vn in self.instance_manager.validator_nodes_mut() {
            if let Some(status) = vn.instance_mut().check_running()? {
                log::error!(
                    "Skipping registration for validator node {}: {} since it is not running: {}",
                    vn.instance().id(),
                    vn.instance().name(),
                    status
                );
                skip.push(vn.instance().id());
            }
        }

        let wallet = self
            .instance_manager
            .minotari_wallets()
            .find(|w| w.instance().is_running())
            .ok_or_else(|| {
                anyhow!(
                    "No running MinoTariConsoleWallet instances found. Please start a wallet before registering \
                     validator nodes"
                )
            })?;

        for vn in self.instance_manager.validator_nodes() {
            if skip.contains(&vn.instance().id()) {
                continue;
            }
            info!("🟡 Registering validator node {}", vn.instance().name());
            if let Err(err) = vn.wait_for_startup(Duration::from_secs(10)).await {
                log::error!(
                    "Skipping registration for validator node {}: {} since it is not responding",
                    vn.instance().id(),
                    err
                );
                continue;
            }

            let reg_info = vn.get_registration_info().await?;
            let tx_id = wallet.register_validator_node(reg_info).await?;
            info!("🟢 Registered validator node {vn} with tx_id: {tx_id}");
            // Just wait a bit :shrug: This could be a bug in the console wallet. If we submit too quickly it uses 0
            // inputs for a transaction.
            sleep(Duration::from_secs(2)).await;
        }
        self.mine(20).await?;
        Ok(())
    }

    async fn register_validator_node(&mut self, instance_id: InstanceId) -> anyhow::Result<()> {
        let vn = self
            .instance_manager
            .validator_nodes()
            .find(|vn| vn.instance().id() == instance_id)
            .ok_or_else(|| anyhow!("Validator node with ID '{}' not found", instance_id))?;

        if !vn.instance().is_running() {
            log::error!(
                "Skipping registration for validator node {}: {} since it is not running",
                vn.instance().id(),
                vn.instance().name()
            );
            return Ok(());
        }

        if let Err(err) = vn.wait_for_startup(Duration::from_secs(10)).await {
            log::error!(
                "Skipping registration for validator node {}: {} since it is not responding",
                vn.instance().id(),
                err
            );
            return Ok(());
        }

        let wallet = self.instance_manager.minotari_wallets().next().ok_or_else(|| {
            anyhow!(
                "No MinoTariConsoleWallet instances found. Please start a wallet before registering validator nodes"
            )
        })?;

        let reg_info = vn.get_registration_info().await?;
        wallet.register_validator_node(reg_info).await?;
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
            .fork_new(executable, InstanceType::MinoTariMiner, "miner".to_string(), args)
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
        info!("🟢 Registered template {template_address}. Mining some blocks");
        self.mine(10).await?;

        Ok(())
    }

    async fn wait_for_wallet_funds(&mut self, min_expected_blocks: u64) -> anyhow::Result<()> {
        // WARN: Assumes one wallet
        let wallet = self.instance_manager.minotari_wallets().next().ok_or_else(|| {
            anyhow!("No MinoTariConsoleWallet instances found. Please start a wallet before waiting for funds")
        })?;

        loop {
            let resp = wallet.get_balance().await?;
            // Total guess of the minimum funds
            if resp.available_balance > min_expected_blocks * 5000000 {
                info!("💰 Wallet has funds. Available balance: {}", resp.available_balance);
                break;
            }
            sleep(Duration::from_secs(2)).await;
            info!("💱 Waiting for wallet to mine some funds");
        }

        Ok(())
    }
}
