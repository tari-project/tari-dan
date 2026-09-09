//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
//! Access control rules for template-related data like component methods and resources

use minicbor::{CborLen, Decode, Decoder, Encode, data::Type, decode};
use tari_bor::adapters::boxed_slice;
use tari_template_abi::rust::{collections::BTreeMap, prelude::*};

use crate::{ComponentAddress, NonFungibleAddress, ResourceAddress, TemplateAddress, crypto::RistrettoPublicKeyBytes};

/// Represents the types of possible access control rules over a component method or resource
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum AccessRule {
    /// AccessRule always passes
    #[n(0)]
    AllowAll,
    /// AccessRule always fails
    #[n(1)]
    DenyAll,
    /// AccessRule that requires a specific condition to be met
    #[n(2)]
    Restricted(#[n(0)] RestrictedAccessRule),
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

    /// Returns `true` if the rule contains a [`RuleRequirement`] for which `predicate` returns `true`.
    pub fn contains_requirement(&self, predicate: &impl Fn(&RuleRequirement) -> bool) -> bool {
        match self {
            Self::AllowAll | Self::DenyAll => false,
            Self::Restricted(rule) => rule.contains_requirement(predicate),
        }
    }

    /// Returns `true` if the rule contains `ScopedToComponent` or `ScopedToTemplate`, which are constant
    /// on component method rules (they always describe the current frame, i.e. the callee).
    pub fn contains_scoped_to_component_or_template(&self) -> bool {
        self.contains_requirement(&|r| {
            matches!(
                r,
                RuleRequirement::ScopedToComponent(_) | RuleRequirement::ScopedToTemplate(_)
            )
        })
    }
}

/// An enum that represents the possible ways to restrict access to components or resources
#[derive(Debug, Clone, Encode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum RestrictedAccessRule {
    /// Requires a specific condition to be met
    #[n(0)]
    Require(#[n(0)] RequireRule),
    /// Requires any of the specified conditions to be met (logical OR)
    #[n(1)]
    AnyOf(
        #[n(0)]
        #[cbor(with = "boxed_slice")]
        Box<[RestrictedAccessRule]>,
    ),
    /// Requires all of the specified conditions to be met (logical AND)
    #[n(2)]
    AllOf(
        #[n(0)]
        #[cbor(with = "boxed_slice")]
        Box<[RestrictedAccessRule]>,
    ),
}

impl RestrictedAccessRule {
    pub fn and(self, other: Self) -> Self {
        Self::AllOf(Box::new([self, other]))
    }

    pub fn or(self, other: Self) -> Self {
        Self::AnyOf(Box::new([self, other]))
    }

    fn contains_requirement(&self, predicate: &impl Fn(&RuleRequirement) -> bool) -> bool {
        match self {
            Self::Require(rule) => rule.contains_requirement(predicate),
            Self::AnyOf(rules) => rules.iter().any(|rule| rule.contains_requirement(predicate)),
            Self::AllOf(rules) => rules.iter().any(|rule| rule.contains_requirement(predicate)),
        }
    }
}

impl RequireRule {
    fn contains_requirement(&self, predicate: &impl Fn(&RuleRequirement) -> bool) -> bool {
        match self {
            Self::Require(requirement) => predicate(requirement),
            Self::AnyOf(requirements) => requirements.iter().any(predicate),
            Self::AllOf(requirements) => requirements.iter().any(predicate),
            Self::MOfN(_, requirements) => requirements.iter().any(predicate),
        }
    }
}

// `Decode` is hand-written (not derived) because `RestrictedAccessRule` is self-recursive through
// `AnyOf`/`AllOf` and decode input is untrusted: a derived recursive decode would overflow the stack
// — an uncatchable process abort, not a recoverable error — on a maliciously deep payload, before any
// validation runs. Threading the nesting depth bounds the recursion and turns over-nested input into
// a clean error, mirroring the guard `tari_bor` applies to the dynamic `Value` tree. `Encode` and
// `CborLen` stay derived, so this decode must match their wire framing; the round-trip tests enforce
// that.
impl<'b, C> Decode<'b, C> for RestrictedAccessRule {
    fn decode(d: &mut Decoder<'b>, ctx: &mut C) -> Result<Self, decode::Error> {
        decode_restricted_access_rule(d, ctx, 0)
    }
}

fn decode_restricted_access_rule<'b, C>(
    d: &mut Decoder<'b>,
    ctx: &mut C,
    depth: usize,
) -> Result<RestrictedAccessRule, decode::Error> {
    if depth >= tari_bor::MAX_DECODE_DEPTH {
        return Err(decode::Error::message(
            "RestrictedAccessRule nesting exceeds the maximum decode depth",
        ));
    }

    // minicbor's derived enum framing is a definite 2-element array of `[variant index, body]`.
    let pos = d.position();
    if d.array()? != Some(2) {
        return Err(decode::Error::message("expected RestrictedAccessRule enum (2-element array)").at(pos));
    }
    let variant_pos = d.position();
    match d.i64()? {
        0 => decode_variant_field(d, ctx, |d, ctx| RequireRule::decode(d, ctx)).map(RestrictedAccessRule::Require),
        1 => decode_variant_field(d, ctx, |d, ctx| decode_restricted_slice(d, ctx, depth + 1))
            .map(RestrictedAccessRule::AnyOf),
        2 => decode_variant_field(d, ctx, |d, ctx| decode_restricted_slice(d, ctx, depth + 1))
            .map(RestrictedAccessRule::AllOf),
        n => Err(decode::Error::unknown_variant(n).at(variant_pos)),
    }
}

