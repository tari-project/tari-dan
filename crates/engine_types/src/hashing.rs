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

use blake2::{
    Blake2b,
    digest::consts::{U32, U64},
};
use borsh::BorshSerialize;
use tari_crypto::{
    hash_domain,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    signatures::SchnorrSignature,
};
use tari_hashing::DomainSeparatedBorshHasher;
use tari_template_lib::types::Hash32;

hash_domain!(TariEngineHashDomain, "com.tari.ootle.engine", 0);

pub fn engine_hasher64(label: EngineHashDomainLabel) -> TariEngineHasher64 {
    TariEngineHasher64::new_with_label(label.as_label())
}

pub fn substate_value_hasher32() -> TariHasher32 {
    hasher32(EngineHashDomainLabel::SubstateValue)
}

pub fn hasher32(label: EngineHashDomainLabel) -> TariHasher32 {
    TariHasher32::new_with_label(label.as_label())
}

pub fn template_hasher32() -> TariHasher32 {
    hasher32(EngineHashDomainLabel::Template)
}

pub fn hash_template_code(code: &[u8]) -> Hash32 {
    template_hasher32().chain(&code).result()
}

pub type EngineSchnorrSignature = SchnorrSignature<RistrettoPublicKey, RistrettoSecretKey, TariEngineHashDomain>;

pub struct TariHasher32 {
    hasher: DomainSeparatedBorshHasher<TariEngineHashDomain, Blake2b<U32>>,
}
impl TariHasher32 {
    pub fn new_with_label(label: &'static str) -> Self {
        let hasher = DomainSeparatedBorshHasher::new_with_label(label);
        Self { hasher }
    }

    pub fn update<T: BorshSerialize + ?Sized>(&mut self, data: &T) {
        self.hasher.update_consensus_encode(data)
    }

    pub fn chain<T: BorshSerialize + ?Sized>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    pub fn result(self) -> Hash32 {
        self.hasher.finalize_into_array().into()
    }
}

pub struct TariEngineHasher64 {
    hasher: DomainSeparatedBorshHasher<TariEngineHashDomain, Blake2b<U64>>,
}

impl TariEngineHasher64 {
    pub fn new_with_label(label: &'static str) -> Self {
        let hasher = DomainSeparatedBorshHasher::new_with_label(label);
        Self { hasher }
    }

    pub fn update<T: BorshSerialize + ?Sized>(&mut self, data: &T) {
        self.hasher.update_consensus_encode(data)
    }

    pub fn chain<T: BorshSerialize + ?Sized>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    pub fn result(self) -> [u8; 64] {
        self.hasher.finalize().into()
    }
}

#[derive(Debug)]
pub enum EngineHashDomainLabel {
    Template,
    SubstateAddress,
    ConfidentialProof,
    ConfidentialTransfer,
    Transaction,
    NonFungibleId,
    UuidOutput,
    Output,
    EntityId,
    ResourceAddress,
    ComponentAddress,
    TransactionReceipt,
    TransactionIntent,
    FeeClaimAddress,
    QuorumCertificate,
    SubstateValue,
    SubstateValuePart,
    ViewableBalanceProof,
    UtxoAddress,
    ConfidentialOutputAddress,
    StealthBalanceProof,
    StealthBalanceProofAux,
    CovenantBalanceProof,
    ValueProof,
    ElgamalValueProof,
    Blob,
    ConditionLeaf,
    ConditionBranch,
    ShardStateLeaf,
    TransitionAccumulator,
}

impl EngineHashDomainLabel {
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Template => "Template",
            Self::SubstateAddress => "SubstateAddress",
            Self::ConfidentialProof => "ConfidentialProof",
            Self::ConfidentialTransfer => "ConfidentialTransfer",
            Self::Transaction => "Transaction",
            Self::NonFungibleId => "NonFungibleId",
            Self::UuidOutput => "UuidOutput",
            Self::Output => "Output",
            Self::EntityId => "EntityId",
            Self::ResourceAddress => "ResourceAddress",
            Self::ComponentAddress => "ComponentAddress",
            Self::TransactionReceipt => "TransactionReceipt",
            Self::TransactionIntent => "TransactionIntent",
            Self::FeeClaimAddress => "FeeClaimAddress",
            Self::QuorumCertificate => "QuorumCertificate",
            Self::SubstateValue => "SubstateValue",
            Self::SubstateValuePart => "SubstateValuePart",
            Self::ViewableBalanceProof => "ViewableBalanceProof",
            Self::UtxoAddress => "UtxoAddress",
            Self::ConfidentialOutputAddress => "ConfidentialOutputAddress",
            Self::StealthBalanceProof => "StealthBalanceProof",
            Self::StealthBalanceProofAux => "StealthBalanceProofAux",
            Self::CovenantBalanceProof => "CovenantBalanceProof",
            Self::ValueProof => "ValueProof",
            Self::ElgamalValueProof => "ElgamalValueProof",
            Self::Blob => "Blob",
            Self::ConditionLeaf => "ConditionLeaf",
            Self::ConditionBranch => "ConditionBranch",
            Self::ShardStateLeaf => "ShardStateLeaf",
            Self::TransitionAccumulator => "TransitionAccumulator",
        }
    }
}
