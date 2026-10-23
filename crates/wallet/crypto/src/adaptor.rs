//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Schnorr adaptor signatures over Ristretto ("scriptless scripts").
//!
//! An adaptor signature is a Schnorr signature "encrypted" under an adaptor point `T = t·G`: a
//! valid-looking pre-signature anyone can verify, but which only the holder of the discrete log `t`
//! can complete into a full signature. Completing it reveals `t` (recoverable by subtracting the
//! pre-signature scalar from the completed one), which is what makes a cross-chain atomic swap
//! atomic — completing one chain's signature publishes the secret that unlocks the other.
//!
//! The completed signature is an ordinary sr25519 signature ([`Sr25519SignatureBytes`]) whose public
//! nonce is `R + T`, so it verifies as a transaction signature and needs no consensus change. The
//! challenge is the transaction signing challenge under [`TRANSACTION_SIGNING_CONTEXT`], over the
//! **offset** nonce `R + T`, so a completed adaptor signature is byte-for-byte one the standard signer
//! could have produced over that nonce.

use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_common_types::signature::{
    Sr25519SignatureBytes,
    TRANSACTION_SIGNING_CONTEXT,
    components,
    from_components,
};
use tari_utilities::ByteArray;

/// A Schnorr adaptor pre-signature encrypted under an adaptor point `T = t·G`.
///
/// `public_nonce` is the offset nonce `R + T` that the completed signature will carry; `signature`
/// is the pre-signature scalar `ŝ = r + e·x`, which does **not** include `t`. Completing with `t`
/// yields the signature `(R + T, ŝ + t)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RistrettoAdaptorSignature {
    public_nonce: RistrettoPublicKey,
    signature: RistrettoSecretKey,
}

impl RistrettoAdaptorSignature {
    /// The offset public nonce `R + T` the completed signature will carry.
    pub fn public_nonce(&self) -> &RistrettoPublicKey {
        &self.public_nonce
    }

    /// The pre-signature scalar `ŝ = r + e·x`.
    pub fn signature(&self) -> &RistrettoSecretKey {
        &self.signature
    }
}

/// True if `point` is the Ristretto identity, whose canonical encoding is 32 zero bytes. Uses a plain
/// byte comparison rather than the constant-time point `==`, since these are public values.
fn is_identity(point: &RistrettoPublicKey) -> bool {
    point.as_bytes().iter().all(|&b| b == 0)
}

/// Produces an adaptor pre-signature `ŝ = r + e·x` with `e = H(R + T, P, message)`, where
/// `R = r·G` for a freshly sampled nonce `r`, `P = secret·G`, and `T = adaptor_point`. The challenge
/// binds the **offset** nonce `R + T` so [`complete`] yields a valid transaction signature.
///
/// The nonce is drawn internally from the OS CSPRNG on each call, so it cannot be reused across
/// signatures (nonce reuse across two messages would leak `secret`).
pub fn sign(
    secret: &RistrettoSecretKey,
    adaptor_point: &RistrettoPublicKey,
    message: &[u8],
) -> RistrettoAdaptorSignature {
    let nonce = RistrettoSecretKey::random(&mut rand::rng());
    let public_key = RistrettoPublicKey::from_secret_key(secret);
    let r = RistrettoPublicKey::from_secret_key(&nonce);
    let public_nonce = &r + adaptor_point; // R + T
    // The verifier's own challenge is what makes a completed adaptor signature a transaction signature.
    let e = TRANSACTION_SIGNING_CONTEXT.challenge(&public_nonce, &public_key, message);
    let ek = &e * secret;
    let s_hat = &nonce + &ek; // ŝ = r + e·x
    RistrettoAdaptorSignature {
        public_nonce,
        signature: s_hat,
    }
}

/// Verifies an adaptor pre-signature against `public_key`, `adaptor_point` (`T`) and `message`,
/// checking `ŝ·G + T == (R + T) + e·P`. A `true` result guarantees that [`complete`] with
/// the discrete log of `T` produces a valid transaction signature.
pub fn verify(
    pre: &RistrettoAdaptorSignature,
    public_key: &RistrettoPublicKey,
    adaptor_point: &RistrettoPublicKey,
    message: &[u8],
) -> bool {
    // Reject identity points: a zero public key, nonce, or adaptor point yields a degenerate or
    // malformed signature. These are public values, so compare the encoding, not the constant-time
    // point equality.
    if is_identity(public_key) || is_identity(&pre.public_nonce) || is_identity(adaptor_point) {
        return false;
    }
    let e = TRANSACTION_SIGNING_CONTEXT.challenge(&pre.public_nonce, public_key, message);
    let s_hat_g = RistrettoPublicKey::from_secret_key(&pre.signature);
    let lhs = &s_hat_g + adaptor_point;
    let rhs = &pre.public_nonce + &e * public_key;
    lhs.as_bytes() == rhs.as_bytes()
}

