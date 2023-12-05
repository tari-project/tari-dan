//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::BTreeMap;

use serde::Serialize;
use tari_bor::encode;
use tari_template_abi::rust::{fmt, ops::RangeInclusive};

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
    pub(super) fn new() -> Self {
        Self {
            owner_rule: OwnerRule::default(),
            metadata: Metadata::new(),
            access_rules: ResourceAccessRules::new(),
            tokens_ids: BTreeMap::new(),
        }
    }

    pub fn with_token_symbol<S: Into<String>>(mut self, symbol: S) -> Self {
        self.metadata.insert(TOKEN_SYMBOL, symbol);
        self
    }

    pub fn with_owner_rule(mut self, rule: OwnerRule) -> Self {
        self.owner_rule = rule;
        self
    }

    pub fn with_access_rules(mut self, rules: ResourceAccessRules) -> Self {
        self.access_rules = rules;
        self
    }

    pub fn mintable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.mintable(rule);
        self
    }

    pub fn burnable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.burnable(rule);
        self
    }

    pub fn recallable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.recallable(rule);
        self
    }

    pub fn withdrawable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.withdrawable(rule);
        self
    }

    pub fn depositable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.depositable(rule);
        self
    }

    pub fn update_non_fungible_data(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.update_non_fungible_data(rule);
        self
    }

    pub fn add_metadata<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.metadata.insert(key, value);
        self
    }

    pub fn with_non_fungible<T, U>(mut self, id: NonFungibleId, data: &T, mutable: &U) -> Self
    where
        T: Serialize,
        U: Serialize,
    {
        self.tokens_ids
            .insert(id, (encode(data).unwrap(), encode(mutable).unwrap()));
        self
    }

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

    pub fn mint_many_with<F, T>(mut self, bounds: RangeInclusive<usize>, mut f: F) -> Self
    where
        F: FnMut(T) -> (NonFungibleId, (Vec<u8>, Vec<u8>)),
        T: TryFrom<usize>,
        T::Error: fmt::Debug,
    {
        self.tokens_ids.extend(bounds.map(|n| f(n.try_into().unwrap())));
        self
    }

    pub fn build(self) -> ResourceAddress {
        // TODO: Improve API
        assert!(self.tokens_ids.is_empty(), "call build_bucket with initial tokens set");
        let (address, _) = Self::build_internal(self.owner_rule, self.access_rules, self.metadata, None);
        address
    }

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
