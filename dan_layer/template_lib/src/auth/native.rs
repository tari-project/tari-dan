//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Serialize, Deserialize};
use tari_template_abi::rust::fmt::{Display, Formatter};

use crate::args::{ComponentAction, ResourceAction, VaultAction};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NativeFunctionCall {
    Component(ComponentAction),
    Resource(ResourceAction),
    Vault(VaultAction),
}

impl Display for NativeFunctionCall {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            NativeFunctionCall::Component(action) => write!(f, "component.{:?}", action),
            NativeFunctionCall::Resource(action) => write!(f, "resource.{:?}", action),
            NativeFunctionCall::Vault(action) => write!(f, "vault.{:?}", action),
        }
    }
}
