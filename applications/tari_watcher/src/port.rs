// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause
use std::net::{IpAddr, SocketAddr};

use anyhow::bail;
use tokio::net::TcpListener;

use crate::config::InstanceType;

#[derive(Clone)]
pub struct PortAllocator {
    pub validator: ValidatorPorts,
    pub env: Vec<(String, String)>,
    pub wallet: MinotariPorts,
}

impl PortAllocator {
    pub fn new(env: Vec<(String, String)>) -> Self {
        Self {
            validator: ValidatorPorts::new(),
            env,
            wallet: MinotariPorts::new(),
        }
    }

    fn vn_json_rpc_port(&self) -> Option<u32> {
        self.env
            .iter()
            .find(|(k, _)| k == "VN_JSON_RPC_PORT")
            .map(|(_, v)| v.parse().unwrap())
    }

    fn vn_http_port(&self) -> Option<u32> {
        self.env
            .iter()
            .find(|(k, _)| k == "VN_HTTP_PORT")
            .map(|(_, v)| v.parse().unwrap())
    }

    pub async fn open_vn_ports(&mut self, instance: InstanceType) -> anyhow::Result<()> {
        if instance != InstanceType::TariValidatorNode {
            log::error!("Unrecognized instance type {}", instance);
            bail!("Unrecognized instance type {}", instance.to_string());
        }

        if let Some(port) = self.vn_json_rpc_port() {
            log::info!("VN json rpc port started at {}", port);
            self.validator.jrpc = Some(port as u16);
        } else {
            // in case we are missing a port from config, allocate a random one
            let fallback = random_port().await?;
            self.validator.jrpc = Some(fallback);
            log::warn!("Missing validator node json rpc port from config, using: {}", fallback);
        }

        if let Some(port) = self.vn_http_port() {
            log::info!("VN http port started at {}", port);
            self.validator.web = Some(port as u16);
        } else {
            let fallback = random_port().await?;
            self.validator.web = Some(fallback);
            log::warn!("Missing validator node http port from config, using: {}", fallback);
        }

        Ok(())
    }
}

async fn random_port() -> anyhow::Result<u16> {
    let addr = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    Ok(addr.local_addr()?.port())
}

#[derive(Clone)]
pub struct ValidatorPorts {
    pub jrpc: Option<u16>,
    pub web: Option<u16>,
}

impl ValidatorPorts {
    fn new() -> Self {
        Self { jrpc: None, web: None }
    }

    pub fn jrpc_pub_address(&self, listen_ip: IpAddr) -> Option<String> {
        self.jrpc?;
        Some(format!("{}: {}", listen_ip, self.jrpc.unwrap()))
    }

    pub fn web_ui_address(&self, listen_ip: IpAddr) -> Option<String> {
        self.web?;
        Some(format!("{}:{}", listen_ip, self.web.unwrap()))
    }
}

#[derive(Clone)]
pub struct MinotariPorts {
    pub p2p: Option<u16>,
    pub grpc: Option<u16>,
}

#[allow(dead_code)]
impl MinotariPorts {
    fn new() -> Self {
        Self { p2p: None, grpc: None }
    }

    pub fn p2p_port_as_string(&self) -> Option<String> {
        self.p2p?;
        Some(format!("{}", self.p2p.unwrap()))
    }

    pub fn grpc_port_as_string(&self) -> Option<String> {
        self.grpc?;
        Some(format!("{}", self.grpc.unwrap()))
    }
}
