//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{
    args::MintArg,
    models::{Amount, Bucket, Metadata, ResourceAddress},
    resource::{builder::TOKEN_SYMBOL, ResourceManager, ResourceType},
};

pub struct FungibleResourceBuilder {
    token_symbol: String,
    initial_supply: Amount,
    metadata: Metadata,
}

impl FungibleResourceBuilder {
    pub(super) fn new<S: Into<String>>(token_symbol: S) -> Self {
        Self {
            token_symbol: token_symbol.into(),
            initial_supply: Amount::zero(),
            metadata: Metadata::new(),
        }
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
        let (address, _) = Self::build_internal(self.token_symbol, self.metadata, None);
        address
    }

    pub fn build_bucket(self) -> Bucket {
        let mint_args = MintArg::Fungible {
            amount: self.initial_supply,
        };

        let (_, bucket) = Self::build_internal(self.token_symbol, self.metadata, Some(mint_args));
        bucket.expect("[build_bucket] Bucket not returned from system")
    }

    fn build_internal(
        token_symbol: String,
        metadata: Metadata,
        mint_args: Option<MintArg>,
    ) -> (ResourceAddress, Option<Bucket>) {
        ResourceManager::new().create(ResourceType::Fungible, token_symbol, metadata, mint_args)
    }
}
