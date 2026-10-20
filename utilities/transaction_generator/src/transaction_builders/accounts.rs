//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use rand::{SeedableRng, rngs::StdRng};
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_engine_types::component::derive_component_address_from_public_key;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib_types::ComponentAddress;

/// The keypair of account `index` in the set of accounts derived from `seed`.
///
/// The same `(seed, index)` always gives the same key, so a flood that funds accounts and a later flood that
/// spends from them agree on who owns which account without sharing a key file.
pub fn account_keypair(seed: u64, index: u64) -> (RistrettoSecretKey, RistrettoPublicKey) {
    let mut rng_seed = [0u8; 32];
    rng_seed[..8].copy_from_slice(&seed.to_le_bytes());
    rng_seed[8..16].copy_from_slice(&index.to_le_bytes());
    let secret = RistrettoSecretKey::random(&mut StdRng::from_seed(rng_seed));
    let public = RistrettoPublicKey::from_secret_key(&secret);
    (secret, public)
}

/// The address `CreateAccount` gives the account owned by `public_key`.
pub fn account_address(public_key: &RistrettoPublicKey) -> ComponentAddress {
    derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &public_key.to_byte_type())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stable_per_seed_and_index() {
        assert_eq!(account_keypair(7, 3).1, account_keypair(7, 3).1);
        assert_ne!(account_keypair(7, 3).1, account_keypair(7, 4).1);
        assert_ne!(account_keypair(7, 3).1, account_keypair(8, 3).1);
    }
}
