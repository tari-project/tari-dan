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

use std::convert::TryFrom;

use serde::Deserialize;
use tari_common_types::types::{PrivateKey, PublicKey, Signature};
use tari_crypto::keys::PublicKey as PublicKeyT;
use tari_utilities::ByteArray;

use crate::{crypto::create_key_pair, hashing::hasher, transaction::Instruction};

#[derive(Debug, Clone, Deserialize)]
pub struct InstructionSignature(Signature);

impl InstructionSignature {
    pub fn sign(secret_key: &PrivateKey, instructions: &[Instruction]) -> Self {
        let public_key = PublicKey::from_secret_key(secret_key);
        let (nonce, nonce_pk) = create_key_pair();
        // TODO: implement dan encoding for (a wrapper of) PublicKey
        let challenge = hasher("instruction-signature")
            .chain(nonce_pk.as_bytes())
            .chain(public_key.as_bytes())
            .chain(instructions)
            .result();
        Self(Signature::sign(secret_key.clone(), nonce, &challenge).unwrap())
    }

    pub fn signature(&self) -> Signature {
        self.0.clone()
    }
}

impl TryFrom<Signature> for InstructionSignature {
    type Error = String;

    fn try_from(sig: Signature) -> Result<Self, Self::Error> {
        Ok(InstructionSignature(sig))
    }
}
