//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_template_abi::rust::collections::BTreeMap;

use crate::models::{NonFungibleAddress, ResourceAddress};

/// Represents the types of possible access control rules over a component method or resource
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

/// An enum that represents the possible ways to restrict access to components or resources
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

/// An enum that allows passing either a resource or a non-fungible argument
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

/// An enum that represents the possible ways to require access to components or resources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequireRule {
    Require(ResourceOrNonFungibleAddress),
    AnyOf(Vec<ResourceOrNonFungibleAddress>),
    AllOf(Vec<ResourceOrNonFungibleAddress>),
}

/// Information needed to specify access rules to methods of a component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentAccessRules {
    method_access: BTreeMap<String, AccessRule>,
    default: AccessRule,
}

impl ComponentAccessRules {
    /// Builds a new set of access rules for a component.
    /// By default, all methods of the component are inaccessible and must be explicitly allowed
    pub fn new() -> Self {
        Self {
            method_access: BTreeMap::new(),
            default: AccessRule::DenyAll,
        }
    }

    /// Builds a new set of access rules for a component, using by default that anyone can call any method on the
    /// component
    pub fn allow_all() -> Self {
        Self {
            method_access: BTreeMap::new(),
            default: AccessRule::AllowAll,
        }
    }

    /// Add a new access rule for a particular method in the component
    pub fn add_method_rule<S: Into<String>>(mut self, name: S, rule: AccessRule) -> Self {
        self.method_access.insert(name.into(), rule);
        self
    }

    /// Set up the default access rule for all methods that do not have a specific rule
    pub fn default(mut self, rule: AccessRule) -> Self {
        self.default = rule;
        self
    }

    /// Return the access rule of a particular method in the component
    pub fn get_method_access_rule(&self, name: &str) -> &AccessRule {
        self.method_access.get(name).unwrap_or(&self.default)
    }

    /// Return an iterator over the access rules of all methods
    pub fn method_access_rules_iter(&self) -> impl Iterator<Item = (&String, &AccessRule)> {
        self.method_access.iter()
    }
}

impl Default for ComponentAccessRules {
    fn default() -> Self {
        Self::new()
    }
}

/// An enum that represents all the possible actions that can be performed on a resource
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

impl ResourceAuthAction {
    pub fn is_recall(&self) -> bool {
        matches!(self, Self::Recall)
    }
}

/// Information needed to specify access rules to a resource
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
    /// Builds a new set of access rules for a resource.
    ///
    /// By default:
    /// * Minting, burning and recalling are disabled for all users
    /// * Withdrawals, deposits and non-fungible data updates are allowed for all users
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

    /// Update the access rules so no one can perform any action on the resource after its creation
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

    /// Sets up who can mint new tokens of the resource
    pub fn mintable(mut self, rule: AccessRule) -> Self {
        self.mintable = rule;
        self
    }

    /// Sets up who can burn (destroy) tokens of the resource
    pub fn burnable(mut self, rule: AccessRule) -> Self {
        self.burnable = rule;
        self
    }

    /// Sets up who can recall tokens of the resource.
    /// A recall is the forceful withdrawal of tokens from any external vault
    pub fn recallable(mut self, rule: AccessRule) -> Self {
        self.recallable = rule;
        self
    }

    /// Sets up who can withdraw tokens of the resource from any vault
    pub fn withdrawable(mut self, rule: AccessRule) -> Self {
        self.withdrawable = rule;
        self
    }

    /// Sets up who can deposit tokens of the resource into any vault
    pub fn depositable(mut self, rule: AccessRule) -> Self {
        self.depositable = rule;
        self
    }

    /// Sets up who can update the mutable data of the tokens in the resource
    pub fn update_non_fungible_data(mut self, rule: AccessRule) -> Self {
        self.update_non_fungible_data = rule;
        self
    }

    /// Returns a reference to the access rule for the specified action
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
