//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub trait SigningService {
    fn sign(&self, challenge: &[u8]) -> Option<Signature>;
    fn verify(&self, signature: &Signature, challenge: &[u8]) -> bool;
    fn verify_for_public_key(&self, public_key: &PublicKey, signature: &Signature, challenge: &[u8]) -> bool;
    fn public_key(&self) -> &CommsPublicKey;
}
