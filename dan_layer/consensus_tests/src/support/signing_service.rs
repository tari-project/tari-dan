//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey};
use tari_consensus::traits::{ValidatorSignatureService, VoteSignatureService};
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSchnorrSignature, ValidatorSignature};

use super::{helpers, TestAddress};

#[derive(Debug, Clone)]
pub struct TestVoteSignatureService {
    pub public_key: PublicKey,
    pub secret_key: PrivateKey,
    pub is_signature_valid: bool,
}

impl TestVoteSignatureService {
    pub fn new(addr: TestAddress) -> Self {
        let (secret_key, public_key) = helpers::derive_keypair_from_address(&addr);
        Self {
            public_key,
            secret_key,
            is_signature_valid: true,
        }
    }
}

impl ValidatorSignatureService for TestVoteSignatureService {
    fn sign<M: AsRef<[u8]>>(&self, message: M) -> ValidatorSchnorrSignature {
        ValidatorSchnorrSignature::sign(&self.secret_key, message, &mut OsRng).unwrap()
    }

    fn public_key(&self) -> &PublicKey {
        &self.public_key
    }
}

impl VoteSignatureService for TestVoteSignatureService {
    fn verify(
        &self,
        _signature: &ValidatorSignature,
        _leaf_hash: &FixedHash,
        _block_id: &BlockId,
        _decision: &QuorumDecision,
    ) -> bool {
        self.is_signature_valid
    }
}
