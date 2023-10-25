//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{
    args::MintArg,
    auth::{AccessRule, OwnerRule, ResourceAccessRules},
    models::{Bucket, Metadata, ResourceAddress},
    prelude::ConfidentialOutputProof,
    resource::{ResourceManager, ResourceType},
};

pub struct ConfidentialResourceBuilder {
    initial_supply_proof: Option<ConfidentialOutputProof>,
    token_symbol: String,
    metadata: Metadata,
    access_rules: ResourceAccessRules,
    owner_rule: OwnerRule,
}

impl ConfidentialResourceBuilder {
    pub(super) fn new<S: Into<String>>(token_symbol: S) -> Self {
        Self {
            token_symbol: token_symbol.into(),
            initial_supply_proof: None,
            metadata: Metadata::new(),
            access_rules: ResourceAccessRules::new(),
            owner_rule: OwnerRule::default(),
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

    pub fn initial_supply(mut self, initial_supply: ConfidentialOutputProof) -> Self {
        self.initial_supply_proof = Some(initial_supply);
        self
    }

    pub fn build(self) -> ResourceAddress {
        // TODO: Improve API
        assert!(
            self.initial_supply_proof.is_none(),
            "call build_bucket when initial supply is set"
        );
        let (address, _) = Self::build_internal(
            self.owner_rule,
            self.access_rules,
            self.token_symbol,
            self.metadata,
            None,
        );
        address
    }

    pub fn build_bucket(self) -> Bucket {
        let mint_args = MintArg::Confidential {
            proof: Box::new(
                self.initial_supply_proof
                    .expect("[build_bucket] initial supply not set"),
            ),
        };

        let (_, bucket) = Self::build_internal(
            self.owner_rule,
            self.access_rules,
            self.token_symbol,
            self.metadata,
            Some(mint_args),
        );
        bucket.expect("[build_bucket] Bucket not returned from system")
    }

    fn build_internal(
        owner_rule: OwnerRule,
        access_rules: ResourceAccessRules,
        token_symbol: String,
        metadata: Metadata,
        mint_args: Option<MintArg>,
    ) -> (ResourceAddress, Option<Bucket>) {
        ResourceManager::new().create(
            ResourceType::Confidential,
            owner_rule,
            access_rules,
            token_symbol,
            metadata,
            mint_args,
        )
    }
}
