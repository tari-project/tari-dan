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

mod builder;
pub use builder::TransactionBuilder;

mod error;

mod processor;
pub use processor::InstructionProcessor;

mod signature;
use std::collections::HashMap;

use digest::{Digest, FixedOutput};
pub use signature::InstructionSignature;
use tari_common_types::types::{BulletRangeProof, ComSignature, Commitment, FixedHash, PublicKey};
use tari_crypto::hash::blake2::Blake256;
use tari_dan_common_types::{ObjectClaim, ObjectId, ShardId, SubstateChange};
use tari_mmr::MerkleProof;
use tari_template_lib::{
    args::Arg,
    models::{ComponentAddress, PackageAddress},
};
use tari_utilities::ByteArray;

#[derive(Debug, Clone)]
pub enum Instruction {
    CallFunction {
        package_address: PackageAddress,
        template: String,
        function: String,
        args: Vec<Arg>,
    },
    CallMethod {
        package_address: PackageAddress,
        component_address: ComponentAddress,
        method: String,
        args: Vec<Arg>,
    },
    PutLastInstructionOutputOnWorkspace {
        key: Vec<u8>,
    },
}

impl Instruction {
    pub fn hash(&self) -> FixedHash {
        // TODO: put in actual hashes
        match self {
            Instruction::CallFunction { .. } => FixedHash::zero(),
            Instruction::CallMethod { .. } => FixedHash::zero(),
            Instruction::PutLastInstructionOutputOnWorkspace { .. } => FixedHash::zero(),
        }
    }
}

// FIXME: fix clippy
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum ThaumInput {
    Standard {
        object_id: ObjectId,
    },
    PegIn {
        commitment: Commitment,
        burn_proof: MerkleProof,
        spending_key: StealthAddress,
        owner_proof: ComSignature,
    },
}

#[derive(Debug, Clone)]
pub struct ThaumOutput {
    _commitment: Commitment,
    _owner: StealthAddress,
    _rangeproof: BulletRangeProof,
}

#[derive(Debug, Clone)]
pub struct StealthAddress {
    _nonce: PublicKey,
    _address: PublicKey,
}

#[derive(Debug, Clone)]
pub struct BalanceProof {}

#[derive(Debug, Clone)]
pub struct Transaction {
    hash: [u8; 32],
    instructions: Vec<Instruction>,
    signature: InstructionSignature,
    _fee: u64,
    sender_public_key: PublicKey,
    // Not part of signature. TODO: Should it be?
    meta: Option<TransactionMeta>,
}

#[derive(Debug, Clone, Default)]
pub struct TransactionMeta {
    involved_objects: HashMap<ShardId, Vec<(ObjectId, SubstateChange, ObjectClaim)>>,
}

impl TransactionMeta {
    pub fn involved_shards(&self) -> Vec<ShardId> {
        self.involved_objects.keys().copied().collect()
    }

    pub fn objects_for_shard(&self, shard_id: ShardId) -> Vec<(ObjectId, SubstateChange, ObjectClaim)> {
        self.involved_objects.get(&shard_id).cloned().unwrap_or_default()
    }
}

impl Transaction {
    pub fn new(
        fee: u64,
        instructions: Vec<Instruction>,
        signature: InstructionSignature,
        sender_public_key: PublicKey,
        meta: TransactionMeta,
    ) -> Self {
        let mut s = Self {
            hash: [0u8; 32],
            instructions,
            signature,
            _fee: fee,
            sender_public_key,
            meta: Some(meta),
        };
        s.calculate_hash();
        s
    }

    pub fn hash(&self) -> &[u8; 32] {
        &self.hash
    }

    pub fn meta(&self) -> &TransactionMeta {
        self.meta.as_ref().unwrap()
    }

    fn calculate_hash(&mut self) {
        let mut res = Blake256::new()
            .chain(self.sender_public_key.as_bytes())
            .chain(self.signature.signature().get_public_nonce().as_bytes())
            .chain(self.signature.signature().get_signature().as_bytes());
        for instruction in &self.instructions {
            res = res.chain(instruction.hash())
        }
        self.hash = res.finalize_fixed().into();
    }

    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    pub fn signature(&self) -> &InstructionSignature {
        &self.signature
    }

    pub fn sender_public_key(&self) -> &PublicKey {
        &self.sender_public_key
    }

    pub fn destruct(self) -> (Vec<Instruction>, InstructionSignature, PublicKey) {
        (self.instructions, self.signature, self.sender_public_key)
    }
}
