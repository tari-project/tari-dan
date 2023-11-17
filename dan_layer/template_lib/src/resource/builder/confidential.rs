//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use super::TOKEN_SYMBOL;
use crate::{
    args::MintArg,
    auth::{AccessRule, OwnerRule, ResourceAccessRules},
    models::{Bucket, Metadata, ResourceAddress},
    prelude::ConfidentialOutputProof,
    resource::{ResourceManager, ResourceType},
};

pub struct ConfidentialResourceBuilder {
    initial_supply_proof: Option<ConfidentialOutputProof>,
    metadata: Metadata,
    access_rules: ResourceAccessRules,
    owner_rule: OwnerRule,
}

impl ConfidentialResourceBuilder {
    pub(super) fn new() -> Self {
        Self {
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

    pub fn with_token_symbol<S: Into<String>>(mut self, symbol: S) -> Self {
        self.metadata.insert(TOKEN_SYMBOL, symbol);
        self
    }

    pub fn add_metadata<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.metadata.insert(key, value);
        self
    }

    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
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
        let (address, _) = Self::build_internal(self.owner_rule, self.access_rules, self.metadata, None);
        address
    }

    pub fn build_bucket(self) -> Bucket {
        let resource = MintArg::Confidential {
            proof: Box::new(
                self.initial_supply_proof
                    .expect("[build_bucket] initial supply not set"),
            ),
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
        ResourceManager::new().create(ResourceType::Confidential, owner_rule, access_rules, metadata, resource)
    }
}
