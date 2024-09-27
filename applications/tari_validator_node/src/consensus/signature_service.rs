//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::PublicKey;
use tari_consensus::traits::{ValidatorSignatureService, VoteSignatureService};
use tari_dan_app_utilities::keypair::RistrettoKeypair;
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSchnorrSignature, ValidatorSignature};

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
    fn verify(&self, signature: &ValidatorSignature, block_id: &BlockId, decision: &QuorumDecision) -> bool {
        let message = self.create_message(block_id, decision);
        signature.verify(message)
    }
}
