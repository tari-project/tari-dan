// Copyright 2024. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use rand::rngs::OsRng;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_consensus::traits::{ValidatorSignatureService, VoteSignatureService};
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSchnorrSignature, ValidatorSignature};

use crate::keypair::RistrettoKeypair;

#[derive(Debug, Clone)]
pub struct TariSignatureService {
    keypair: RistrettoKeypair,
}

impl TariSignatureService {
    pub fn new(keypair: RistrettoKeypair) -> Self {
        Self { keypair }
    }
}

impl ValidatorSignatureService for TariSignatureService {
    fn sign<M: AsRef<[u8]>>(&self, message: M) -> ValidatorSchnorrSignature {
        ValidatorSchnorrSignature::sign(self.keypair.secret_key(), message, &mut OsRng).unwrap()
    }

    fn public_key(&self) -> &PublicKey {
        self.keypair.public_key()
    }
}

impl VoteSignatureService for TariSignatureService {
    fn verify(
        &self,
        signature: &ValidatorSignature,
        leaf_hash: &FixedHash,
        block_id: &BlockId,
        decision: &QuorumDecision,
    ) -> bool {
        let challenge = self.create_challenge(leaf_hash, block_id, decision);
        signature.verify(challenge)
    }
}
