// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

#[derive(Clone)]
pub struct PortAllocator {
    pub wallet: MinotariPorts,
}

impl PortAllocator {
    pub fn new() -> Self {
        Self {
            wallet: MinotariPorts::new(),
        }
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
