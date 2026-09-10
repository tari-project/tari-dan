//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    io::BufReader as StdBufReader,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use anyhow::{Context, anyhow};
use indexmap::IndexMap;
use log::{debug, info};
use slug::slugify;
use tari_ootle_wallet_sdk::Network;
use tokio::{
    fs,
    fs::File,
    io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader},
    task,
    time::sleep,
};

use super::InstanceId;

/// Written to the swarm directory when there is no constants file there. Everything is commented
/// out, so it changes nothing until a line is uncommented, and it lists what may be changed without
/// having to go and look.
const CONSENSUS_CONSTANTS_TEMPLATE: &str = r#"# Consensus constants for this swarm, read once at
# start-up by every node in it. Only LocalNet reads this file at all. Uncomment a line to change it
# and restart the swarm; anything left commented keeps the network's own value.
#
# Every value below is the one this network already uses, so uncommenting a line as it stands changes
# nothing. The number of committees is the registered validator count divided by the committee size,
# so a smaller committee splits the shard space across fewer nodes.
# committee_size_per_shard_group = 7

# pacemaker_block_time_secs = 10
# base_layer_confirmations = 3
# missed_proposal_suspend_threshold = 5
# missed_proposal_evict_threshold = 10
# missed_proposal_recovery_threshold = 5
# max_transaction_validity_epochs = 2160
"#;
use crate::{
    config::{InstanceConfig, InstanceType},
    logger::FORWARDED_TARGET,
    process_definitions::{CONSENSUS_CONSTANTS_FILE_NAME, ProcessContext, get_definition},
    process_manager::{
        AllocatedPorts,
        IndexerProcess,
        Instance,
        WalletDaemonProcess,
        executables::{Executable, Executables},
        port_allocator::PortAllocator,
        processes::{MinoTariMinerProcess, MinoTariNodeProcess, MinoTariWalletProcess, ValidatorNodeProcess},
    },
};

pub struct InstanceManager {
    base_path: PathBuf,
    config: Vec<InstanceConfig>,
    global_settings: HashMap<String, String>,
    network: Network,
    minotari_nodes: IndexMap<InstanceId, MinoTariNodeProcess>,
    minotari_wallets: IndexMap<InstanceId, MinoTariWalletProcess>,
    minotari_miners: IndexMap<InstanceId, MinoTariMinerProcess>,
    validator_nodes: IndexMap<InstanceId, ValidatorNodeProcess>,
    indexers: IndexMap<InstanceId, IndexerProcess>,
    wallet_daemons: IndexMap<InstanceId, WalletDaemonProcess>,
    port_allocator: PortAllocator,
    instance_id: InstanceId,
}

impl InstanceManager {
    pub fn new(
        base_path: PathBuf,
        network: Network,
        global_settings: HashMap<String, String>,
        config: Vec<InstanceConfig>,
        start_port: u16,
    ) -> Self {
        Self {
            base_path,
            port_allocator: PortAllocator::new(start_port, &config),
            config,
            network,
            global_settings,
            minotari_nodes: IndexMap::new(),
            minotari_wallets: IndexMap::new(),
            minotari_miners: IndexMap::new(),
            validator_nodes: IndexMap::new(),
            indexers: IndexMap::new(),
            wallet_daemons: IndexMap::new(),
            instance_id: 0,
        }
    }

