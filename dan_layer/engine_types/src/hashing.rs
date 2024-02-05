//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{io, io::Write};

use blake2::{
    digest::consts::{U32, U64},
    Blake2b,
};
use digest::Digest;
use serde::Serialize;
use tari_bor::encode_into;
use tari_crypto::hashing::DomainSeparation;
use tari_hash_domains::TariEngineHashDomain;
use tari_template_lib::Hash;

pub fn hasher64(label: EngineHashDomainLabel) -> TariHasher64 {
    TariHasher64::new_with_label::<TariEngineHashDomain>(label.as_label())
}

pub fn template_hasher64() -> TariHasher64 {
    hasher64(EngineHashDomainLabel::Template)
}

pub fn substate_value_hasher32() -> TariHasher32 {
    hasher32(EngineHashDomainLabel::SubstateValue)
}

pub fn hasher32(label: EngineHashDomainLabel) -> TariHasher32 {
    TariHasher32::new_with_label::<TariEngineHashDomain>(label.as_label())
}

pub fn template_hasher32() -> TariHasher32 {
    hasher32(EngineHashDomainLabel::Template)
}

#[derive(Debug, Clone)]
pub struct TariHasher32 {
    hasher: Blake2b<U32>,
}
impl TariHasher32 {
    pub fn new_with_label<TDomain: DomainSeparation>(label: &'static str) -> Self {
        let mut hasher = Blake2b::<U32>::new();
        TDomain::add_domain_separation_tag(&mut hasher, label);
        Self { hasher }
    }

    pub fn update<T: Serialize + ?Sized>(&mut self, data: &T) {
        // CBOR encoding does not make any contract to say that if the writer is infallible (as it is here) then
        // encoding in infallible. However this should be the case. Since it is very unergonomic to return an
        // error in hash chain functions, and therefore all usages of the hasher, we assume all types implement
        // infallible encoding.
        encode_into(data, &mut self.hash_writer()).expect("encoding failed")
    }

    pub fn chain<T: Serialize + ?Sized>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    pub fn digest<T: Serialize + ?Sized>(self, data: &T) -> Hash {
        self.chain(data).result()
    }

    pub fn result(self) -> Hash {
        let hash: [u8; 32] = self.hasher.finalize().into();
        hash.into()
    }

    pub fn finalize_into(self, output: &mut digest::Output<Blake2b<U32>>) {
        digest::FixedOutput::finalize_into(self.hasher, output)
    }

    fn hash_writer(&mut self) -> impl Write + '_ {
        struct HashWriter<'a>(&'a mut Blake2b<U32>);
        impl Write for HashWriter<'_> {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.update(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        HashWriter(&mut self.hasher)
    }
}

#[derive(Debug, Clone)]
pub struct TariHasher64 {
    hasher: Blake2b<U64>,
}

impl TariHasher64 {
    pub fn new_with_label<TDomain: DomainSeparation>(label: &'static str) -> Self {
        let mut hasher = Blake2b::<U64>::new();
        TDomain::add_domain_separation_tag(&mut hasher, label);
        Self { hasher }
    }

    pub fn update<T: Serialize + ?Sized>(&mut self, data: &T) {
        // CBOR encoding does not make any contract to say that if the writer is infallible (as it is here) then
        // encoding in infallible. However this should be the case. Since it is very unergonomic to return an
        // error in hash chain functions, and therefore all usages of the hasher, we assume all types implement
        // infallible encoding.
        encode_into(data, &mut self.hash_writer()).expect("encoding failed")
    }

    pub fn chain<T: Serialize + ?Sized>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    pub fn digest<T: Serialize + ?Sized>(self, data: &T) -> [u8; 64] {
        self.chain(data).result()
    }

    pub fn result(self) -> [u8; 64] {
        self.hasher.finalize().into()
    }

    pub fn finalize_into(self, output: &mut digest::Output<Blake2b<U64>>) {
        digest::FixedOutput::finalize_into(self.hasher, output)
    }

    fn hash_writer(&mut self) -> impl Write + '_ {
        struct HashWriter<'a>(&'a mut Blake2b<U64>);
        impl Write for HashWriter<'_> {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.update(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        HashWriter(&mut self.hasher)
    }
}

#[derive(Debug)]
pub enum EngineHashDomainLabel {
    Template,
    SubstateAddress,
    ConfidentialProof,
    ConfidentialTransfer,
    ShardPledgeCollection,
    HotStuffTreeNode,
    Transaction,
    NonFungibleId,
    NonFungibleIndex,
    UuidOutput,
    Output,
    TransactionSignature,
    ResourceAddress,
    ComponentAddress,
    RandomBytes,
    TransactionReceipt,
    FeeClaimAddress,
    QuorumCertificate,
    SubstateValue,
}

impl EngineHashDomainLabel {
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Template => "Template",
            Self::SubstateAddress => "SubstateAddress",
            Self::ConfidentialProof => "ConfidentialProof",
            Self::ConfidentialTransfer => "ConfidentialTransfer",
            Self::ShardPledgeCollection => "ShardPledgeCollection",
            Self::HotStuffTreeNode => "HotStuffTreeNode",
            Self::Transaction => "Transaction",
            Self::NonFungibleId => "NonFungibleId",
            Self::NonFungibleIndex => "NonFungibleIndex",
            Self::UuidOutput => "UuidOutput",
            Self::Output => "Output",
            Self::TransactionSignature => "TransactionSignature",
            Self::ResourceAddress => "ResourceAddress",
            Self::ComponentAddress => "ComponentAddress",
            Self::RandomBytes => "RandomBytes",
            Self::TransactionReceipt => "TransactionReceipt",
            Self::FeeClaimAddress => "FeeClaimAddress",
            Self::QuorumCertificate => "QuorumCertificate",
            Self::SubstateValue => "SubstateValue",
        }
    }
}