// Decodes one `AnyOf`/`AllOf` boxed slice, recursing at the incremented depth. Reuses the
// `boxed_slice` adapter's element loop (and its `MAX_PREALLOC` cap) via a depth-threading decoder.
fn decode_restricted_slice<'b, C>(
    d: &mut Decoder<'b>,
    ctx: &mut C,
    depth: usize,
) -> Result<Box<[RestrictedAccessRule]>, decode::Error> {
    boxed_slice::decode_with_fn(d, ctx, |d, ctx| decode_restricted_access_rule(d, ctx, depth))
}

// Every `RestrictedAccessRule` variant carries a single field at index 0. minicbor's derive encodes a
// variant body as an array and reads its fields by position; this mirrors that — decode position 0,
// skip any further positions — so it accepts exactly what the derived decode would.
fn decode_variant_field<'b, C, T>(
    d: &mut Decoder<'b>,
    ctx: &mut C,
    decode_field: impl FnOnce(&mut Decoder<'b>, &mut C) -> Result<T, decode::Error>,
) -> Result<T, decode::Error> {
    let pos = d.position();
    match d.array()? {
        Some(0) => Err(decode::Error::missing_value(0).at(pos)),
        Some(len) => {
            let field = decode_field(d, ctx)?;
            for _ in 1..len {
                d.skip()?;
            }
            Ok(field)
        },
        None => {
            if matches!(d.datatype()?, Type::Break) {
                return Err(decode::Error::missing_value(0).at(pos));
            }
            let field = decode_field(d, ctx)?;
            while !matches!(d.datatype()?, Type::Break) {
                d.skip()?;
            }
            d.skip()?;
            Ok(field)
        },
    }
}

