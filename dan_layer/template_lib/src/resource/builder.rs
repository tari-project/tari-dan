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

use tari_template_abi::{encode, Encode};

use crate::{
    args::MintResourceArg,
    models::{Amount, Bucket, Metadata},
};

pub struct ResourceBuilder;

impl ResourceBuilder {
    pub fn fungible() -> FungibleResourceBuilder {
        FungibleResourceBuilder::new()
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
        self.metadata.insert(b"SYMBOL".to_vec(), symbol.into().into_bytes());
        self
    }

    pub fn with_metadata<K: Encode, V: Encode>(mut self, key: K, value: V) -> Self {
        self.metadata.insert(encode(&key).unwrap(), encode(&value).unwrap());
        self
    }

    pub fn initial_supply<A: Into<Amount>>(mut self, initial_supply: A) -> Self {
        self.initial_supply = initial_supply.into();
        self
    }

    pub fn build_bucket(self) -> Bucket {
        crate::get_context().with_resource_manager(|manager| {
            manager.mint_resource(MintResourceArg::Fungible {
                amount: self.initial_supply,
                metadata: self.metadata,
            })
        })
    }
}
