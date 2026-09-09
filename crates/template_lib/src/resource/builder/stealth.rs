//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use tari_template_abi::rust::prelude::*;
use tari_template_lib_types::{
    Amount,
    AuthHook,
    ComponentAddress,
    FunctionName,
    Metadata,
    OwnerRule,
    ResourceAddress,
    ResourceType,
    access_rules::{AccessRule, ResourceAccessRules, UpdateRule},
    constants::{DEFAULT_DIVISIBILITY, IMAGE_URL, TOKEN_SYMBOL},
    crypto::RistrettoPublicKeyBytes,
};

use crate::{
    args::MintArg,
    error_variants::ERR_AUTH_HOOK_FN_NAME_LEN,
    models::{Bucket, ResourceAddressAllocation},
    resource::ResourceManager,
};

/// Implements the builder pattern for Confidential resources.
pub struct StealthResourceBuilder {
    metadata: Metadata,
    access_rules: ResourceAccessRules,
    view_key: Option<RistrettoPublicKeyBytes>,
    token_symbol: Option<String>,
    owner_rule: OwnerRule,
    authorize_hook: Option<AuthHook>,
    address_allocation: Option<ResourceAddressAllocation>,
    divisibility: u8,
    is_total_supply_tracking_enabled: bool,
}

impl StealthResourceBuilder {
    /// Returns a new confidential resource builder
    pub(super) fn new() -> Self {
        Self {
            metadata: Metadata::new(),
            access_rules: ResourceAccessRules::new(),
            view_key: None,
            token_symbol: None,
            owner_rule: OwnerRule::default(),
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
    /// let resource = ResourceBuilder::stealth()
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
    /// Resource owners are the only ones allowed to update the resource's access rules after creation
    pub fn with_owner_rule(mut self, rule: OwnerRule) -> Self {
        self.owner_rule = rule;
        self
    }

    /// Sets up who can access the resource for each type of action
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

    /// Specify a view key for the stealth resource. This allows anyone with the secret key to uncover the balance
    /// of commitments generated for the resource.
    /// NOTE: it is not currently possible to change the view key after the resource is created.
    /// Equivalent to calling `with_view_key_opt(Some(view_key))`.
    pub fn with_view_key(self, view_key: RistrettoPublicKeyBytes) -> Self {
        self.with_view_key_opt(Some(view_key))
    }

    /// Optionally, specify a view key for the stealth resource. This allows anyone with the secret key to uncover the
    /// balance of commitments generated for the resource.
    /// NOTE: it is not currently possible to change the view key after the resource is created.
    pub fn with_view_key_opt(mut self, view_key: Option<RistrettoPublicKeyBytes>) -> Self {
        self.view_key = view_key;
        self
    }

    /// Sets up who can mint new tokens of the resource, and who may later change the mint rule.
    pub fn mintable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.mintable(rule, updater);
        self
    }

    /// Sets up who can burn (destroy) tokens of the resource, and who may later change the burn rule.
    pub fn burnable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.burnable(rule, updater);
        self
    }

    /// Sets up who can recall tokens of the resource, and who may later change the recall rule.
    /// A recall is the forceful withdrawal of tokens from any external vault.
    pub fn recallable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.recallable(rule, updater);
        self
    }

    /// Sets up who can freeze vaults containing this resource, and who may later change the freeze rule.
    pub fn freezable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.freezable(rule, updater);
        self
    }

    /// Sets up who can withdraw tokens of the resource from any vault, and who may later change the
    /// withdraw rule.
    pub fn withdrawable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.withdrawable(rule, updater);
        self
    }

    /// Sets up who can deposit tokens of the resource into any vault, and who may later change the
    /// deposit rule.
    pub fn depositable<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.depositable(rule, updater);
        self
    }

    /// Sets up who can update the resource's metadata, and who may later change that rule. The
    /// token symbol remains immutable once set.
    pub fn update_metadata<U: Into<UpdateRule>>(mut self, rule: AccessRule, updater: U) -> Self {
        self.access_rules = self.access_rules.update_metadata(rule, updater);
        self
    }

    /// Sets up the specified `symbol` as the token symbol in the metadata of the resource
    pub fn with_token_symbol<S: Into<String>>(mut self, symbol: S) -> Self {
        self.token_symbol = Some(symbol.into());
        self
    }

    /// Adds a new metadata entry to the resource
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
    /// ResourceBuilder::stealth()
    ///    .metadata("CharacterName", "Tari")
    ///    .metadata("CharacterType", "Mascot")
    ///    .metadata("CharacterLvl", "99")
    /// .build();
    /// ```
    pub fn metadata<K: Into<String>, V: Into<String>>(self, key: K, value: V) -> Self {
        self.add_metadata(key, value)
    }

    /// Sets up all the metadata entries of the resource.
    /// WARNING: this will overwrite any previously set metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Sets up the image URL of the resource
    pub fn with_image_url(self, url: String) -> Self {
        self.add_metadata(IMAGE_URL, url)
    }

    /// Sets the divisibility of the resource. i.e. the number of decimal places.
    /// Panic if the divisibility is greater than 18.
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
    /// The resource will fail to build if the component's template does not have a method with the correct signature.
    /// Hooks are only run when the resource is acted on by an external component.
    ///
    /// ## Examples
    ///
    /// Building a resource with a hook from within a component
    /// ```ignore
    /// # use tari_template_lib::{caller_context::CallerContext, prelude::ResourceBuilder};
    /// ResourceBuilder::confidential()
    ///     .with_authorization_hook(CallerContext::current_component_address(), "my_hook")
    ///     .build();
    /// ```
    ///
    /// Building a resource with a hook in a static template function. The address is allocated beforehand.
    ///
    /// ```ignore
    /// # use tari_template_lib::{caller_context::CallerContext, prelude::ResourceBuilder};
    /// let alloc = CallerContext::allocate_component_address();
    /// ResourceBuilder::confidential()
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
    /// ResourceBuilder::stealth()
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
    /// This is useful for resources that do not need to track the total supply.
    /// Disabling total supply tracking can save on fees.
    pub fn disable_total_supply_tracking(mut self) -> Self {
        self.is_total_supply_tracking_enabled = false;
        self
    }

    /// Build the resource, returning the address
    pub fn build(self) -> ResourceAddress {
        let (address, _) = self.build_internal(None);
        address
    }

    /// Sets up how many tokens are going to be minted on resource creation
    /// This builds the resource and mints the initial supply of tokens, returning the address of the resource.
    /// NOTE that stealth resources do not return the bucket of the initial supply since
    /// they are minted as individual UTXO substates and cannot be placed in vault.
    pub fn initial_supply<A: Into<Amount>>(self, initial_supply: A) -> Bucket {
        let mint_arg = MintArg::Stealth {
            amount: initial_supply.into(),
        };

        let (_, bucket) = self.build_internal(Some(mint_arg));
        bucket.expect("[initial_supply] Bucket not returned from engine")
    }

    fn build_internal(mut self, mint_arg: Option<MintArg>) -> (ResourceAddress, Option<Bucket>) {
        if let Some(symbol) = self.token_symbol {
            self.metadata.insert(TOKEN_SYMBOL, symbol);
        }
        ResourceManager::create(
            ResourceType::Stealth,
            self.owner_rule,
            self.access_rules,
            self.metadata,
            mint_arg,
            self.view_key,
            self.authorize_hook,
            self.address_allocation,
            self.divisibility,
            self.is_total_supply_tracking_enabled,
        )
    }
}
