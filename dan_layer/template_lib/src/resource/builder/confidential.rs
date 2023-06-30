//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{
    args::MintArg,
    models::{Bucket, Metadata, ResourceAddress},
    prelude::ConfidentialOutputProof,
    resource::{ResourceManager, ResourceType},
};

pub struct ConfidentialResourceBuilder {
    initial_supply_proof: Option<ConfidentialOutputProof>,
    token_symbol: String,
    metadata: Metadata,
}

impl ConfidentialResourceBuilder {
    pub(super) fn new<S: Into<String>>(token_symbol: S) -> Self {
        Self {
            token_symbol: token_symbol.into(),
            initial_supply_proof: None,
            metadata: Metadata::new(),
        }
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
            self.initial_supply_proof.is_some(),
            "call build_bucket when initial supply set"
        );
        let (address, _) = Self::build_internal(self.token_symbol, self.metadata, None);
        address
    }

    pub fn build_bucket(self) -> Bucket {
        let mint_args = MintArg::Confidential {
            proof: Box::new(
                self.initial_supply_proof
                    .expect("[build_bucket] initial supply not set"),
            ),
        };

        let (_, bucket) = Self::build_internal(self.token_symbol, self.metadata, Some(mint_args));
        bucket.expect("[build_bucket] Bucket not returned from system")
    }

    fn build_internal(
        token_symbol: String,
        metadata: Metadata,
        mint_args: Option<MintArg>,
    ) -> (ResourceAddress, Option<Bucket>) {
        ResourceManager::new().create(ResourceType::Confidential, token_symbol, metadata, mint_args)
    }
}
