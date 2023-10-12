//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use rand::rngs::OsRng;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::NodeIdentity;
use tari_consensus::traits::{ValidatorSignatureService, VoteSignatureService};
use tari_dan_storage::consensus_models::{BlockId, QuorumDecision, ValidatorSchnorrSignature, ValidatorSignature};

#[derive(Debug, Clone)]
pub struct TariSignatureService {
    node_identity: Arc<NodeIdentity>,
}

impl TariSignatureService {
    pub fn new(node_identity: Arc<NodeIdentity>) -> Self {
        Self { node_identity }
    }
}

impl ValidatorSignatureService<PublicKey> for TariSignatureService {
    fn sign<M: AsRef<[u8]>>(&self, message: M) -> ValidatorSchnorrSignature {
        ValidatorSchnorrSignature::sign_message(self.node_identity.secret_key(), message, &mut OsRng).unwrap()
    }

    fn public_key(&self) -> &PublicKey {
        self.node_identity.public_key()
    }
}

impl VoteSignatureService<PublicKey> for TariSignatureService {
    fn verify(
        &self,
        signature: &ValidatorSignature<PublicKey>,
        leaf_hash: &FixedHash,
        block_id: &BlockId,
        decision: &QuorumDecision,
    ) -> bool {
        let challenge = self.create_challenge(leaf_hash, block_id, decision);
        signature.verify(challenge)
    }
}
