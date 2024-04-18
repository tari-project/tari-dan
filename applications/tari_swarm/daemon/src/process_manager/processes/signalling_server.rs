//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::process_manager::Instance;

pub struct SignallingServerProcess {
    instance: Instance,
}

impl SignallingServerProcess {
    pub fn new(instance: Instance) -> Self {
        Self { instance }
    }

    #[allow(dead_code)]
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    #[allow(dead_code)]
    pub fn instance_mut(&mut self) -> &mut Instance {
        &mut self.instance
    }
}
