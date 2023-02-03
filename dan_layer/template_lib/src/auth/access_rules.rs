//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};
use tari_template_abi::rust::collections::HashMap;

use crate::{auth::NativeFunctionCall, models::NonFungibleAddress};

#[derive(Debug, Clone, Decode, Encode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AccessRule {
    AllowAll,
    DenyAll,
    Restricted(RestrictedAccessRule),
}

impl AccessRule {
    pub fn is_access_allowed(&self, proofs: &[NonFungibleAddress]) -> bool {
        match self {
            AccessRule::AllowAll => true,
            AccessRule::DenyAll => false,
            AccessRule::Restricted(rule) => rule.is_access_allowed(proofs),
        }
    }
}

#[derive(Debug, Clone, Decode, Encode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RestrictedAccessRule {
    Require(NonFungibleAddress),
    // TODO: Other requirements, for example, holder of a particular resource or particular balance of funds locked
    //       for the transaction
}

impl RestrictedAccessRule {
    pub fn is_access_allowed(&self, proofs: &[NonFungibleAddress]) -> bool {
        match self {
            RestrictedAccessRule::Require(proof) => proofs.contains(proof),
        }
    }
}

#[derive(Debug, Clone, Decode, Encode)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AccessRules {
    method_access: HashMap<String, AccessRule>,
    native_method_access: HashMap<NativeFunctionCall, AccessRule>,
    default: AccessRule,
}

impl AccessRules {
    pub fn new() -> Self {
        Self {
            method_access: HashMap::new(),
            native_method_access: HashMap::new(),
            default: AccessRule::DenyAll,
        }
    }

    // TODO: we use this for all component functions which return Self. Either we need to remove Self support from
    //       template macro (meaning users HAVE to define AccessRules), or we provide
    //       #[template(access_rule = "require(...)")] method annotations, or we decide that AllowAll is a sensible
    //       default for this case.
    pub fn with_default_allow() -> Self {
        Self {
            method_access: HashMap::new(),
            native_method_access: HashMap::new(),
            default: AccessRule::AllowAll,
        }
    }

    pub fn add_method_rule<S: Into<String>>(mut self, name: S, rule: AccessRule) -> Self {
        let name = name.into();
        self.method_access.insert(name, rule);
        self
    }

    pub fn add_native_rule(mut self, call: NativeFunctionCall, rule: AccessRule) -> Self {
        self.native_method_access.insert(call, rule);
        self
    }

    pub fn default(mut self, rule: AccessRule) -> Self {
        self.default = rule;
        self
    }

    pub fn get_method_access_rule(&self, name: &str) -> &AccessRule {
        self.method_access.get(name).unwrap_or(&self.default)
    }

    pub fn get_native_access_rule(&self, call: &NativeFunctionCall) -> &AccessRule {
        self.native_method_access.get(call).unwrap_or(&self.default)
    }

    pub fn method_access_rules_iter(&self) -> impl Iterator<Item = (&String, &AccessRule)> {
        self.method_access.iter()
    }
}

impl Default for AccessRules {
    fn default() -> Self {
        Self::new()
    }
}