/// Specifies a requirement for a [RequireRule].
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum RuleRequirement {
    /// Requires a proof of a specific resource
    #[n(0)]
    Resource(#[n(0)] ResourceAddress),
    /// Requires a proof of a specific non-fungible token
    #[n(1)]
    NonFungibleAddress(#[n(0)] NonFungibleAddress),
    /// Requires execution within a specific component (the current frame is that component)
    #[n(2)]
    ScopedToComponent(#[n(0)] ComponentAddress),
    /// Requires execution within a specific template (the current frame is that template)
    #[n(3)]
    ScopedToTemplate(#[n(0)] TemplateAddress),
    /// Requires the badge naming a specific component as the caller. Shorthand for
    /// `NonFungibleAddress(NonFungibleAddress::caller_component_badge(address))`.
    #[n(4)]
    CallerComponent(#[n(0)] ComponentAddress),
    /// Requires the badge naming a specific template as the immediate caller's: any component instance of it, or any
    /// static function of it. Shorthand for
    /// `NonFungibleAddress(NonFungibleAddress::direct_caller_template_badge(address))`.
    #[n(5)]
    DirectCallerTemplate(#[n(0)] TemplateAddress),
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

impl From<RistrettoPublicKeyBytes> for RuleRequirement {
    fn from(public_key: RistrettoPublicKeyBytes) -> Self {
        Self::NonFungibleAddress(NonFungibleAddress::from_public_key(public_key))
    }
}

/// A rule requiring specific condition(s) to be met
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum RequireRule {
    /// Requires a specific condition to be met
    #[n(0)]
    Require(#[n(0)] RuleRequirement),
    /// Requires any of the specified conditions to be met (logical OR)
    #[n(1)]
    AnyOf(
        #[n(0)]
        #[cbor(with = "boxed_slice")]
        Box<[RuleRequirement]>,
    ),
    /// Requires all of the specified conditions to be met (logical AND)
    #[n(2)]
    AllOf(
        #[n(0)]
        #[cbor(with = "boxed_slice")]
        Box<[RuleRequirement]>,
    ),
    /// Requires N of the specified conditions to be met
    #[n(3)]
    MOfN(
        #[n(0)] u16,
        #[n(1)]
        #[cbor(with = "boxed_slice")]
        Box<[RuleRequirement]>,
    ),
}

/// Information needed to specify access rules to methods of a component
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
pub struct ComponentAccessRules {
    #[cfg_attr(feature = "ts", ts(type = "Record<string, AccessRule>"))]
    #[n(0)]
    method_access: BTreeMap<String, AccessRule>,
    #[n(1)]
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
        assert!(
            !rule.contains_scoped_to_component_or_template(),
            "`component(..)`/`template(..)` are constant on component method rules; use \
             `caller_component(..)`/`direct_caller_template(..)` to gate on the caller"
        );
        self.method_access.insert(name.into(), rule);
        self
    }

    /// Add a new access rule for a particular method in the component
    pub fn method<S: Into<String>>(self, name: S, rule: AccessRule) -> Self {
        self.add_method_rule(name, rule)
    }

    /// Returns the number of custom access rules
    pub fn num_access_rules(&self) -> usize {
        self.method_access.len()
    }

    /// Set up the default access rule for all methods that do not have a specific rule
    pub fn default(mut self, rule: AccessRule) -> Self {
        assert!(
            !rule.contains_scoped_to_component_or_template(),
            "`component(..)`/`template(..)` are constant on component method rules; use \
             `caller_component(..)`/`direct_caller_template(..)` to gate on the caller"
        );
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

    /// Returns `true` if the default rule or any method rule contains `ScopedToComponent` or
    /// `ScopedToTemplate`, which are constant on component method rules (they always describe the current
    /// frame, i.e. the callee).
    pub fn contains_scoped_to_component_or_template(&self) -> bool {
        self.default.contains_scoped_to_component_or_template() ||
            self.method_access
                .values()
                .any(AccessRule::contains_scoped_to_component_or_template)
    }
}

impl Default for ComponentAccessRules {
    fn default() -> Self {
        Self::new()
    }
}

/// An enum that represents all the possible actions that can be performed on a resource
#[derive(Debug, Clone, Copy, Encode, Decode, CborLen, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ResourceAuthAction {
    #[n(0)]
    Mint,
    #[n(1)]
    Burn,
    #[n(2)]
    Recall,
    #[n(3)]
    Withdraw,
    #[n(4)]
    Deposit,
    #[n(5)]
    UpdateNonFungibleData,
    #[n(6)]
    Freeze,
    #[n(7)]
    UpdateMetadata,
}

impl ResourceAuthAction {
    pub fn is_recall(&self) -> bool {
        matches!(self, Self::Recall)
    }
}

#[derive(Debug, Clone, Default, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum UpdateRule {
    #[n(0)]
    #[default]
    Locked,
    #[n(1)]
    Owner,
    #[n(2)]
    AccessRule(#[n(0)] AccessRule),
}

impl From<AccessRule> for UpdateRule {
    fn from(rule: AccessRule) -> Self {
        Self::AccessRule(rule)
    }
}

pub const LOCKED: UpdateRule = UpdateRule::Locked;
pub const OWNER: UpdateRule = UpdateRule::Owner;

/// Information needed to specify access rules to a resource
#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ResourceAccessRules {
    #[n(0)]
    mint: AccessRule,
    #[n(1)]
    mint_updater: UpdateRule,
    #[n(2)]
    burn: AccessRule,
    #[n(3)]
    burn_updater: UpdateRule,
    #[n(4)]
    recall: AccessRule,
    #[n(5)]
    recall_updater: UpdateRule,
    #[n(6)]
    withdraw: AccessRule,
    #[n(7)]
    withdraw_updater: UpdateRule,
    #[n(8)]
    deposit: AccessRule,
    #[n(9)]
    deposit_updater: UpdateRule,
    #[n(10)]
    update_nft_data: AccessRule,
    #[n(11)]
    nft_data_updater: UpdateRule,
    #[n(12)]
    freeze: AccessRule,
    #[n(13)]
    freeze_updater: UpdateRule,
    #[n(14)]
    update_metadata: AccessRule,
    #[n(15)]
    metadata_updater: UpdateRule,
    /// Who may install, replace or remove the resource's [`AuthHook`](crate::AuthHook). The hook itself is not
    /// an [`AccessRule`], so this updater stands alone rather than pairing with one.
    ///
    /// A hook runs on nearly every resource action, so one that panics or denies unconditionally takes the
    /// resource offline and strands the balances in its vaults. `Locked` — the default — keeps a hook binding
    /// for the life of the resource; anything else lets the hook be repaired or retired, at the cost of letting
    /// whoever satisfies the updater change the rules that existing holders are relying on — up to and
    /// including giving a hook-free resource a hook.
    #[n(16)]
    #[cbor(default)]
    #[cfg_attr(feature = "serde", serde(default))]
    auth_hook_updater: UpdateRule,
}

impl ResourceAccessRules {
    /// Builds a new set of access rules for a resource.
    ///
    /// By default:
    /// * Updating the access rules is disabled for all users (i.e. only the OwnerRule applies)
    /// * Minting, burning, recalling and freezing are disabled for all users
    /// * Withdrawals, deposits and non-fungible data updates are allowed for all users
    pub const fn new() -> Self {
        Self {
            // User should explicitly enable minting, burning etc
            mint: AccessRule::DenyAll,
            mint_updater: UpdateRule::Locked,
            burn: AccessRule::DenyAll,
            burn_updater: UpdateRule::Locked,
            recall: AccessRule::DenyAll,
            recall_updater: UpdateRule::Locked,
            freeze: AccessRule::DenyAll,
            freeze_updater: UpdateRule::Locked,
            update_metadata: AccessRule::DenyAll,
            metadata_updater: UpdateRule::Owner,
            // But explicitly disable withdrawing, updating and/or depositing
            withdraw: AccessRule::AllowAll,
            withdraw_updater: UpdateRule::Locked,
            deposit: AccessRule::AllowAll,
            deposit_updater: UpdateRule::Locked,
            update_nft_data: AccessRule::AllowAll,
            nft_data_updater: UpdateRule::Owner,
            auth_hook_updater: UpdateRule::Locked,
        }
    }

    /// Update the access rules so no one can perform any action on the resource after its creation
    pub fn deny_all() -> Self {
        Self {
            mint: AccessRule::DenyAll,
            mint_updater: UpdateRule::Locked,
            burn: AccessRule::DenyAll,
            burn_updater: UpdateRule::Locked,
            recall: AccessRule::DenyAll,
            recall_updater: UpdateRule::Locked,
            withdraw: AccessRule::DenyAll,
            withdraw_updater: UpdateRule::Locked,
            deposit: AccessRule::DenyAll,
            deposit_updater: UpdateRule::Locked,
            update_nft_data: AccessRule::DenyAll,
            nft_data_updater: UpdateRule::Locked,
            freeze: AccessRule::DenyAll,
            freeze_updater: UpdateRule::Locked,
            update_metadata: AccessRule::DenyAll,
            metadata_updater: UpdateRule::Locked,
            auth_hook_updater: UpdateRule::Locked,
        }
    }

    /// Sets up who can mint new tokens of the resource
    pub fn mintable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.mint = rule;
        self.mint_updater = updater.into();
        self
    }

    /// Sets up who can burn (destroy) tokens of the resource
    pub fn burnable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.burn = rule;
        self.burn_updater = updater.into();
        self
    }

    /// Sets up who can recall tokens of the resource.
    /// A recall is the forceful withdrawal of tokens from any external vault
    pub fn recallable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.recall = rule;
        self.recall_updater = updater.into();
        self
    }

    /// Sets up who can freeze a vault (or UTXO in the case of stealth) containing this resource, preventing
    /// withdrawals.
    pub fn freezable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.freeze = rule;
        self.freeze_updater = updater.into();
        self
    }

    /// Sets up who can withdraw tokens of the resource from any vault
    pub fn withdrawable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.withdraw = rule;
        self.withdraw_updater = updater.into();
        self
    }

    /// Sets up who can deposit tokens of the resource into any vault
    pub fn depositable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.deposit = rule;
        self.deposit_updater = updater.into();
        self
    }

    /// Sets up who can update the mutable data of the tokens in the resource
    pub fn update_non_fungible_data<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.update_nft_data = rule;
        self.nft_data_updater = updater.into();
        self
    }

    /// Sets up who can update the resource's metadata. The token symbol remains immutable once set.
    pub fn update_metadata<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.update_metadata = rule;
        self.metadata_updater = updater.into();
        self
    }

    /// Sets up who can install, replace or remove the resource's authorization hook. Locked by default, which
    /// makes a hook binding for the life of the resource.
    pub fn set_auth_hook_updater<U: Into<UpdateRule>>(mut self, updater: U) -> Self {
        self.auth_hook_updater = updater.into();
        self
    }

    /// Returns the updater rule that governs who may replace or remove the resource's authorization hook.
    pub fn auth_hook_updater(&self) -> &UpdateRule {
        &self.auth_hook_updater
    }

    /// Returns a reference to the access rule for the specified action
    pub fn get_access_rule(&self, action: &ResourceAuthAction) -> &AccessRule {
        match action {
            ResourceAuthAction::Mint => &self.mint,
            ResourceAuthAction::Burn => &self.burn,
            ResourceAuthAction::Recall => &self.recall,
            ResourceAuthAction::Withdraw => &self.withdraw,
            ResourceAuthAction::Deposit => &self.deposit,
            ResourceAuthAction::UpdateNonFungibleData => &self.update_nft_data,
            ResourceAuthAction::UpdateMetadata => &self.update_metadata,
            ResourceAuthAction::Freeze => &self.freeze,
        }
    }

    /// Returns a reference to the updater rule that governs who may change the access rule for the
    /// specified action.
    pub fn get_updater(&self, action: &ResourceAuthAction) -> &UpdateRule {
        match action {
            ResourceAuthAction::Mint => &self.mint_updater,
            ResourceAuthAction::Burn => &self.burn_updater,
            ResourceAuthAction::Recall => &self.recall_updater,
            ResourceAuthAction::Withdraw => &self.withdraw_updater,
            ResourceAuthAction::Deposit => &self.deposit_updater,
            ResourceAuthAction::UpdateNonFungibleData => &self.nft_data_updater,
            ResourceAuthAction::UpdateMetadata => &self.metadata_updater,
            ResourceAuthAction::Freeze => &self.freeze_updater,
        }
    }

    /// Replaces the access rule for the specified action without changing its updater rule.
    /// The caller is responsible for verifying that the change is authorized.
    pub fn set_access_rule(&mut self, action: ResourceAuthAction, rule: AccessRule) {
        match action {
            ResourceAuthAction::Mint => self.mint = rule,
            ResourceAuthAction::Burn => self.burn = rule,
            ResourceAuthAction::Recall => self.recall = rule,
            ResourceAuthAction::Withdraw => self.withdraw = rule,
            ResourceAuthAction::Deposit => self.deposit = rule,
            ResourceAuthAction::UpdateNonFungibleData => self.update_nft_data = rule,
            ResourceAuthAction::UpdateMetadata => self.update_metadata = rule,
            ResourceAuthAction::Freeze => self.freeze = rule,
        }
    }

    /// Writes the rules as a protocol version 0 substate hash preimage: every field but `auth_hook_updater`,
    /// which version 0 resources do not carry.
    #[cfg(feature = "borsh")]
    #[doc(hidden)]
    pub fn borsh_serialize_v0<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        // Destructured so that a field added to the struct fails to compile here rather than being silently
        // dropped from the version 0 preimage.
        let Self {
            mint,
            mint_updater,
            burn,
            burn_updater,
            recall,
            recall_updater,
            withdraw,
            withdraw_updater,
            deposit,
            deposit_updater,
            update_nft_data,
            nft_data_updater,
            freeze,
            freeze_updater,
            update_metadata,
            metadata_updater,
            auth_hook_updater: _,
        } = self;

        borsh::BorshSerialize::serialize(mint, writer)?;
        borsh::BorshSerialize::serialize(mint_updater, writer)?;
        borsh::BorshSerialize::serialize(burn, writer)?;
        borsh::BorshSerialize::serialize(burn_updater, writer)?;
        borsh::BorshSerialize::serialize(recall, writer)?;
        borsh::BorshSerialize::serialize(recall_updater, writer)?;
        borsh::BorshSerialize::serialize(withdraw, writer)?;
        borsh::BorshSerialize::serialize(withdraw_updater, writer)?;
        borsh::BorshSerialize::serialize(deposit, writer)?;
        borsh::BorshSerialize::serialize(deposit_updater, writer)?;
        borsh::BorshSerialize::serialize(update_nft_data, writer)?;
        borsh::BorshSerialize::serialize(nft_data_updater, writer)?;
        borsh::BorshSerialize::serialize(freeze, writer)?;
        borsh::BorshSerialize::serialize(freeze_updater, writer)?;
        borsh::BorshSerialize::serialize(update_metadata, writer)?;
        borsh::BorshSerialize::serialize(metadata_updater, writer)
    }
}

