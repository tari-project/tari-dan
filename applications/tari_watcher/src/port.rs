// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause
use std::{
    io,
    net::{IpAddr, SocketAddr},
};

use tokio::net::TcpListener;

use crate::config::InstanceType;

#[derive(Clone)]
pub struct PortAllocator {
    pub validator: ValidatorPorts,
    pub wallet: MinotariPorts,
}

impl PortAllocator {
    pub fn new() -> Self {
        Self {
            validator: ValidatorPorts::new(),
            wallet: MinotariPorts::new(),
        }
    }

    pub async fn open_at(&mut self, instance: InstanceType, name: &'static str) -> io::Result<u16> {
        let addr = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
        let port = addr.local_addr()?.port();
        match instance {
            InstanceType::TariValidatorNode => {
                if name == "jrpc" {
                    self.validator.jrpc = Some(port)
                } else if name == "web" {
                    self.validator.web = Some(port)
                } else {
                    log::error!("Invalid port name for {} instance: {}", instance, port);
                }
            },
            InstanceType::MinoTariConsoleWallet => {
                if name == "p2p" {
                    self.wallet.p2p = Some(port)
                } else if name == "grpc" {
                    self.wallet.grpc = Some(port)
                } else {
                    log::error!("Invalid port name for {} instance: {}", instance, port);
                }
            },
        }

        log::info!("Started a {}-{} port on {}", instance.to_string(), name, port);

        Ok(port)
    }
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
