//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_template_abi::rust::collections::BTreeMap;
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{component, models::{ComponentAddress, NonFungibleAddress, ObjectKey, ResourceAddress, TemplateAddress}};

/// Represents the types of possible access control rules over a component method or resource
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
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

/// Specifies a requirement for a [RequireRule].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum RuleRequirement {
    /// Requires ownership of a specific resource
    Resource(ResourceAddress),
    /// Requires ownership of a specific non-fungible token
    NonFungibleAddress(NonFungibleAddress),
    /// Requires execution within a specific component
    ScopedToComponent(ComponentAddress),
    /// Requires execution within a specific template
    ScopedToTemplate(#[cfg_attr(feature = "ts", ts(type = "Uint8Array"))] TemplateAddress),
}

impl From<ResourceAddress> for RuleRequirement {
    fn from(address: ResourceAddress) -> Self {
        Self::Resource(address)
    }
}

impl From<NonFungibleAddress> for RuleRequirement {
    fn from(address: NonFungibleAddress) -> Self {
        Self::NonFungibleAddress(address)
    }
}

impl From<ComponentAddress> for RuleRequirement {
    fn from(address: ComponentAddress) -> Self {
        Self::ScopedToComponent(address)
    }
}

impl From<TemplateAddress> for RuleRequirement {
    fn from(address: TemplateAddress) -> Self {
        Self::ScopedToTemplate(address)
    }
}

/// An enum that represents the possible ways to require access to components or resources
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum RequireRule {
    Require(RuleRequirement),
    AnyOf(Vec<RuleRequirement>),
    AllOf(Vec<RuleRequirement>),
}

/// Information needed to specify access rules to methods of a component
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ComponentAccessRules {
    #[cfg_attr(feature = "ts", ts(type = "Record<string, AccessRule>"))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
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

#[macro_export]
macro_rules! rule {
    (allow_all) => {
        AccessRule::AllowAll
    };
    (deny_all) => {
        AccessRule::DenyAll
    };

    (resource($x: expr)) => {
        rule! { @access_rule (RuleRequirement::Resource($x)) }
    };
    (non_fungible($x: expr)) => {
        rule! { @access_rule (RuleRequirement::NonFungibleAddress($x)) }
    };
    (component($x: expr)) => {
        rule! { @access_rule (RuleRequirement::ScopedToComponent($x)) }
    };
    (template($x: expr)) => {
        rule! { @access_rule (RuleRequirement::ScopedToTemplate($x)) }
    };

    (@access_rule ($x: expr)) => {
        AccessRule::Restricted(RestrictedAccessRule::Require(RequireRule::Require($x)))
    };
}

#[macro_export]
macro_rules! require_rule {
    (any_of($($tail:tt)*)) => {
        RequireRule::AnyOf(build_vec!($($tail)*))
    };
    (all_of($($tail:tt)*)) => {
        RequireRule::AllOf(build_vec!($($tail)*))
    };
    ($a:ident($b:expr)) => {
        RequireRule::Require(rule_requirement!($a($b)));
    };
}


/// Utility macro for building multiple instruction arguments
#[macro_export]
macro_rules! build_vec {
    () => (Vec::new());

    ($a:ident($b:expr), $($tail:tt)*) => {{
        let mut items = Vec::with_capacity(1 + $crate::__expr_counter!($($tail)*));
        $crate::__build_vec_inner!(@ { items } $a($b), $($tail)*);
        items
    }};

    ($a:ident($b:expr) $(,)?) => {{
        let mut items = Vec::new();
        $crate::__build_vec_inner!(@ { items } $a($b),);
        items
    }};
}

/// Low-level macro for building vecs. Not intended for general
/// usage.
#[macro_export]
macro_rules! __build_vec_inner {
    (@ { $this:ident } $a:ident($e:expr), $($tail:tt)*) => {
        $crate::args::__push(&mut $this, rule_requirement!($a($e)));
        $crate::__build_vec_inner!(@ { $this } $($tail)*);
    };
    (@ { $this:ident } $a:ident($e:expr) $(,)*) => {
        $crate::args::__push(&mut $this, rule_requirement!($a($e)));
    };
    
    (@ { $this:ident } $(,)?) => { };
}

#[macro_export]
macro_rules! rule_requirement {
    (resource($x: expr)) => {
        RuleRequirement::Resource($x)
    };
    (non_fungible($x: expr)) => {
       RuleRequirement::NonFungibleAddress($x)
    };
    (component($x: expr)) => {
        RuleRequirement::ScopedToComponent($x)
    };
    (template($x: expr)) => {
        RuleRequirement::ScopedToTemplate($x)
    };
}

// This is a workaround for a false positive for `clippy::vec_init_then_push` with this macro. We cannot ignore this
// lint as expression attrs are experimental.
#[allow(clippy::inline_always)]
#[inline(always)]
pub fn __push<T>(v: &mut Vec<T>, arg: T) {
    v.push(arg);
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crypto::RistrettoPublicKeyBytes, models::ObjectKey};

    #[test]
    fn build_vec_test() {
        let resource_address = ResourceAddress::new(ObjectKey::default());
        let component_address = ComponentAddress::new(ObjectKey::default());
        let foo = build_vec!(component(component_address), resource(resource_address));
        eprintln!("{:?}", foo);
        let foo = build_vec!(component(component_address));
        eprintln!("{:?}", foo);


        let foo = require_rule!(any_of(component(component_address), resource(resource_address)));
        eprintln!("{:?}", foo);
        let foo = require_rule!(all_of(component(component_address), resource(resource_address)));
        eprintln!("{:?}", foo);
        let foo = require_rule!(component(component_address));
        eprintln!("{:?}", foo);
    }

    #[test]
    fn it_builds_correct_access_rules() {
        // allow all
        let rule = rule!(allow_all);
        assert_eq!(rule, AccessRule::AllowAll);

        // deny all
        let rule = rule!(deny_all);
        assert_eq!(rule, AccessRule::DenyAll);

        // restricted to resource address
        let resource_address = ResourceAddress::new(ObjectKey::default());
        let rule = rule!(resource(resource_address));
        assert_eq!(
            rule,
            access_rule_from_requirement(RuleRequirement::Resource(resource_address))
        );

        // restricted to component
        let component_address = ComponentAddress::new(ObjectKey::default());
        let rule = rule!(component(component_address));
        assert_eq!(
            rule,
            access_rule_from_requirement(RuleRequirement::ScopedToComponent(component_address))
        );

        // restricted to template
        let template_address = TemplateAddress::default();
        let rule = rule!(template(template_address));
        assert_eq!(
            rule,
            access_rule_from_requirement(RuleRequirement::ScopedToTemplate(template_address))
        );

        // restricted to non fungible
        let non_fungible_address = NonFungibleAddress::from_public_key(RistrettoPublicKeyBytes::default());
        let rule = rule!(non_fungible(non_fungible_address.clone()));
        assert_eq!(
            rule,
            access_rule_from_requirement(RuleRequirement::NonFungibleAddress(non_fungible_address))
        );
    }

    fn access_rule_from_requirement(requirement: RuleRequirement) -> AccessRule {
        AccessRule::Restricted(RestrictedAccessRule::Require(RequireRule::Require(requirement)))
    }
}
