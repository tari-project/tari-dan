//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_bor::{encode, Encode};
use tari_template_abi::rust::{collections::HashMap, fmt, ops::RangeInclusive};

use crate::{
    args::MintArg,
    models::{Amount, Bucket, Metadata, NonFungibleId, ResourceAddress},
    prelude::ResourceType,
    resource::ResourceManager,
};

const TOKEN_SYMBOL: &str = "SYMBOL";

pub struct ResourceBuilder;

impl ResourceBuilder {
    pub fn fungible() -> FungibleResourceBuilder {
        FungibleResourceBuilder::new()
    }

    pub fn non_fungible() -> NonFungibleResourceBuilder {
        NonFungibleResourceBuilder::new()
    }
}

pub struct FungibleResourceBuilder {
    initial_supply: Amount,
    metadata: Metadata,
}

impl FungibleResourceBuilder {
    fn new() -> Self {
        Self {
            initial_supply: Amount::zero(),
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
        let (address, _) = Self::build_internal(self.metadata, None);
        address
    }

    pub fn build_bucket(self) -> Bucket {
        let mint_args = MintArg::Fungible {
            amount: self.initial_supply,
        };

        let (_, bucket) = Self::build_internal(self.metadata, Some(mint_args));
        bucket.expect("[build_bucket] Bucket not returned from system")
    }

    fn build_internal(metadata: Metadata, mint_args: Option<MintArg>) -> (ResourceAddress, Option<Bucket>) {
        ResourceManager::new().create(ResourceType::Fungible, metadata, mint_args)
    }
}

pub struct NonFungibleResourceBuilder {
    metadata: Metadata,
    tokens_ids: HashMap<NonFungibleId, (Vec<u8>, Vec<u8>)>,
}

impl NonFungibleResourceBuilder {
    fn new() -> Self {
        Self {
            metadata: Metadata::new(),
            tokens_ids: HashMap::new(),
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

    pub fn with_non_fungibles<'a, I, T, U>(mut self, tokens: I) -> Self
    where
        I: IntoIterator<Item = (NonFungibleId, (&'a T, &'a U))>,
        T: Encode + 'a,
        U: Encode + 'a,
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
        let (address, _) = Self::build_internal(self.metadata, None);
        address
    }

    pub fn build_bucket(self) -> Bucket {
        let mint_args = MintArg::NonFungible {
            tokens: self.tokens_ids,
        };

        let (_, bucket) = Self::build_internal(self.metadata, Some(mint_args));
        bucket.expect("[build_bucket] Bucket not returned from system")
    }

    fn build_internal(metadata: Metadata, mint_args: Option<MintArg>) -> (ResourceAddress, Option<Bucket>) {
        ResourceManager::new().create(ResourceType::NonFungible, metadata, mint_args)
    }
}