impl Default for ResourceAccessRules {
    fn default() -> Self {
        Self::new()
    }
}

/// A macro to build access rules for components and resources.
///
/// It allows for defining rules such as `allow_all`, `deny_all`, and more complex rules using `any_of`, `all_of` and
/// `n_of` constructs.
///
/// `component(addr)` / `template(addr)` require execution within a component/template, while
/// `caller_component(addr)` / `direct_caller_template(addr)` require the badge of that component/template, which the
/// engine stamps into a frame's authorization scope naming its immediate caller. "Direct" means the immediate caller
/// only: if A calls B and B calls C, C's frame carries B's badge, not A's. Because they are badges they hold for the
/// lifetime of the frame and are checkable wherever a proof requirement is — component method rules, owner rules,
/// resource rules and spend conditions alike. They are re-derived at every frame push and are not capturable as a
/// `Proof`, so a callee cannot forward the identity it was called with.
///
/// **Caution:** `caller_component` / `direct_caller_template` match the immediate caller's identity, which is only as
/// trustworthy as the code that makes the call. A method that forwards a caller-supplied component and method (a
/// "proxy" method) delegates that identity, so anyone who can call the proxy can act as the proxied component/template.
/// `direct_caller_template` is a package identity: it matches any instance of the template (including ones anyone can
/// create) and any static function of it (which anyone can call), so it is only as strong as the least careful outgoing
/// call anywhere in that template.
///
/// **Caution, the other way round:** calling out hands the callee your identity as a live badge for the whole of its
/// frame, usable at every auth point it reaches — a resource rule, an ownership rule, a spend condition — not only at
/// the method it entered through. Weigh that before calling into code you do not control, and gate the rules that
/// matter on a proof the callee cannot obtain rather than on the caller badge alone.
///
/// `component(addr)` / `template(addr)` are constant on component **method** rules and owner rules (they always
/// describe the current frame, i.e. the component itself). The builder methods reject them at construction, and the
/// engine rejects them on component creation and `ComponentAction::SetAccessRules`.
///
/// # Examples:
///
/// ```rust
/// use tari_template_lib_types::rule;
/// // Allow all access
/// let allow_all_rule = rule!(allow_all);
/// // Deny all access
/// let deny_all_rule = rule!(deny_all);
/// // Restricted access to a specific resource
/// let resource_address = tari_template_lib_types::ResourceAddress::new(
///     tari_template_lib_types::ObjectKey::default(),
/// );
/// let resource_rule = rule!(resource(resource_address));
/// // Restricted access to a component
/// let component_address = tari_template_lib_types::ComponentAddress::new(
///     tari_template_lib_types::ObjectKey::default(),
/// );
/// let component_rule = rule!(component(component_address));
/// // Restricted access to a template
/// let template_address = tari_template_lib_types::TemplateAddress::default();
/// let template_rule = rule!(template(template_address));
/// // Restricted access to calls from a specific component
/// let caller_component_rule = rule!(caller_component(component_address));
/// // Restricted access to calls from a specific template
/// let caller_template_rule = rule!(direct_caller_template(template_address));
/// // Restricted access to a non-fungible token
/// let non_fungible_address = tari_template_lib_types::NonFungibleAddress::from_public_key(
///     tari_template_lib_types::crypto::RistrettoPublicKeyBytes::default(),
/// );
/// let non_fungible_rule = rule!(non_fungible(non_fungible_address));
/// // Complex rules using `any_of`, `all_of` and `n_of`
/// let complex_rule = rule!(any_of(
///     component(component_address),
///     resource(resource_address)
/// ));
/// # let pk1 = tari_template_lib_types::crypto::RistrettoPublicKeyBytes::default();
/// # let pk2 = tari_template_lib_types::crypto::RistrettoPublicKeyBytes::default();
/// let n_of_rule = rule!(m_of_n(2, public_key(pk1), public_key(pk2)));
/// ```
#[macro_export]
macro_rules! rule {
    (allow_all) => {
        $crate::access_rules::AccessRule::AllowAll
    };
    (deny_all) => {
        $crate::access_rules::AccessRule::DenyAll
    };
    ($($tail:tt)*) => {
        $crate::access_rules::AccessRule::Restricted($crate::__restricted_access_rule!($($tail)*))
    };
}