    /// Fork all defined processes in order
    pub async fn fork_all(&mut self, executables: Executables<'_>) -> anyhow::Result<()> {
        self.write_consensus_constants_template().await?;
        for mut instance in self.config.clone() {
            let executable = executables.get(instance.execution_instance_type()).ok_or_else(|| {
                anyhow!(
                    "No executable found for instance type '{}'. This is a bug in the configuration",
                    instance.execution_instance_type()
                )
            })?;

            let mut settings = self.global_settings.clone();
            settings.extend(instance.settings.drain());
            for i in 0..instance.num_instances {
                self.fork_new(
                    executable,
                    instance.instance_type,
                    instance.instance_name(i),
                    instance.base_path_override().cloned(),
                    instance.envs.clone(),
                    settings.clone(),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Writes a commented consensus constants file into the swarm directory if there is not one
    /// already. Every node forked here is pointed at it, so a devnet is retuned by uncommenting a
    /// line and restarting rather than by finding out where the setting lives.
    async fn write_consensus_constants_template(&self) -> anyhow::Result<()> {
        let path = self.base_path.join(CONSENSUS_CONSTANTS_FILE_NAME);
        if fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(());
        }

        fs::create_dir_all(&self.base_path)
            .await
            .context("create_dir_all for the consensus constants file")?;
        fs::write(&path, CONSENSUS_CONSTANTS_TEMPLATE)
            .await
            .context("write the consensus constants file")?;
        log::info!("📝 Wrote consensus constants file {}", path.display());
        Ok(())
    }

    pub async fn fork_new(
        &mut self,
        executable: &Executable,
        instance_type: InstanceType,
        instance_name: String,
        base_path_override: Option<PathBuf>,
        envs: Vec<(String, String)>,
        settings: HashMap<String, String>,
    ) -> anyhow::Result<InstanceId> {
        let instance_id = self.next_instance_id();
        self.fork(
            instance_id,
            executable,
            instance_type,
            instance_name,
            base_path_override,
            envs,
            settings,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_lines)]
    async fn fork(
        &mut self,
        instance_id: InstanceId,
        executable: &Executable,
        instance_type: InstanceType,
        instance_name: String,
        base_path_override: Option<PathBuf>,
        instance_envs: Vec<(String, String)>,
        mut instance_settings: HashMap<String, String>,
        ports: Option<AllocatedPorts>,
    ) -> anyhow::Result<InstanceId> {
        let listen_ip = instance_settings
            .get("listen_ip")
            .map(|s| s.parse())
            .transpose()
            .context("Failed to parse listen_ip arg")?
            .unwrap_or_else(|| IpAddr::V4(Ipv4Addr::from([127, 0, 0, 1])));
        let definition = get_definition(instance_type);

        log::info!(
            "🚀 Starting {} (id: {}, exec path: {}, listen_ip: {})",
            instance_type,
            instance_id,
            executable.path.display(),
            listen_ip
        );

        let mut allocated_ports = ports.unwrap_or_else(|| self.port_allocator.create(instance_type));

        let processes_path = self.base_path.join("processes");
        let base_path = match base_path_override {
            Some(base_path) => {
                if base_path.is_absolute() {
                    base_path
                } else {
                    processes_path.join(base_path)
                }
            },
            None => processes_path.join(slugify(&instance_name)),
        };
        fs::create_dir_all(&base_path).await.context("create_dir_all in fork")?;

        // Special handling to set the claim public key if we find a file containing one
        if instance_type.is_validator() {
            let claim_public_key_file = processes_path.join("claim_key.json");
            if claim_public_key_file.exists() {
                let file = File::open(&claim_public_key_file)
                    .await
                    .context("Failed to open claim public key file")?;
                let file = file.into_std().await;
                let reader = StdBufReader::new(file);
                let claim_data = serde_json::from_reader::<_, serde_json::Value>(reader)
                    .context("Failed to read claim public key file")?;
                let claim_public_key = claim_data
                    .get("account_public_key")
                    .and_then(|pk| pk.as_str())
                    .ok_or_else(|| anyhow!("Failed to extract public key from claim public key file: {claim_data}"))?;
                info!("Setting claim public key to {}", claim_public_key);
                instance_settings.insert("claim_public_key".to_string(), claim_public_key.to_string());
            }
        }

        let context = ProcessContext::new(
            instance_id,
            &executable.path,
            &instance_envs,
            base_path.clone(),
            processes_path,
            self.base_path.clone(),
            self.network,
            listen_ip,
            &mut allocated_ports,
            self,
            &instance_settings,
        );
        if !context.bin().exists() {
            return Err(anyhow::anyhow!(
                "{} binary not found at {}",
                instance_type,
                context.bin().display()
            ));
        }

        let mut command = definition.get_command(context).await.context("get_command")?;
        let stdout_log_path = base_path.join("stdout.log");
        let stderr_log_path = base_path.join("stderr.log");
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Any attempt to use stdin will fail immediately
            .stdin(Stdio::null());
        debug!("Command: {:?}", command);
        let mut child = command.spawn().with_context(|| format!("spawn {instance_type}"))?;

        self.port_allocator.register(instance_id, allocated_ports.clone());

        if let Some(stdout) = child.stdout.take() {
            forward_logs(stdout_log_path, stdout, instance_name.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            forward_logs(stderr_log_path, stderr, instance_name.clone());
        }

        let mut instance = Instance::new_started(
            instance_id,
            instance_name,
            instance_type,
            child,
            allocated_ports,
            // This saves us from having to join the network string to the path all over the place, since everything we
            // want is under {base_dir}/{network}
            base_path.join(self.network.to_string()),
            base_path,
            instance_envs,
            instance_settings,
        );

        // Wait for base layer nodes to start
        if instance_type.is_base_layer_node() {
            sleep(Duration::from_secs(2)).await;
            instance.check_running().context("Failed to start instance")?;
        }

        log::info!(
            "🟢 Started {} (id: {}, path: {}, pid: {:?})",
            instance_type,
            instance_id,
            executable.path.display(),
            instance.child().id()
        );

        match instance_type {
            InstanceType::MinoTariNode => {
                self.minotari_nodes
                    .insert_sorted(instance_id, MinoTariNodeProcess::new(instance));
            },
            InstanceType::MinoTariConsoleWallet => {
                self.minotari_wallets
                    .insert_sorted(instance_id, MinoTariWalletProcess::new(instance));
            },
            InstanceType::MinoTariMiner => {
                self.minotari_miners
                    .insert_sorted(instance_id, MinoTariMinerProcess::new(instance));
            },
            InstanceType::TariValidatorNode => {
                self.validator_nodes
                    .insert_sorted(instance_id, ValidatorNodeProcess::new(instance));
            },
            InstanceType::TariIndexer => {
                self.indexers.insert_sorted(instance_id, IndexerProcess::new(instance));
            },
            InstanceType::TariWalletDaemon => {
                self.wallet_daemons
                    .insert_sorted(instance_id, WalletDaemonProcess::new(instance));
            },
            InstanceType::TariWalletDaemonCreateKey => {
                self.wallet_daemons
                    .insert_sorted(instance_id, WalletDaemonProcess::new(instance));
            },
        }

        // Wait a bit after starting the instance
        sleep(definition.after_start_delay()).await;

        Ok(instance_id)
    }

    pub fn minotari_nodes(&self) -> impl Iterator<Item = &MinoTariNodeProcess> + Sized {
        self.minotari_nodes.values()
    }

    pub fn minotari_wallets(&self) -> impl Iterator<Item = &MinoTariWalletProcess> + Sized {
        self.minotari_wallets.values()
    }

    pub fn validator_nodes(&self) -> impl Iterator<Item = &ValidatorNodeProcess> + Sized {
        self.validator_nodes.values()
    }

    pub fn num_instances(&self) -> usize {
        self.instances().count()
    }

    pub fn num_validator_nodes(&self) -> usize {
        self.validator_nodes.len()
    }

    pub fn validator_nodes_mut(&mut self) -> impl Iterator<Item = &mut ValidatorNodeProcess> + Sized {
        self.validator_nodes.values_mut()
    }

    // pub fn minotari_miners(&self) -> impl Iterator<Item = &MinoTariMinerProcess> + Sized {
    //     self.minotari_miners.values()
    // }

    pub fn indexers(&self) -> impl Iterator<Item = &IndexerProcess> + Sized {
        self.indexers.values()
    }

    // pub fn wallet_daemons(&self) -> impl Iterator<Item = &WalletDaemonProcess> + Sized {
    //     self.wallet_daemons.values()
    // }

    pub fn get_wallet_daemon_mut(&mut self, id: InstanceId) -> Option<&mut WalletDaemonProcess> {
        self.wallet_daemons.get_mut(&id)
    }

    pub fn get_instance_mut(&mut self, id: InstanceId) -> Option<&mut Instance> {
        self.instances_mut().find(|i| i.id() == id)
    }

    pub async fn wait(&mut self, id: InstanceId) -> anyhow::Result<ExitStatus> {
        let instance = self.get_instance_mut(id).ok_or_else(|| anyhow!("Instance not found"))?;
        let status = instance.child_mut().wait().await?;
        Ok(status)
    }

    pub async fn start_instance(&mut self, id: InstanceId, executable: &Executable) -> anyhow::Result<()> {
        let instance = self
            .instances()
            .find(|i| i.id() == id)
            .ok_or_else(|| anyhow!("Instance not found"))?;

        let instance_type = instance.instance_type();
        let instance_name = instance.name().to_string();
        let settings = instance.settings().clone();
        let ports = instance.allocated_ports().clone();
        let envs = instance.envs().to_vec();

        // This will just overwrite the previous instance
        self.fork(
            id,
            executable,
            instance_type,
            instance_name,
            None,
            envs,
            settings,
            Some(ports),
        )
        .await?;

        Ok(())
    }

    pub async fn stop_instance(&mut self, id: InstanceId) -> anyhow::Result<()> {
        let instance = self
            .instances_mut()
            .find(|i| i.id() == id)
            .ok_or_else(|| anyhow!("Instance not found"))?;

        info!("🛑 Stopping {} (id: {})", instance.instance_type(), instance.id());
        instance.terminate().await?;
        instance.check_running()?;
        Ok(())
    }

    pub async fn delete_instance_data(&mut self, id: InstanceId) -> anyhow::Result<()> {
        let instance = self
            .instances_mut()
            .find(|i| i.id() == id)
            .ok_or_else(|| anyhow!("Instance not found"))?;

        let definition = get_definition(instance.instance_type());

        if let Some(data_path) = definition.get_relative_data_path() {
            let path = instance.base_path().join(data_path);
            info!(
                "Deleting data directory for instance {}: {}",
                instance.name(),
                path.display()
            );
            fs::remove_dir_all(path).await?;
        }
        Ok(())
    }

    pub fn remove_instance(&mut self, id: InstanceId) -> anyhow::Result<()> {
        let instance = self
            .instances()
            .find(|i| i.id() == id)
            .ok_or_else(|| anyhow!("Instance not found"))?;

        match instance.instance_type() {
            InstanceType::MinoTariNode => {
                self.minotari_nodes.shift_remove(&id);
            },
            InstanceType::MinoTariConsoleWallet => {
                self.minotari_wallets.shift_remove(&id);
            },
            InstanceType::MinoTariMiner => {
                self.minotari_miners.shift_remove(&id);
            },
            InstanceType::TariValidatorNode => {
                self.validator_nodes.shift_remove(&id);
            },
            InstanceType::TariIndexer => {
                self.indexers.shift_remove(&id);
            },
            InstanceType::TariWalletDaemon => {
                self.wallet_daemons.shift_remove(&id);
            },
            InstanceType::TariWalletDaemonCreateKey => {
                self.wallet_daemons.shift_remove(&id);
            },
        }

        // Remove allocated any ports for instance
        self.port_allocator.unregister(id);

        Ok(())
    }

    pub fn instances_mut(&mut self) -> impl Iterator<Item = &mut Instance> {
        self.minotari_nodes
            .values_mut()
            .map(|x| x.instance_mut())
            .chain(self.minotari_wallets.values_mut().map(|x| x.instance_mut()))
            .chain(self.minotari_miners.values_mut().map(|x| x.instance_mut()))
            .chain(self.validator_nodes.values_mut().map(|x| x.instance_mut()))
            .chain(self.indexers.values_mut().map(|x| x.instance_mut()))
            .chain(self.wallet_daemons.values_mut().map(|x| x.instance_mut()))
    }

    pub fn instances(&self) -> impl Iterator<Item = &Instance> {
        self.minotari_nodes
            .values()
            .map(|x| x.instance())
            .chain(self.minotari_wallets.values().map(|x| x.instance()))
            .chain(self.minotari_miners.values().map(|x| x.instance()))
            .chain(self.validator_nodes.values().map(|x| x.instance()))
            .chain(self.indexers.values().map(|x| x.instance()))
            .chain(self.wallet_daemons.values().map(|x| x.instance()))
    }

    /// Apply updated instance configs from a reloaded config file.
    /// Updates settings and envs for matching instances and marks them as config dirty
    /// so that the user knows to restart them.
    pub fn apply_config_update(
        &mut self,
        new_global_settings: HashMap<String, String>,
        new_instance_configs: &[InstanceConfig],
    ) {
        for instance_config in new_instance_configs {
            for i in 0..instance_config.num_instances {
                let expected_name = instance_config.instance_name(i);
                let mut new_settings = new_global_settings.clone();
                new_settings.extend(instance_config.settings.clone());

                if let Some(instance) = self.instances_mut().find(|inst| inst.name() == expected_name) {
                    let settings_changed = *instance.settings() != new_settings;
                    let envs_changed = instance.envs() != instance_config.envs;
                    if settings_changed || envs_changed {
                        info!(
                            "📝 Config changed for instance '{}' (settings_changed={}, envs_changed={}). Restart to \
                             apply.",
                            expected_name, settings_changed, envs_changed,
                        );
                        instance.set_settings(new_settings);
                        instance.set_envs(instance_config.envs.clone());
                        instance.set_config_dirty(true);
                    }
                }
            }
        }
        self.global_settings = new_global_settings;
        self.config = new_instance_configs.to_vec();
    }

    fn next_instance_id(&mut self) -> InstanceId {
        let id = self.instance_id;
        self.instance_id += 1;
        id
    }
}

fn forward_logs<R: AsyncRead + Unpin + Send + 'static>(path: PathBuf, reader: R, target: String) {
    let mut lines = BufReader::new(reader).lines();
    task::spawn(async move {
        let mut log_file = match File::create(path).await {
            Ok(file) => file,
            Err(err) => {
                log::error!("Failed to create log file for {target}: {err}");
                return;
            },
        };
        while let Some(output) = lines.next_line().await.unwrap() {
            log::debug!(target: FORWARDED_TARGET, "[{target}] {output}");
            if let Err(err) = log_file.write_all(output.as_bytes()).await {
                log::error!("forward_logs: {err}");
                return;
            }
            if let Err(err) = log_file.write_all(b"\n").await {
                log::error!("forward_logs: {err}");
                return;
            }
            if let Err(err) = log_file.flush().await {
                log::error!("forward_logs: {err}");
                return;
            }
        }
        log::debug!(target: FORWARDED_TARGET, "Process exited ({target})");
    });
}
