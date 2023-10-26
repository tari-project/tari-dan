//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use super::TOKEN_SYMBOL;
use crate::{
    args::MintArg,
    models::{Bucket, Metadata, ResourceAddress},
    prelude::ConfidentialOutputProof,
    resource::{ResourceManager, ResourceType},
};

pub struct ConfidentialResourceBuilder {
    initial_supply_proof: Option<ConfidentialOutputProof>,
    metadata: Metadata,
}

impl ConfidentialResourceBuilder {
    pub(super) fn new() -> Self {
        Self {
            initial_supply_proof: None,
            metadata: Metadata::new(),
        }
    }

    pub fn with_token_symbol<S: Into<String>>(mut self, symbol: S) -> Self {
        self.metadata.insert(TOKEN_SYMBOL, symbol);
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
            self.initial_supply_proof.is_some(),
            "call build_bucket when initial supply set"
        );
        let (address, _) = Self::build_internal(self.metadata, None);
        address
    }

    pub fn build_bucket(self) -> Bucket {
        let mint_args = MintArg::Confidential {
            proof: Box::new(
                self.initial_supply_proof
                    .expect("[build_bucket] initial supply not set"),
            ),
        };

        let (_, bucket) = Self::build_internal(self.metadata, Some(mint_args));
        bucket.expect("[build_bucket] Bucket not returned from system")
    }

    fn build_internal(metadata: Metadata, mint_args: Option<MintArg>) -> (ResourceAddress, Option<Bucket>) {
        ResourceManager::new().create(ResourceType::Confidential, metadata, mint_args)
    }
}
