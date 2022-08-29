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
use digest::{Digest, FixedOutput};

mod error;

mod processor;
pub use processor::InstructionProcessor;

mod signature;
pub use signature::InstructionSignature;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_crypto::hash::blake2::Blake256;
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

#[derive(Debug, Clone)]
pub struct Transaction {
    hash: FixedHash,
    instructions: Vec<Instruction>,
    signature: InstructionSignature,
    sender_public_key: PublicKey,
}

impl Transaction {
    pub fn new(instructions: Vec<Instruction>, signature: InstructionSignature, sender_public_key: PublicKey) -> Self {
        let mut s = Self {
            hash: FixedHash::zero(),
            instructions,
            signature,
            sender_public_key,
        };
        s.calculate_hash();
        s
    }

    pub fn hash(&self) -> &FixedHash {
        &self.hash
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
