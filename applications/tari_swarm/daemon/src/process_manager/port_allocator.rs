//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, net::SocketAddr};

use tokio::net::TcpListener;

use crate::process_manager::InstanceId;

pub struct PortAllocator {
    ports: HashMap<InstanceId, HashMap<&'static str, u16>>,
    current_port: u16,
}

impl PortAllocator {
    pub fn new() -> Self {
        Self {
            ports: HashMap::new(),
            current_port: 12000,
        }
    }

    pub async fn next_port(&mut self, instance_id: InstanceId, name: &'static str) -> u16 {
        loop {
            let port = self.current_port;
            self.current_port += 1;
            if check_local_port(port).await {
                self.ports.entry(instance_id).or_default().insert(name, port);
                return port;
            }
        }
    }
}

async fn check_local_port(port: u16) -> bool {
    TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .is_ok()
}
