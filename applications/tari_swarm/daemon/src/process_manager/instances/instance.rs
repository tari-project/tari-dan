//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use tokio::process::Child;

use crate::{config::InstanceType, process_manager::AllocatedPorts};

pub type InstanceId = u32;

pub struct Instance {
    id: InstanceId,
    name: String,
    instance_type: InstanceType,
    child: Child,
    allocated_ports: AllocatedPorts,
    base_path: PathBuf,
}

impl Instance {
    pub fn new(
        id: InstanceId,
        name: String,
        instance_type: InstanceType,
        child: Child,
        allocated_ports: AllocatedPorts,
        base_path: PathBuf,
    ) -> Self {
        Self {
            id,
            name,
            instance_type,
            child,
            allocated_ports,
            base_path,
        }
    }

    pub fn id(&self) -> InstanceId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn instance_type(&self) -> InstanceType {
        self.instance_type
    }

    pub fn child(&self) -> &Child {
        &self.child
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub fn allocated_ports(&self) -> &AllocatedPorts {
        &self.allocated_ports
    }

    pub fn base_path(&self) -> &PathBuf {
        &self.base_path
    }

    pub fn is_running(&mut self) -> bool {
        // try_wait returns none if not exited
        self.child_mut().try_wait().map(|v| v.is_none()).unwrap_or(false)
    }
}
