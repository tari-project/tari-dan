//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use super::TOKEN_SYMBOL;
use crate::{
    args::MintArg,
    auth::{AccessRule, AuthHook, OwnerRule, ResourceAccessRules},
    models::{Amount, Bucket, ComponentAddress, Metadata, ResourceAddress},
    resource::{ResourceManager, ResourceType},
};

/// Utility for building fungible resources inside templates
pub struct FungibleResourceBuilder {
    owner_rule: OwnerRule,
    access_rules: ResourceAccessRules,
    token_symbol: Option<String>,
    metadata: Metadata,
    authorize_hook: Option<AuthHook>,
}

impl FungibleResourceBuilder {
    /// Returns a new fungible resource builder
    pub(super) fn new() -> Self {
        Self {
            owner_rule: OwnerRule::default(),
            access_rules: ResourceAccessRules::new(),
            token_symbol: None,
            metadata: Metadata::new(),
            authorize_hook: None,
        }
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

    /// Sets up who can mint new tokens of the resource
    pub fn mintable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.mintable(rule);
        self
    }

    /// Sets up who can burn (destroy) tokens of the resource
    pub fn burnable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.burnable(rule);
        self
    }

    /// Sets up who can recall tokens of the resource.
    /// A recall is the forceful withdrawal of tokens from any external vault
    pub fn recallable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.recallable(rule);
        self
    }

    /// Sets up who can withdraw tokens of the resource from any vault
    pub fn withdrawable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.withdrawable(rule);
        self
    }

    /// Sets up who can deposit tokens of the resource into any vault
    pub fn depositable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.depositable(rule);
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

    /// Sets up all the metadata entries of the resource
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
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
    /// ```rust
    /// use tari_template_lib::{caller_context::CallerContext, prelude::ResourceBuilder};
    /// ResourceBuilder::fungible()
    ///     .with_authorization_hook(CallerContext::current_component_address(), "my_hook")
    ///     .build();
    /// ```
    ///
    /// Building a resource with a hook in a static template function. The address is allocated beforehand.
    ///
    /// ```rust
    /// use tari_template_lib::{caller_context::CallerContext, prelude::ResourceBuilder};
    /// let alloc = CallerContext::allocate_component_address();
    /// ResourceBuilder::fungible()
    ///     .with_authorization_hook(*alloc.address(), "my_hook")
    ///     .build();
    /// ```
    pub fn with_authorization_hook<T: Into<String>>(mut self, address: ComponentAddress, auth_callback: T) -> Self {
        self.authorize_hook = Some(AuthHook::new(address, auth_callback.into()));
        self
    }

    /// Build the resource, returning the address
    pub fn build(self) -> ResourceAddress {
        let (address, _) = self.build_internal(None);
        address
    }

    /// Sets up how many tokens are going to be minted on resource creation
    /// This builds the resource and returns a bucket containing the initial supply.
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
        ResourceManager::new().create(
            ResourceType::Fungible,
            self.owner_rule,
            self.access_rules,
            self.metadata,
            mint_arg,
            None,
            self.authorize_hook,
        )
    }
}
