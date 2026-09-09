//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use tari_template_abi::rust::prelude::*;
use tari_template_lib_types::{
    AuthHook,
    ComponentAddress,
    FunctionName,
    Metadata,
    OwnerRule,
    ResourceAddress,
    ResourceType,
    access_rules::{AccessRule, ResourceAccessRules, UpdateRule},
    constants::{DEFAULT_DIVISIBILITY, IMAGE_URL, TOKEN_SYMBOL},
};

use crate::{
    args::MintArg,
    error_variants::ERR_AUTH_HOOK_FN_NAME_LEN,
    models::{Bucket, ResourceAddressAllocation},
    resource::ResourceManager,
    types::Amount,
};

/// A builder for creating fungible resources (tokens) inside templates.
///
/// This builder provides a fluent API to configure and create fungible tokens with
/// various properties such as ownership rules, access controls, metadata, divisibility,
/// and authorization hooks.
///
/// If values are not set, defaults for the various properties will be used.
///
/// # Usage
///
/// You typically start by creating a new builder via
/// [`ResourceBuilder::public_fungible()`](super::ResourceBuilder::public_fungible), then chain configuration methods
/// like `.with_owner_rule()`, `.mintable()`, or `.with_token_symbol()`, and finally call `.build()` or
/// `.initial_supply()` to create the resource.
///
/// # Examples
///
/// Basic usage:
/// ```ignore
/// use tari_template_lib::resource::builder::ResourceBuilder;
///
/// let resource_address = ResourceBuilder::public_fungible()
///     .with_token_symbol("TARI")
///     .with_divisibility(9)
///     .mintable(rule!(allow_all))
///     .build();
/// ```
///
/// Creating a resource with an initial supply:
/// ```ignore
/// use tari_template_lib::resource::builder::ResourceBuilder;
/// use tari_template_lib::types::Amount;
///
/// let bucket = ResourceBuilder::public_fungible()
///     .with_token_symbol("YOUR_TOKEN")
///     .initial_supply(Amount::from(1_000_000));
/// ```
pub struct FungibleResourceBuilder {
    owner_rule: OwnerRule,
    access_rules: ResourceAccessRules,
    token_symbol: Option<String>,
    metadata: Metadata,
    authorize_hook: Option<AuthHook>,
    address_allocation: Option<ResourceAddressAllocation>,
    divisibility: u8,
    is_total_supply_tracking_enabled: bool,
}

impl FungibleResourceBuilder {
    /// Returns a new fungible resource builder with default values.
    pub(super) fn new() -> Self {
        Self {
            owner_rule: OwnerRule::default(),
            access_rules: ResourceAccessRules::new(),
            token_symbol: None,
            metadata: Metadata::new(),
            authorize_hook: None,
            address_allocation: None,
            divisibility: DEFAULT_DIVISIBILITY,
            is_total_supply_tracking_enabled: true,
        }
    }

    /// Allows for chaining of builder methods even when conditionally applying builder methods.
    ///
    /// ## Example
    ///
    /// ```ignore
    /// use tari_template_lib::prelude::*;
    /// let resource = ResourceBuilder::public_fungible()
    ///    .with_owner_rule(rule!(allow_all))
    ///   .then(|builder| {
    ///     if some_condition {
    ///        builder.do_something_on_some_condition(..)
    ///     } else {
    ///        // or do nothing
    ///        builder
    ///     }
    ///   })
    ///   .build();
    /// ```
    pub fn then<F: FnOnce(Self) -> Self>(self, f: F) -> Self {
        f(self)
    }

    /// Sets up who will be the owner of the resource.
    ///
    /// By default, the owner is the signer of the resource creation transaction ([`OwnerRule::OwnedBySigner`]).
    ///
    /// Resource owners are the only ones allowed to update the resource's access rules after creation.
    pub fn with_owner_rule(mut self, rule: OwnerRule) -> Self {
        self.owner_rule = rule;
        self
    }

    /// Sets up who can access the resource for each type of action
    ///
    /// This allows you to pass the access rules that will be applied to the resource in a single call.
    ///
    /// Using this function will override the default access rules defined in [`ResourceAccessRules::new()`].
    pub fn with_access_rules(mut self, rules: ResourceAccessRules) -> Self {
        self.access_rules = rules;
        self
    }

    /// Sets the already allocated address for the resource
    pub fn with_address_allocation(self, address: ResourceAddressAllocation) -> Self {
        self.with_address_allocation_opt(Some(address))
    }

