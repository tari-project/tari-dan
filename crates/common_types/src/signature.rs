//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Transaction signatures.
//!
//! Transaction authorization and seal signatures are standard sr25519 signatures
//! ([`Sr25519SignatureBytes`]) under [`TRANSACTION_SIGNING_CONTEXT`]. sr25519 keys are Ristretto255
//! keys, so the keys tari_crypto manages sign directly, and off-the-shelf sr25519 signers (HSMs, MPC
//! vaults, Polkadot wallets) produce signatures that verify here unchanged.

use tari_crypto::ristretto::RistrettoPublicKey;
pub use tari_ootle_sr25519::{SigningContext, Sr25519SignatureBytes, components, from_components};

/// The sr25519 signing context of every transaction authorization and seal signature.
///
/// This is schnorrkel's default context, the only one many stock sr25519 signers support. Separation
/// from Substrate chains comes from the message: a transaction signing message is a 64-byte
/// domain-separated hash, and no Substrate signing payload equals one.
pub const TRANSACTION_SIGNING_CONTEXT: SigningContext<'static> = SigningContext::new(b"substrate");

#[derive(Debug, Clone)]
pub struct SignatureOutput {
    pub signature: Sr25519SignatureBytes,
    pub public_key: RistrettoPublicKey,
}

#[cfg(test)]
mod tests {
    use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

    use super::*;

    /// A stock sr25519 signer's signature verifies as a transaction signature: this one is from
    /// `@scure/sr25519` 2.4.0 under its default context, over the 64-byte message `0x00..0x3f`.
    #[test]
    fn a_stock_sr25519_signature_verifies_under_the_transaction_context() {
        let message = std::array::from_fn::<u8, 64, _>(|i| i as u8);
        let public_key =
            RistrettoPublicKeyBytes::from_hex("ccddafdb8188e3edb0984780053ac7f357f1741c59292884d9fe3e5832d2f90f")
                .unwrap();
        let signature: Sr25519SignatureBytes =
            "d8280e79fe66ee85d9f20cdbfccff261bbb7401140d098a54ec1b046bdfd670a50fa8e16adbe36b86fc1ac6e4c86921eae2817b5f86fdec12424d482077abc84"
                .parse()
                .unwrap();

        assert!(TRANSACTION_SIGNING_CONTEXT.verify(&signature, &public_key, &message));
    }
}
