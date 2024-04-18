//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, net::IpAddr, path::PathBuf};

use tari_common::configuration::Network;

use crate::process_manager::{
    AllocatedPorts,
    IndexerProcess,
    InstanceId,
    InstanceManager,
    MinoTariNodeProcess,
    MinoTariWalletProcess,
};

pub struct ProcessContext<'a> {
    instance_id: InstanceId,
    bin: &'a PathBuf,
    base_path: PathBuf,
    network: Network,
    local_ip: IpAddr,
    port_allocator: &'a mut AllocatedPorts,
    instances: &'a InstanceManager,
    args: &'a HashMap<String, String>,
}

impl<'a> ProcessContext<'a> {
    pub(crate) fn new(
        instance_id: InstanceId,
        bin: &'a PathBuf,
        base_path: PathBuf,
        network: Network,
        local_ip: IpAddr,
        port_allocator: &'a mut AllocatedPorts,
        instances: &'a InstanceManager,
        args: &'a HashMap<String, String>,
    ) -> Self {
        Self {
            instance_id,
            bin,
            base_path,
            network,
            local_ip,
            port_allocator,
            instances,
            args,
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

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn get_arg(&self, key: &str) -> Option<&String> {
        self.args.get(key)
    }

    pub async fn get_free_port(&mut self, name: &'static str) -> anyhow::Result<u16> {
        Ok(self.port_allocator.next_port(name).await)
    }

    pub fn local_ip(&self) -> &IpAddr {
        &self.local_ip
    }

    pub fn environment(&self) -> Vec<(&str, &str)> {
        vec![]
    }

    pub fn minotari_nodes(&self) -> impl Iterator<Item = &MinoTariNodeProcess> {
        self.instances.minotari_nodes()
    }

    pub fn minotari_wallets(&self) -> impl Iterator<Item = &MinoTariWalletProcess> {
        self.instances.minotari_wallets()
    }

    pub fn indexers(&self) -> impl Iterator<Item = &IndexerProcess> {
        self.instances.indexers()
    }
}
