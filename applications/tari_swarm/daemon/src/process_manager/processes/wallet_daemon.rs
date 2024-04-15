//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::process_manager::Instance;

pub struct WalletDaemonProcess {
    instance: Instance,
}

impl WalletDaemonProcess {
    pub fn new(instance: Instance) -> Self {
        Self { instance }
    }

    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }

    pub async fn connect_client(&self) -> anyhow::Result<()> {
        unimplemented!()
    }
}