#[macro_export]
macro_rules! __restricted_access_rule {
    (any_of($($tail:tt)*)) => {
        $crate::access_rules::RestrictedAccessRule::AnyOf($crate::__build_vec!(@ {__restricted_access_rule} $($tail)*).into_boxed_slice())
    };
    (all_of($($tail:tt)*)) => {
        $crate::access_rules::RestrictedAccessRule::AllOf($crate::__build_vec!(@ {__restricted_access_rule} $($tail)*).into_boxed_slice())
    };
    ($a:ident($($tail:tt)*)) => {
        $crate::access_rules::RestrictedAccessRule::Require($crate::__require_rule!($a($($tail)*)))
    };
}

#[macro_export]
macro_rules! __require_rule {
    (any_of($($tail:tt)*)) => {
        $crate::access_rules::RequireRule::AnyOf($crate::__build_vec!(@ {__rule_requirement} $($tail)*).into_boxed_slice())
    };
    (all_of($($tail:tt)*)) => {
        $crate::access_rules::RequireRule::AllOf($crate::__build_vec!(@ {__rule_requirement} $($tail)*).into_boxed_slice())
    };
    (m_of_n($n:literal, $($tail:tt)*)) => {
        $crate::access_rules::RequireRule::MOfN($n, $crate::__build_vec!(@ {__rule_requirement} $($tail)*).into_boxed_slice())
    };
    ($a:ident($b:expr)) => {
        $crate::access_rules::RequireRule::Require($crate::__rule_requirement!($a($b)))
    };
}

