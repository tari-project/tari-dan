//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::BTreeMap;

use serde::Serialize;
use tari_bor::encode;

use super::TOKEN_SYMBOL;
use crate::{
    args::MintArg,
    auth::{AccessRule, OwnerRule, ResourceAccessRules},
    models::{Bucket, Metadata, NonFungibleId, ResourceAddress},
    resource::{ResourceManager, ResourceType},
};

/// Utility for building non-fungible resources inside templates
pub struct NonFungibleResourceBuilder {
    owner_rule: OwnerRule,
    metadata: Metadata,
    access_rules: ResourceAccessRules,
    tokens_ids: BTreeMap<NonFungibleId, (Vec<u8>, Vec<u8>)>,
}

impl NonFungibleResourceBuilder {
    /// Returns a new non-fungible resource builder
    pub(super) fn new() -> Self {
        Self {
            owner_rule: OwnerRule::default(),
            metadata: Metadata::new(),
            access_rules: ResourceAccessRules::new(),
            tokens_ids: BTreeMap::new(),
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

    /// Sets up who can update the mutable data of the tokens in the resource
    pub fn update_non_fungible_data(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.update_non_fungible_data(rule);
        self
    }

    /// Sets up the specified `symbol` as the token symbol in the metadata of the resource
    pub fn with_token_symbol<S: Into<String>>(mut self, symbol: S) -> Self {
        self.metadata.insert(TOKEN_SYMBOL, symbol);
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

    /// Sets up an initial non-fungible token to be minted on resource creation
    pub fn with_non_fungible<T, U>(mut self, id: NonFungibleId, data: &T, mutable: &U) -> Self
    where
        T: Serialize,
        U: Serialize,
    {
        self.tokens_ids
            .insert(id, (encode(data).unwrap(), encode(mutable).unwrap()));
        self
    }

    /// Sets up multiple initial non-fungible tokens to be minted on resource creation
    pub fn with_non_fungibles<'a, I, T, U>(mut self, tokens: I) -> Self
    where
        I: IntoIterator<Item = (NonFungibleId, (&'a T, &'a U))>,
        T: Serialize + 'a,
        U: Serialize + 'a,
    {
        self.tokens_ids.extend(
            tokens
                .into_iter()
                .map(|(id, (data, mutable))| (id, (encode(data).unwrap(), encode(mutable).unwrap()))),
        );
        self
    }

    /// Sets up multiple initial non-fungible tokens to be minted on resource creation by applying the provided function
    /// N times
    pub fn mint_many_with<F, I, V>(mut self, iter: I, f: F) -> Self
    where
        F: FnMut(V) -> (NonFungibleId, (Vec<u8>, Vec<u8>)),
        I: IntoIterator<Item = V>,
    {
        self.tokens_ids.extend(iter.into_iter().map(f));
        self
    }

    /// Build the resource, returning the address
    pub fn build(self) -> ResourceAddress {
        // TODO: Improve API
        assert!(self.tokens_ids.is_empty(), "call build_bucket with initial tokens set");
        let (address, _) = Self::build_internal(self.owner_rule, self.access_rules, self.metadata, None);
        address
    }

    /// Build the resource and return a bucket with the initial minted tokens (if specified previously)
    pub fn build_bucket(self) -> Bucket {
        let resource = MintArg::NonFungible {
            tokens: self.tokens_ids,
        };

        let (_, bucket) = Self::build_internal(self.owner_rule, self.access_rules, self.metadata, Some(resource));
        bucket.expect("[build_bucket] Bucket not returned from system")
    }

    fn build_internal(
        owner_rule: OwnerRule,
        access_rules: ResourceAccessRules,
        metadata: Metadata,
        resource: Option<MintArg>,
    ) -> (ResourceAddress, Option<Bucket>) {
        ResourceManager::new().create(ResourceType::NonFungible, owner_rule, access_rules, metadata, resource)
    }
}
