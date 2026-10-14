//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    net::IpAddr,
    path::{Path, PathBuf},
};

use tari_ootle_wallet_sdk::Network;
use url::Url;

use crate::process_manager::{
    AllocatedPorts,
    IndexerProcess,
    InstanceId,
    InstanceManager,
    MinoTariNodeProcess,
    MinoTariWalletProcess,
    ValidatorNodeProcess,
};

/// Name of the swarm-wide consensus constants file. It is written on start-up so that it can simply
/// be edited, and every node the swarm forks is pointed at it.
pub const CONSENSUS_CONSTANTS_FILE_NAME: &str = "consensus_constants.toml";

pub struct ProcessContext<'a> {
    instance_id: InstanceId,
    bin: &'a PathBuf,
    envs: &'a [(String, String)],
    base_path: PathBuf,
    processes_path: PathBuf,
    swarm_path: PathBuf,
    network: Network,
    listen_ip: IpAddr,
    port_allocator: &'a mut AllocatedPorts,
    instances: &'a InstanceManager,
    settings: &'a HashMap<String, String>,
}

impl<'a> ProcessContext<'a> {
    pub(crate) fn new(
        instance_id: InstanceId,
        bin: &'a PathBuf,
        envs: &'a [(String, String)],
        base_path: PathBuf,
        processes_path: PathBuf,
        swarm_path: PathBuf,
        network: Network,
        listen_ip: IpAddr,
        port_allocator: &'a mut AllocatedPorts,
        instances: &'a InstanceManager,
        settings: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            instance_id,
            bin,
            envs,
            base_path,
            processes_path,
            swarm_path,
            network,
            listen_ip,
            port_allocator,
            instances,
            settings,
        }
    }

    pub fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    pub fn bin(&self) -> &PathBuf {
        self.bin
    }

    pub fn base_path(&self) -> &PathBuf {
        &self.base_path
    }

    pub fn processes_path(&self) -> &PathBuf {
        &self.processes_path
    }

    /// The swarm's own directory, which holds the files shared by every instance in it.
    pub fn swarm_path(&self) -> &Path {
        &self.swarm_path
    }

    /// The consensus constants every node in this swarm reads. Constants have to agree across a
    /// network, so this is one file for the swarm rather than one per instance.
    pub fn consensus_constants_file(&self) -> PathBuf {
        self.swarm_path().join(CONSENSUS_CONSTANTS_FILE_NAME)
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn get_setting(&self, key: &str) -> Option<&str> {
        self.settings.get(key).map(|s| s.as_str())
    }

    pub fn get_public_api_url(&self) -> Url {
        match self.settings.get("public_api_url") {
            Some(url) => url.parse().expect("Invalid API URL"),
            None => {
                let public_ip = self
                    .settings
                    .get("public_ip")
                    .map(|s| {
                        if s == "127.0.0.1" {
                            return "localhost";
                        }
                        s.as_str()
                    })
                    .unwrap_or("localhost");
                let port = self
                    .port_allocator
                    .get("api")
                    .expect("API port must be allocated before calling get_public_api_url");
                format!("http://{public_ip}:{port}").parse().expect("Invalid API URL")
            },
        }
    }

    pub async fn get_free_port(&mut self, name: &'static str) -> anyhow::Result<u16> {
        Ok(self.port_allocator.get_or_next_port(name).await)
    }

    pub fn port_allocator_mut(&mut self) -> &mut AllocatedPorts {
        self.port_allocator
    }

    pub fn listen_ip(&self) -> &IpAddr {
        &self.listen_ip
    }

    pub fn environment(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.envs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn minotari_nodes(&self) -> impl Iterator<Item = &MinoTariNodeProcess> {
        self.instances.minotari_nodes()
    }

    pub fn validator_nodes(&self) -> impl Iterator<Item = &ValidatorNodeProcess> {
        self.instances.validator_nodes()
    }

    pub fn minotari_wallets(&self) -> impl Iterator<Item = &MinoTariWalletProcess> {
        self.instances.minotari_wallets()
    }

    pub fn indexers(&self) -> impl Iterator<Item = &IndexerProcess> {
        self.instances.indexers()
    }
}
