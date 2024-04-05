//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::config::WebserverConfig;

#[derive(Debug, Clone)]
pub struct HandlerContext {
    config: WebserverConfig,
}

impl HandlerContext {
    pub fn new(config: WebserverConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &WebserverConfig {
        &self.config
    }
}
