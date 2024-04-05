//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{net::IpAddr, path::PathBuf};

use async_trait::async_trait;
use tari_common::configuration::Network;
use tokio::process::Command;

use crate::process_manager::{InstanceId, PortAllocator};

#[async_trait]
pub trait ProcessDefinition: Send {
    async fn get_command(&self, context: ProcessContext<'_>) -> anyhow::Result<Command>;
}

pub struct ProcessContext<'a> {
    instance_id: InstanceId,
    bin: &'a PathBuf,
    base_path: PathBuf,
    network: Network,
    local_ip: IpAddr,
    port_allocator: &'a mut PortAllocator,
}

impl<'a> ProcessContext<'a> {
    pub(crate) fn new(
        instance_id: InstanceId,
        bin: &'a PathBuf,
        base_path: PathBuf,
        network: Network,
        local_ip: IpAddr,
        port_allocator: &'a mut PortAllocator,
    ) -> Self {
        Self {
            instance_id,
            bin,
            base_path,
            network,
            local_ip,
            port_allocator,
        }
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

    pub async fn get_free_port(&mut self, name: &'static str) -> anyhow::Result<u16> {
        Ok(self.port_allocator.next_port(self.instance_id, name).await)
    }

    pub fn local_ip(&self) -> &IpAddr {
        &self.local_ip
    }

    pub fn environment(&self) -> Vec<(&str, &str)> {
        vec![]
    }
}