    /// Sets the already allocated address for the resource, optionally
    pub fn with_address_allocation_opt(mut self, address: Option<ResourceAddressAllocation>) -> Self {
        self.address_allocation = address;
        self
    }

    /// Sets up who can mint new tokens of the resource, and who may later change the mint rule.
    ///
    /// Allows you to pass an [`AccessRule`] that defines who can mint new tokens of the resource,
    /// together with an [`UpdateRule`] that controls who can update the mint rule after creation.
    ///
    /// By default, minting is disabled for all users and the rule is [`UpdateRule::Locked`].
    ///
    /// #Examples
    ///
    /// ```rust, ignore
    /// use tari_template_lib::auth::AccessRule;
    /// use tari_template_lib::prelude::{OWNER, rule};
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .mintable(rule!(allow_all), OWNER)
    ///     .build();
    /// ```
    pub fn mintable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.mintable(rule, updater);
        self
    }

    /// Sets up who can burn (destroy) tokens of the resource, and who may later change the burn rule.
    ///
    /// Allows you to pass an [`AccessRule`] that defines who can burn tokens of the resource,
    /// together with an [`UpdateRule`] that controls who can update the burn rule after creation.
    ///
    /// By default, burning is disabled for all users and the rule is [`UpdateRule::Locked`].
    ///
    /// #Examples
    ///
    /// ```rust, ignore
    /// use tari_template_lib::auth::AccessRule;
    /// use tari_template_lib::prelude::{LOCKED, rule};
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .burnable(rule!(allow_all), LOCKED)
    ///     .build();
    /// ```
    pub fn burnable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.burnable(rule, updater);
        self
    }

    /// Sets up who can recall tokens of the resource, and who may later change the recall rule.
    ///
    /// A recall is the forceful withdrawal of **ALL** tokens from any external vault.
    ///
    /// Allows you to pass an [`AccessRule`] that defines who can recall tokens from a vault, together
    /// with an [`UpdateRule`] that controls who can update the recall rule after creation. Pass
    /// [`UpdateRule::Locked`] to permanently prevent the recall rule from being changed, which can
    /// give holders strong guarantees that recall cannot be enabled later.
    ///
    /// By default, recalling is disabled for all users and the rule is [`UpdateRule::Locked`].
    ///
    /// # Examples
    ///
    /// ```rust, ignore
    /// use tari_template_lib::auth::AccessRule;
    /// use tari_template_lib::prelude::{LOCKED, rule};
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///    .recallable(rule!(allow_all), LOCKED)
    ///   .build();
    /// ```
    pub fn recallable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.recallable(rule, updater);
        self
    }

    /// Sets up who can freeze vaults containing this resource, and who may later change the freeze
    /// rule.
    ///
    /// Allows you to pass an [`AccessRule`] that defines who can freeze a vault holding this
    /// resource, together with an [`UpdateRule`] that controls who can update the freeze rule after
    /// creation. Pass [`UpdateRule::Locked`] to permanently prevent the freeze rule from being
    /// changed, which gives holders a binding guarantee that vaults of this resource can never be
    /// frozen later.
    ///
    /// By default, freezing is disabled for all users and the rule is [`UpdateRule::Locked`].
    ///
    /// # Examples
    ///
    /// ```rust, ignore
    /// use tari_template_lib::auth::AccessRule;
    /// use tari_template_lib::prelude::{LOCKED, rule};
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .freezable(rule!(deny_all), LOCKED)
    ///     .build();
    /// ```
    pub fn freezable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.freezable(rule, updater);
        self
    }

    /// Sets up who can withdraw tokens of the resource from any vault, and who may later change the
    /// withdraw rule.
    ///
    /// Allows you to pass an [`AccessRule`] that defines who can withdraw tokens (via a specified amount) from a vault,
    /// together with an [`UpdateRule`] that controls who can update the withdraw rule after creation.
    ///
    /// By default, withdrawal is allowed for all users and the rule is [`UpdateRule::Locked`].
    ///
    /// # Examples
    ///
    /// ```rust, ignore
    /// use tari_template_lib::auth::AccessRule;
    /// use tari_template_lib::prelude::LOCKED;
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .withdrawable(AccessRule::DenyAll, LOCKED)
    ///     .build();
    /// ```
    pub fn withdrawable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.withdrawable(rule, updater);
        self
    }

    /// Sets up who can deposit tokens of the resource into any vault, and who may later change the
    /// deposit rule.
    ///
    /// Allows you to pass an [`AccessRule`] that defines who can deposit tokens (via a specified amount) into a vault,
    /// together with an [`UpdateRule`] that controls who can update the deposit rule after creation.
    ///
    /// By default, deposit is allowed for all users and the rule is [`UpdateRule::Locked`].
    ///
    /// # Examples
    ///
    /// ```rust, ignore
    /// use tari_template_lib::auth::AccessRule;
    /// use tari_template_lib::prelude::{LOCKED, rule};
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///    .depositable(rule!(allow_all), LOCKED)
    ///     .build();
    /// ```
    pub fn depositable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.depositable(rule, updater);
        self
    }

    /// Sets up who can update the resource's metadata, and who may later change that rule. The
    /// token symbol remains immutable once set.
    ///
    /// Allows you to pass an [`AccessRule`] that defines who can replace the resource's metadata,
    /// together with an [`UpdateRule`] that controls who can update the metadata rule after
    /// creation.
    ///
    /// By default, updating the metadata is disabled for all users and the updater rule is
    /// [`UpdateRule::Owner`].
    ///
    /// # Examples
    ///
    /// ```rust, ignore
    /// use tari_template_lib::auth::AccessRule;
    /// use tari_template_lib::prelude::{OWNER, rule};
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .update_metadata(rule!(allow_all), OWNER)
    ///     .build();
    /// ```
    pub fn update_metadata<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.update_metadata(rule, updater);
        self
    }

    /// Sets up the specified `symbol` as the token symbol in the metadata of the resource
    ///
    /// # Examples
    /// ```rust, ignore
    ///  use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .with_token_symbol("MY_TOKEN")
    ///     .build();
    /// ```
    pub fn with_token_symbol<S: Into<String>>(mut self, symbol: S) -> Self {
        self.token_symbol = Some(symbol.into());
        self
    }

    /// Adds a new metadata entry to the resource
    ///
    /// Allows you to add a key-value pair to the resource's metadata.
    ///
    /// # Notes
    ///
    /// `.add_metadata()` will override any existing metadata with the same key.
    ///
    /// # Examples
    /// ```rust, ignore
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///    .add_metadata("CharacterName", "Tari")
    ///    .add_metadata("CharacterType", "Mascot")
    ///    .add_metadata("CharacterLvl", "98")
    /// .build();
    /// ```
    pub fn add_metadata<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Adds a new metadata entry to the resource
    ///
    /// Allows you to add a key-value pair to the resource's metadata.
    /// This is an alias for `.add_metadata()`.
    ///
    /// # Notes
    ///
    /// `.metadata()` will override any existing metadata with the same key.
    ///
    /// # Examples
    /// ```rust, ignore
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///    .metadata("CharacterName", "Tari")
    ///    .metadata("CharacterType", "Mascot")
    ///    .metadata("CharacterLvl", "99")
    /// .build();
    /// ```
    pub fn metadata<K: Into<String>, V: Into<String>>(self, key: K, value: V) -> Self {
        self.add_metadata(key, value)
    }

    /// Replaces the resource's metadata with the given [`Metadata`] object.
    ///
    /// Note: This will overwrite any existing metadata, including entries added via `.add_metadata()`.
    ///
    /// # Examples
    /// ```rust, ignore
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// use tari_template_lib::types::Metadata;
    /// let metadata = Metadata::from([
    ///     ("Type", "NFT"),
    ///     ("Creator", "Tari Project"),
    /// ]);
    /// let address = ResourceBuilder::public_fungible()
    ///     .with_metadata(metadata)
    ///     .build();
    /// ```
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Sets up the image URL of the resource
    ///
    /// Allows you to set the image URL of the resource, which can be used in user interfaces to display the token's
    /// logo or image.
    ///
    /// # Examples
    /// ```rust, ignore
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .with_image_url("https://example.com/my_token_image.png".to_string())
    ///     .build();
    /// ```
    pub fn with_image_url(self, url: String) -> Self {
        self.add_metadata(IMAGE_URL, url)
    }

    /// Sets the divisibility of the resource. i.e. the number of decimal places
    ///
    /// The default divisibility is 18, which means the smallest unit of the resource is 0.000000000000000001 of the
    /// whole unit.
    ///
    /// # Panics
    /// method will panic if:
    /// * The divisibility is greater than 18.
    ///
    /// # Examples
    /// ```rust, ignore
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .with_divisibility(9)
    ///     .build();
    /// ```
    pub fn with_divisibility(mut self, divisibility: u8) -> Self {
        if divisibility > 18 {
            panic!("Divisibility cannot be greater than 18");
        }
        self.divisibility = divisibility;
        self
    }

    /// Specify a hook method that will be called to authorize actions on the resource.
    /// The signature of the method must be `fn(action: ResourceAuthAction, caller: CallerContext)`.
    /// The method should panic to deny the action.
    /// The resource will fail to build if the component's template does not have a method with the specified signature.
    /// Hooks are only run when the resource is acted on by an external component.
    ///
    /// ## Examples
    ///
    /// Building a resource with a hook from within a component
    /// ```ignore
    /// use tari_template_lib::{caller_context::CallerContext, prelude::ResourceBuilder};
    /// ResourceBuilder::public_fungible()
    ///     .with_authorization_hook(CallerContext::current_component_address(), "my_hook")
    ///     .build();
    /// ```
    ///
    /// Building a resource with a hook in a static template function. The address is allocated beforehand.
    ///
    /// ```ignore
    /// use tari_template_lib::{caller_context::CallerContext, prelude::ResourceBuilder};
    /// let alloc = CallerContext::allocate_component_address();
    /// ResourceBuilder::public_fungible()
    ///     .with_authorization_hook(*alloc.address(), "my_hook")
    ///     .build();
    /// ```
    /// Sets up who can replace or remove the resource's authorization hook after creation.
    ///
    /// A hook is [`LOCKED`](tari_template_lib_types::access_rules::LOCKED) by default: it binds for the life of
    /// the resource, and a hook that panics or denies unconditionally leaves every vault of the resource
    /// unspendable. Setting an updater buys the ability to repair or retire the hook, at the cost of letting
    /// whoever satisfies the updater change the rules that existing holders are relying on.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use tari_template_lib::{caller_context::CallerContext, prelude::{OWNER, ResourceBuilder}};
    /// ResourceBuilder::public_fungible()
    ///     .with_authorization_hook(CallerContext::current_component_address(), "my_hook")
    ///     .with_authorization_hook_updater(OWNER)
    ///     .build();
    /// ```
    pub fn with_authorization_hook_updater<U: Into<UpdateRule>>(mut self, updater: U) -> Self {
        self.access_rules = self.access_rules.set_auth_hook_updater(updater);
        self
    }

    pub fn with_authorization_hook<T: TryInto<FunctionName>>(
        mut self,
        address: ComponentAddress,
        auth_callback: T,
    ) -> Self {
        self.authorize_hook = Some(AuthHook::new(
            address,
            auth_callback
                .try_into()
                .unwrap_or_else(|_| panic!("{}", ERR_AUTH_HOOK_FN_NAME_LEN)),
        ));
        self
    }

    /// Disables the tracking of total supply for the resource.
    ///
    /// By default, total supply tracking is enabled. `.disable_total_supply_tracking()` can be used to disable it.
    /// Use cases include privacy focused tokens or utility tokens where the total supply is not relevant.
    ///
    /// # Examples
    /// ```rust, ignore
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// ResourceBuilder::public_fungible()
    ///     .disable_total_supply_tracking()
    ///     .build();
    /// ```
    pub fn disable_total_supply_tracking(mut self) -> Self {
        self.is_total_supply_tracking_enabled = false;
        self
    }

    /// Build the resource, returning the address
    ///
    /// Utilises an internal method to create the resource with the specified properties.
    ///      
    pub fn build(self) -> ResourceAddress {
        let (address, _) = self.build_internal(None);
        address
    }

    /// This builds the resource and returns a bucket containing the initial supply based on the passed
    /// [`Amount`].
    ///
    /// # Examples
    /// ```rust, ignore
    /// use tari_template_lib::resource::builder::ResourceBuilder;
    /// use tari_template_lib::types::Amount;
    /// let bucket = ResourceBuilder::public_fungible()
    ///     .with_token_symbol("YOUR_TOKEN")
    ///     .initial_supply(Amount::from(1_000_000));
    /// ```
    pub fn initial_supply<A: Into<Amount>>(self, initial_supply: A) -> Bucket {
        let mint_arg = MintArg::Fungible {
            amount: initial_supply.into(),
        };

        let (_, bucket) = self.build_internal(Some(mint_arg));
        bucket.expect("[initial_supply] Bucket not returned from system")
    }

    fn build_internal(mut self, mint_arg: Option<MintArg>) -> (ResourceAddress, Option<Bucket>) {
        if let Some(symbol) = self.token_symbol {
            self.metadata.insert(TOKEN_SYMBOL, symbol);
        }
        ResourceManager::create(
            ResourceType::Fungible,
            self.owner_rule,
            self.access_rules,
            self.metadata,
            mint_arg,
            None,
            self.authorize_hook,
            self.address_allocation,
            self.divisibility,
            self.is_total_supply_tracking_enabled,
        )
    }
}