/// Completes the pre-signature with the adaptor secret `t` (`T = t·G`), yielding an ordinary
/// signature `(R + T, ŝ + t)` that verifies as a transaction signature.
///
/// The caller is responsible for ensuring `t` is the discrete log of the adaptor point the
/// pre-signature was produced under; verify with [`verify`] first if in doubt.
pub fn complete(pre: &RistrettoAdaptorSignature, t: &RistrettoSecretKey) -> Sr25519SignatureBytes {
    let s = &pre.signature + t;
    from_components(&pre.public_nonce, &s)
}

/// Recovers the adaptor secret `t` from a pre-signature and its completed signature: `t = s − ŝ`.
///
/// This is the swap-reveal primitive: a watcher who holds the pre-signature and observes the
/// completed signature on-chain recovers the secret that unlocks the other leg. Returns `None` when
/// `completed` is not a well-formed signature.
pub fn extract(pre: &RistrettoAdaptorSignature, completed: &Sr25519SignatureBytes) -> Option<RistrettoSecretKey> {
    let (_, s) = components(completed)?;
    Some(&s - &pre.signature)
}

#[cfg(test)]
mod tests {
    use ootle_byte_type::ToByteType;
    use tari_crypto::ristretto::{RistrettoPublicKey, RistrettoSecretKey};

    use super::*;

    fn keypair() -> (RistrettoSecretKey, RistrettoPublicKey) {
        let secret = RistrettoSecretKey::random(&mut rand::rng());
        let public = RistrettoPublicKey::from_secret_key(&secret);
        (secret, public)
    }

    const MESSAGE: &[u8] = b"adaptor signature test message";

    #[test]
    fn complete_yields_a_valid_transaction_signature() {
        let (x, p) = keypair();
        let (t, big_t) = keypair();
        let pre = sign(&x, &big_t, MESSAGE);
        assert!(verify(&pre, &p, &big_t, MESSAGE));

        let sig = complete(&pre, &t);
        // The completed signature is an ordinary transaction signature with no adaptor awareness needed.
        assert!(TRANSACTION_SIGNING_CONTEXT.verify(&sig, &p.to_byte_type(), MESSAGE));
    }

    #[test]
    fn extract_recovers_the_adaptor_secret() {
        let (x, _p) = keypair();
        let (t, big_t) = keypair();
        let pre = sign(&x, &big_t, MESSAGE);
        let sig = complete(&pre, &t);

        assert_eq!(extract(&pre, &sig), Some(t));
    }

    #[test]
    fn completing_with_the_wrong_secret_does_not_verify() {
        let (x, p) = keypair();
        let (_t, big_t) = keypair();
        let (wrong_t, _) = keypair();
        let pre = sign(&x, &big_t, MESSAGE);
        let sig = complete(&pre, &wrong_t);
        assert!(!TRANSACTION_SIGNING_CONTEXT.verify(&sig, &p.to_byte_type(), MESSAGE));
    }

    #[test]
    fn verify_rejects_a_tampered_message_or_wrong_key() {
        let (x, p) = keypair();
        let (_t, big_t) = keypair();
        let (_, other_p) = keypair();
        let pre = sign(&x, &big_t, MESSAGE);
        assert!(!verify(&pre, &p, &big_t, b"different message"));
        assert!(!verify(&pre, &other_p, &big_t, MESSAGE));
    }

    #[test]
    fn verify_rejects_identity_points() {
        let (x, p) = keypair();
        let (_t, big_t) = keypair();
        let identity = RistrettoPublicKey::default();

        let pre = sign(&x, &big_t, MESSAGE);
        assert!(!verify(&pre, &identity, &big_t, MESSAGE), "identity public key");
        assert!(!verify(&pre, &p, &identity, MESSAGE), "identity adaptor point");
    }
}
