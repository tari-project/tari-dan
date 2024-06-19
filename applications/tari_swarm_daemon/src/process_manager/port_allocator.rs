//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, net::SocketAddr};

use tokio::net::TcpListener;

use crate::process_manager::InstanceId;

pub struct PortAllocator {
    instances: HashMap<InstanceId, AllocatedPorts>,
    current_port: u16,
}

impl PortAllocator {
    pub fn new(start_port: u16) -> Self {
        Self {
            instances: HashMap::new(),
            current_port: start_port,
        }
    }

    // pub fn get_ports(&self, instance_id: InstanceId) -> Option<&AllocatedPorts> {
    //     self.instances.get(&instance_id)
    // }

    pub fn create(&mut self) -> AllocatedPorts {
        AllocatedPorts {
            ports: HashMap::new(),
            current_port: self.current_port,
        }
    }

    pub fn register(&mut self, instance_id: InstanceId, ports: AllocatedPorts) {
        self.current_port = ports.current_port;
        self.instances.insert(instance_id, ports);
    }
}

#[derive(Debug, Clone)]
pub struct AllocatedPorts {
    current_port: u16,
    ports: HashMap<&'static str, u16>,
}

impl AllocatedPorts {
    pub fn get(&self, name: &'static str) -> Option<u16> {
        self.ports.get(name).copied()
    }

    pub fn expect(&self, name: &'static str) -> u16 {
        self.ports[name]
    }

    pub fn into_ports(self) -> HashMap<&'static str, u16> {
        self.ports
    }

    pub async fn next_port(&mut self, name: &'static str) -> u16 {
        loop {
            let port = self.current_port;
            self.current_port += 1;
            if check_local_port(port).await {
                log::debug!("Port {port} is free for {name}");
                self.ports.insert(name, port);
                return port;
            }
        }
    }
}

// pub struct InstancePortAllocator<'a> {
//     ports: &'a mut HashMap<&'static str, u16>,
//     current_port: &'a mut u16,
// }
//
//

async fn check_local_port(port: u16) -> bool {
    log::debug!("Checking port {}", port);
    TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))
        .await
        .is_ok()
}
