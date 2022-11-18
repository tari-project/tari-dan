//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use rand::rngs::OsRng;
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey, Signature};
use tari_core::{consensus::DomainSeparatedConsensusHasher, transactions::TransactionHashDomain};
use tari_crypto::keys::PublicKey as PublicKeyT;

// TODO: Find a neat way to encapsulated this signature so that it can be used by the validator node and the base layer
// signature validation
pub fn sign_validator_node_registration(private_key: &PrivateKey, _epoch: u64) -> Signature {
    let (secret_nonce, public_nonce) = PublicKey::random_keypair(&mut OsRng);
    let public_key = PublicKey::from_secret_key(private_key);
    // TODO: epoch should be committed to, but this is currently not the case on the base node, so we leave it out for
    //       now so that the transaction passes validation.
    let challenge = construct_challenge(&public_key, &public_nonce, b"");
    Signature::sign(private_key.clone(), secret_nonce, &*challenge)
        .expect("Sign cannot fail with 32-byte challenge and a RistrettoPublicKey")
}

fn construct_challenge(public_key: &PublicKey, public_nonce: &PublicKey, msg: &[u8]) -> FixedHash {
    DomainSeparatedConsensusHasher::<TransactionHashDomain>::new("validator_node_registration")
        .chain(public_key)
        .chain(public_nonce)
        .chain(&msg)
        .finalize()
        .into()
}
