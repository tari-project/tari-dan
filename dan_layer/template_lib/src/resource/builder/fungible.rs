//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{
    args::MintArg,
    auth::{AccessRule, OwnerRule, ResourceAccessRules},
    models::{Amount, Bucket, Metadata, ResourceAddress},
    resource::{ResourceManager, ResourceType},
};

pub struct FungibleResourceBuilder {
    token_symbol: String,
    initial_supply: Amount,
    owner_rule: OwnerRule,
    access_rules: ResourceAccessRules,
    metadata: Metadata,
}

impl FungibleResourceBuilder {
    pub(super) fn new<S: Into<String>>(token_symbol: S) -> Self {
        Self {
            token_symbol: token_symbol.into(),
            initial_supply: Amount::zero(),
            owner_rule: OwnerRule::default(),
            access_rules: ResourceAccessRules::new(),
            metadata: Metadata::new(),
        }
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

    pub fn withdrawable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.withdrawable(rule);
        self
    }

    pub fn depositable(mut self, rule: AccessRule) -> Self {
        self.access_rules = self.access_rules.depositable(rule);
        self
    }

    pub fn add_metadata<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.metadata.insert(key, value);
        self
    }

    pub fn initial_supply<A: Into<Amount>>(mut self, initial_supply: A) -> Self {
        self.initial_supply = initial_supply.into();
        self
    }

    pub fn build(self) -> ResourceAddress {
        // TODO: Improve API
        assert!(
            self.initial_supply.is_zero(),
            "call build_bucket when initial supply set"
        );
        let (address, _) = Self::build_internal(
            self.token_symbol,
            self.owner_rule,
            self.access_rules,
            self.metadata,
            None,
        );
        address
    }

    pub fn build_bucket(self) -> Bucket {
        let mint_args = MintArg::Fungible {
            amount: self.initial_supply,
        };

        let (_, bucket) = Self::build_internal(
            self.token_symbol,
            self.owner_rule,
            self.access_rules,
            self.metadata,
            Some(mint_args),
        );
        bucket.expect("[build_bucket] Bucket not returned from system")
    }

    fn build_internal(
        token_symbol: String,
        owner_rule: OwnerRule,
        access_rules: ResourceAccessRules,
        metadata: Metadata,
        mint_args: Option<MintArg>,
    ) -> (ResourceAddress, Option<Bucket>) {
        ResourceManager::new().create(
            ResourceType::Fungible,
            owner_rule,
            access_rules,
            token_symbol,
            metadata,
            mint_args,
        )
    }
}
