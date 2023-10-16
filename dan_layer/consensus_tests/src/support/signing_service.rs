//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use rand::rngs::OsRng;
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey};
use tari_consensus::traits::{ValidatorSignatureService, VoteSignatureService};
use tari_crypto::keys::PublicKey as _;
use tari_dan_common_types::NodeAddressable;
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSchnorrSignature, ValidatorSignature};

#[derive(Debug, Clone)]
pub struct TestVoteSignatureService<TAddr> {
    pub public_key: TAddr,
    pub secret_key: PrivateKey,
    pub is_signature_valid: bool,
}

impl<TAddr> TestVoteSignatureService<TAddr> {
    pub fn new(public_key: TAddr) -> Self {
        let (secret_key, _public_key) = PublicKey::random_keypair(&mut OsRng);
        Self {
            public_key,
            secret_key,
            is_signature_valid: true,
        }
    }
}

impl<TAddr> ValidatorSignatureService<TAddr> for TestVoteSignatureService<TAddr> {
    fn sign<M: AsRef<[u8]>>(&self, message: M) -> ValidatorSchnorrSignature {
        ValidatorSchnorrSignature::sign_message(&self.secret_key, message, &mut OsRng).unwrap()
    }

    fn public_key(&self) -> &TAddr {
        &self.public_key
    }
}

impl<TAddr: NodeAddressable> VoteSignatureService<TAddr> for TestVoteSignatureService<TAddr> {
    fn verify(
        &self,
        _signature: &ValidatorSignature<TAddr>,
        _leaf_hash: &FixedHash,
        _block_id: &BlockId,
        _decision: &QuorumDecision,
    ) -> bool {
        self.is_signature_valid
    }
}