#[macro_export]
macro_rules! __rule_requirement {
    (resource($x: expr)) => {
        $crate::access_rules::RuleRequirement::Resource($x)
    };
    (non_fungible($x: expr)) => {
        $crate::access_rules::RuleRequirement::NonFungibleAddress($x.into())
    };
    (public_key($x: expr)) => {
        $crate::access_rules::RuleRequirement::NonFungibleAddress($crate::NonFungibleAddress::from_public_key($x))
    };
    (component($x: expr)) => {
        $crate::access_rules::RuleRequirement::ScopedToComponent($x)
    };
    (template($x: expr)) => {
        $crate::access_rules::RuleRequirement::ScopedToTemplate($x)
    };
    (caller_component($x: expr)) => {
        $crate::access_rules::RuleRequirement::CallerComponent($x)
    };
    (direct_caller_template($x: expr)) => {
        $crate::access_rules::RuleRequirement::DirectCallerTemplate($x)
    };
}

#[macro_export]
macro_rules! __build_vec {
    () => (Vec::new());

    (@ {$item_fn:ident} $a:ident($b:expr), $($tail:tt)*) => {{
        let mut items = Vec::with_capacity(1 + $crate::__expr_counter!($($tail)*));
        $crate::__build_vec_inner!(@ { items, $item_fn } $a($b), $($tail)*);
        items
    }};

    (@ {$item_fn:ident} $a:ident($b:expr) $(,)?) => {{
        let mut items = Vec::new();
        $crate::__build_vec_inner!(@ { items, $item_fn } $a($b),);
        items
    }};
}

