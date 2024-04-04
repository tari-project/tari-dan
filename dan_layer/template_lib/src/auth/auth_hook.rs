//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_abi::rust::fmt;

use crate::models::{ComponentAddress, TemplateAddress};

#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthHook {
    pub component_address: ComponentAddress,
    pub method: String,
}

impl AuthHook {
    pub fn new(component_address: ComponentAddress, method: String) -> Self {
        Self {
            component_address,
            method,
        }
    }
}

impl fmt::Display for AuthHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.component_address, self.method)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthHookCaller {
    component_address: Option<ComponentAddress>,
    template_address: TemplateAddress,
    component_state: Option<tari_bor::Value>,
}

impl AuthHookCaller {
    pub fn new(template_address: TemplateAddress, component_address: Option<ComponentAddress>) -> Self {
        Self {
            component_address,
            template_address,
            component_state: None,
        }
    }

    pub fn with_component_state(&mut self, component_state: tari_bor::Value) -> &mut Self {
        self.component_state = Some(component_state);
        self
    }

    pub fn component_state(&self) -> Option<&tari_bor::Value> {
        self.component_state.as_ref()
    }

    pub fn component(&self) -> Option<&ComponentAddress> {
        self.component_address.as_ref()
    }

    pub fn template(&self) -> &TemplateAddress {
        &self.template_address
    }
}
