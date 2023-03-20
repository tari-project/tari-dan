// Copyright 2021. The Tari Project
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

use std::sync::Arc;

use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::{
    types::{CommsPublicKey, Signature},
    NodeIdentity,
};
use tari_dan_common_types::{crypto::create_key_pair, hashing::tari_hasher};
use tari_utilities::ByteArray;

use crate::TariDanCoreHashDomain;

pub trait SigningService {
    fn sign(&self, challenge: &[u8]) -> Option<Signature>;
    fn verify(&self, signature: &Signature, challenge: &[u8]) -> bool;
    fn verify_for_public_key(&self, public_key: &PublicKey, signature: &Signature, challenge: &[u8]) -> bool;
    fn public_key(&self) -> &CommsPublicKey;
}

#[derive(Debug, Clone)]
pub struct NodeIdentitySigningService {
    node_identity: Arc<NodeIdentity>,
}

impl NodeIdentitySigningService {
    pub fn new(node_identity: Arc<NodeIdentity>) -> Self {
        Self { node_identity }
    }

    fn create_challenge(public_key: &PublicKey, public_nonce: &PublicKey, challenge: &[u8]) -> FixedHash {
        tari_hasher::<TariDanCoreHashDomain>("signing_service")
            .chain(&key_to_fixed_bytes(public_key))
            .chain(&key_to_fixed_bytes(public_nonce))
            .chain(challenge)
            .result()
    }
}

impl SigningService for NodeIdentitySigningService {
    fn sign(&self, challenge: &[u8]) -> Option<Signature> {
        let (nonce, public_nonce) = create_key_pair();
        let challenge = Self::create_challenge(&public_nonce, self.node_identity.public_key(), challenge);
        let sig = Signature::sign_raw(self.node_identity.secret_key(), nonce, &*challenge).ok()?;
        Some(sig)
    }

    fn verify(&self, signature: &Signature, challenge: &[u8]) -> bool {
        self.verify_for_public_key(self.node_identity.public_key(), signature, challenge)
    }

    fn verify_for_public_key(&self, public_key: &PublicKey, signature: &Signature, challenge: &[u8]) -> bool {
        let challenge = Self::create_challenge(signature.get_public_nonce(), public_key, challenge);
        signature.verify_challenge(public_key, &*challenge)
    }

    fn public_key(&self) -> &CommsPublicKey {
        self.node_identity.public_key()
    }
}

// TODO: this wont be required when CBOR support is added to tari crypto
fn key_to_fixed_bytes(key: &CommsPublicKey) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf.copy_from_slice(key.as_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use tari_comms::{peer_manager::PeerFeatures, test_utils::node_identity::build_node_identity};

    use super::*;

    #[test]
    fn basic() {
        let node_identity = build_node_identity(PeerFeatures::COMMUNICATION_NODE);
        let signing_service = NodeIdentitySigningService::new(node_identity);
        let challenge = b"challenge";
        let signature = signing_service.sign(challenge).unwrap();
        assert!(signing_service.verify(&signature, challenge));
    }
}