#[macro_export]
macro_rules! __build_vec_inner {
    (@ { $this:ident, $item_fn:ident } $a:ident($e:expr), $($tail:tt)*) => {
        $crate::access_rules::__push(&mut $this, $crate::$item_fn!($a($e)));
        $crate::__build_vec_inner!(@ {$this, $item_fn } $($tail)*);
    };
    (@ { $this:ident, $item_fn:ident } $a:ident($e:expr) $(,)*) => {
        $crate::access_rules::__push(&mut $this, $crate::$item_fn!($a($e)));
    };
}

/// Low-level macro used for counting characters in the encoding of arguments. Not intended for general usage
#[macro_export]
macro_rules! __expr_counter {
    () => (0usize);
    ( $x:expr $(,)? ) => (1usize);
    ( $x:expr, $($next:tt)* ) => (1usize + $crate::__expr_counter!($($next)*));
}

// This is a workaround for a false positive for `clippy::vec_init_then_push` with this macro. We cannot ignore this
// lint as expression attrs are experimental.
#[allow(clippy::inline_always)]
#[inline(always)]
#[doc(hidden)]
pub fn __push<T>(v: &mut Vec<T>, arg: T) {
    v.push(arg);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObjectKey, crypto::RistrettoPublicKeyBytes};

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

        // composition of rules
        let rule = rule!(any_of(component(component_address), resource(resource_address)));
        assert_eq!(
            rule,
            AccessRule::Restricted(RestrictedAccessRule::AnyOf(Box::new([
                RestrictedAccessRule::Require(RequireRule::Require(RuleRequirement::ScopedToComponent(
                    component_address
                ))),
                RestrictedAccessRule::Require(RequireRule::Require(RuleRequirement::Resource(resource_address))),
            ])))
        );

        let rule = rule!(all_of(component(component_address), resource(resource_address)));
        assert_eq!(
            rule,
            AccessRule::Restricted(RestrictedAccessRule::AllOf(Box::new([
                RestrictedAccessRule::Require(RequireRule::Require(RuleRequirement::ScopedToComponent(
                    component_address
                ))),
                RestrictedAccessRule::Require(RequireRule::Require(RuleRequirement::Resource(resource_address))),
            ])))
        );

        let rule = rule!(m_of_n(1, component(component_address), resource(resource_address)));
        assert_eq!(
            rule,
            AccessRule::Restricted(RestrictedAccessRule::Require(RequireRule::MOfN(
                1,
                Box::new([
                    RuleRequirement::ScopedToComponent(component_address),
                    RuleRequirement::Resource(resource_address),
                ])
            )))
        );
    }

    fn access_rule_from_requirement(requirement: RuleRequirement) -> AccessRule {
        AccessRule::Restricted(RestrictedAccessRule::Require(RequireRule::Require(requirement)))
    }

    fn leaf_rule() -> RestrictedAccessRule {
        RestrictedAccessRule::Require(RequireRule::Require(RuleRequirement::Resource(ResourceAddress::new(
            ObjectKey::default(),
        ))))
    }

    fn nested_any_of(depth: usize) -> RestrictedAccessRule {
        let mut rule = leaf_rule();
        for _ in 0..depth {
            rule = RestrictedAccessRule::AnyOf(Box::new([rule]));
        }
        rule
    }

    #[test]
    fn restricted_access_rule_roundtrips_all_variants() {
        let component_address = ComponentAddress::new(ObjectKey::default());
        let resource_address = ResourceAddress::new(ObjectKey::default());
        let rules = [
            leaf_rule(),
            RestrictedAccessRule::Require(RequireRule::MOfN(
                1,
                Box::new([
                    RuleRequirement::ScopedToComponent(component_address),
                    RuleRequirement::Resource(resource_address),
                ]),
            )),
            RestrictedAccessRule::AnyOf(Box::new([
                RestrictedAccessRule::Require(RequireRule::Require(RuleRequirement::ScopedToComponent(
                    component_address,
                ))),
                RestrictedAccessRule::AllOf(Box::new([leaf_rule()])),
            ])),
            nested_any_of(8),
        ];
        for rule in rules {
            let bytes = tari_bor::encode(&rule).unwrap();
            let decoded: RestrictedAccessRule = tari_bor::decode(&bytes).unwrap();
            assert_eq!(decoded, rule);
        }
    }

    #[test]
    fn restricted_access_rule_decodes_up_to_max_depth() {
        // One level below the limit decodes and round-trips; at the limit it is rejected.
        let ok = nested_any_of(tari_bor::MAX_DECODE_DEPTH - 1);
        let bytes = tari_bor::encode(&ok).unwrap();
        assert_eq!(tari_bor::decode::<RestrictedAccessRule>(&bytes).unwrap(), ok);

        let too_deep = nested_any_of(tari_bor::MAX_DECODE_DEPTH);
        let bytes = tari_bor::encode(&too_deep).unwrap();
        assert!(tari_bor::decode::<RestrictedAccessRule>(&bytes).is_err());
    }

    #[test]
    fn restricted_access_rule_rejects_deeply_nested_payload_without_overflow() {
        // A tiny-per-level payload that, decoded by a recursive decode without a depth bound, would
        // overflow the stack (a process abort). Each `AnyOf` level is the 4 bytes `82 01 81 81`. The
        // depth guard must reject it as a clean error long before the recursion can overflow.
        let bytes: Vec<u8> = (0..100_000).flat_map(|_| [0x82u8, 0x01, 0x81, 0x81]).collect();
        assert!(tari_bor::decode::<RestrictedAccessRule>(&bytes).is_err());
    }

    #[test]
    #[should_panic(expected = "constant on component method rules")]
    fn method_rule_rejects_scoped_component() {
        let address = ComponentAddress::new(ObjectKey::default());
        ComponentAccessRules::new().method("foo", rule!(component(address)));
    }

    #[test]
    #[should_panic(expected = "constant on component method rules")]
    fn method_rule_rejects_scoped_template() {
        let address = TemplateAddress::default();
        ComponentAccessRules::new().default(rule!(template(address)));
    }

    #[test]
    fn resource_rule_allows_caller_requirements() {
        let component = ComponentAddress::new(ObjectKey::default());
        let template = TemplateAddress::default();
        ResourceAccessRules::new()
            .mintable(rule!(caller_component(component)), LOCKED)
            .withdrawable(rule!(direct_caller_template(template)), LOCKED);
    }

    #[test]
    fn method_rule_allows_caller_requirements() {
        let address = ComponentAddress::new(ObjectKey::default());
        ComponentAccessRules::new().method("foo", rule!(caller_component(address)));
    }

    #[test]
    fn resource_rule_allows_scoped_requirements() {
        let address = ComponentAddress::new(ObjectKey::default());
        ResourceAccessRules::new().mintable(rule!(component(address)), LOCKED);
    }

    #[test]
    fn requirement_detection_is_recursive() {
        let component = ComponentAddress::new(ObjectKey::default());
        let rule = rule!(any_of(
            resource(ResourceAddress::new(ObjectKey::default())),
            caller_component(component)
        ));
        assert!(!rule.contains_scoped_to_component_or_template());
        assert!(
            rule!(any_of(
                resource(ResourceAddress::new(ObjectKey::default())),
                component(component)
            ))
            .contains_scoped_to_component_or_template()
        );
    }

    #[test]
    fn caller_badges_are_namespaced_by_their_own_resource() {
        let component = ComponentAddress::new(ObjectKey::default());
        let template = TemplateAddress::default();

        let component_badge = NonFungibleAddress::caller_component_badge(component);
        let template_badge = NonFungibleAddress::direct_caller_template_badge(template);

        assert_eq!(
            *component_badge.resource_address(),
            crate::constants::CALLER_COMPONENT_RESOURCE_ADDRESS
        );
        assert_eq!(
            *template_badge.resource_address(),
            crate::constants::DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS
        );
        // A component address and a template address are both 32 bytes, so the two badge resources must
        // keep them apart: an all-zero component must never match an all-zero template.
        assert_ne!(component_badge, template_badge);
        assert!(component_badge.resource_address().is_caller_badge());
        assert!(template_badge.resource_address().is_caller_badge());
        assert!(component_badge.resource_address().is_system_reserved());
    }

    #[test]
    fn auth_hook_updater_defaults_to_locked() {
        assert!(matches!(
            ResourceAccessRules::new().auth_hook_updater(),
            UpdateRule::Locked
        ));
        assert!(matches!(
            ResourceAccessRules::deny_all().auth_hook_updater(),
            UpdateRule::Locked
        ));
    }

    /// A caller requirement is a badge on a resource rule, so an updater carrying one is satisfiable rather than
    /// `Locked` wearing a disguise: the component it names can repair the hook, which is what this updater exists
    /// to guarantee.
    #[test]
    fn auth_hook_updater_accepts_a_caller_requirement() {
        let address = ComponentAddress::new(ObjectKey::default());
        let rules = ResourceAccessRules::new().set_auth_hook_updater(rule!(caller_component(address)));
        assert!(matches!(rules.auth_hook_updater(), UpdateRule::AccessRule(_)));
    }

    #[test]
    fn component_access_rules_detects_scoped_requirement() {
        let address = ComponentAddress::new(ObjectKey::default());

        // A valid builder-constructed rule set has no scoped requirement.
        assert!(
            !ComponentAccessRules::new()
                .method("foo", rule!(caller_component(address)))
                .contains_scoped_to_component_or_template()
        );

        // A decoded (hand-written) rule set that bypasses the builder is still detected.
        let mut degenerate = ComponentAccessRules::new();
        degenerate
            .method_access
            .insert("foo".to_string(), rule!(component(address)));
        assert!(degenerate.contains_scoped_to_component_or_template());
    }
}
