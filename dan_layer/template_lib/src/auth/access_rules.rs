//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_template_abi::rust::collections::BTreeMap;

use crate::models::{NonFungibleAddress, ResourceAddress};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccessRule {
    AllowAll,
    DenyAll,
    Restricted(RestrictedAccessRule),
}

impl AccessRule {
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::AllowAll, Self::AllowAll) => Self::AllowAll,
            (Self::DenyAll, _) | (_, Self::DenyAll) => Self::DenyAll,
            (Self::Restricted(rule1), Self::Restricted(rule2)) => Self::Restricted(rule1.and(rule2)),
            (Self::Restricted(rule), Self::AllowAll) | (Self::AllowAll, Self::Restricted(rule)) => {
                Self::Restricted(rule)
            },
        }
    }

    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::AllowAll, _) | (_, Self::AllowAll) => Self::AllowAll,
            (Self::DenyAll, Self::DenyAll) => Self::DenyAll,
            (Self::Restricted(rule1), Self::Restricted(rule2)) => Self::Restricted(rule1.or(rule2)),
            (Self::Restricted(rule), Self::DenyAll) | (Self::DenyAll, Self::Restricted(rule)) => Self::Restricted(rule),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestrictedAccessRule {
    Require(RequireRule),
    AnyOf(Vec<RestrictedAccessRule>),
    AllOf(Vec<RestrictedAccessRule>),
}

impl RestrictedAccessRule {
    pub fn and(self, other: Self) -> Self {
        Self::AllOf(vec![self, other])
    }

    pub fn or(self, other: Self) -> Self {
        Self::AnyOf(vec![self, other])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResourceOrNonFungibleAddress {
    Resource(ResourceAddress),
    NonFungibleAddress(NonFungibleAddress),
}

impl From<ResourceAddress> for ResourceOrNonFungibleAddress {
    fn from(address: ResourceAddress) -> Self {
        Self::Resource(address)
    }
}

impl From<NonFungibleAddress> for ResourceOrNonFungibleAddress {
    fn from(address: NonFungibleAddress) -> Self {
        Self::NonFungibleAddress(address)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequireRule {
    Require(ResourceOrNonFungibleAddress),
    AnyOf(Vec<ResourceOrNonFungibleAddress>),
    AllOf(Vec<ResourceOrNonFungibleAddress>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentAccessRules {
    method_access: BTreeMap<String, AccessRule>,
    default: AccessRule,
}

impl ComponentAccessRules {
    pub fn new() -> Self {
        Self {
            method_access: BTreeMap::new(),
            default: AccessRule::DenyAll,
        }
    }

    pub fn allow_all() -> Self {
        Self {
            method_access: BTreeMap::new(),
            default: AccessRule::AllowAll,
        }
    }

    pub fn add_method_rule<S: Into<String>>(mut self, name: S, rule: AccessRule) -> Self {
        self.method_access.insert(name.into(), rule);
        self
    }

    pub fn default(mut self, rule: AccessRule) -> Self {
        self.default = rule;
        self
    }

    pub fn get_method_access_rule(&self, name: &str) -> &AccessRule {
        self.method_access.get(name).unwrap_or(&self.default)
    }

    pub fn method_access_rules_iter(&self) -> impl Iterator<Item = (&String, &AccessRule)> {
        self.method_access.iter()
    }
}

impl Default for ComponentAccessRules {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum ResourceAuthAction {
    Mint,
    Burn,
    Recall,
    Withdraw,
    Deposit,
    UpdateNonFungibleData,
    UpdateAccessRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAccessRules {
    mintable: AccessRule,
    burnable: AccessRule,
    recallable: AccessRule,
    withdrawable: AccessRule,
    depositable: AccessRule,
    update_non_fungible_data: AccessRule,
}

impl ResourceAccessRules {
    pub fn new() -> Self {
        Self {
            // User should explicitly enable minting and/or burning
            mintable: AccessRule::DenyAll,
            burnable: AccessRule::DenyAll,
            recallable: AccessRule::DenyAll,
            // But explicitly disable withdrawing, updating and/or depositing
            withdrawable: AccessRule::AllowAll,
            depositable: AccessRule::AllowAll,
            update_non_fungible_data: AccessRule::AllowAll,
        }
    }

    pub fn deny_all() -> Self {
        Self {
            mintable: AccessRule::DenyAll,
            burnable: AccessRule::DenyAll,
            recallable: AccessRule::DenyAll,
            withdrawable: AccessRule::DenyAll,
            depositable: AccessRule::DenyAll,
            update_non_fungible_data: AccessRule::DenyAll,
        }
    }

    pub fn mintable(mut self, rule: AccessRule) -> Self {
        self.mintable = rule;
        self
    }

    pub fn burnable(mut self, rule: AccessRule) -> Self {
        self.burnable = rule;
        self
    }

    pub fn recallable(mut self, rule: AccessRule) -> Self {
        self.recallable = rule;
        self
    }

    pub fn withdrawable(mut self, rule: AccessRule) -> Self {
        self.withdrawable = rule;
        self
    }

    pub fn depositable(mut self, rule: AccessRule) -> Self {
        self.depositable = rule;
        self
    }

    pub fn update_non_fungible_data(mut self, rule: AccessRule) -> Self {
        self.update_non_fungible_data = rule;
        self
    }

    pub fn get_access_rule(&self, action: &ResourceAuthAction) -> &AccessRule {
        match action {
            ResourceAuthAction::Mint => &self.mintable,
            ResourceAuthAction::Burn => &self.burnable,
            ResourceAuthAction::Recall => &self.recallable,
            ResourceAuthAction::Withdraw => &self.withdrawable,
            ResourceAuthAction::Deposit => &self.depositable,
            ResourceAuthAction::UpdateNonFungibleData => &self.update_non_fungible_data,
            // Only owner can do this
            ResourceAuthAction::UpdateAccessRules => &AccessRule::DenyAll,
        }
    }
}

impl Default for ResourceAccessRules {
    fn default() -> Self {
        Self::new()
    }
}
