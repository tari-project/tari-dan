//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! sr25519 signatures over tari_crypto Ristretto keys.
//!
//! sr25519 is Schnorr over Ristretto255, `s·G = R + e·P`, with the challenge `e` drawn from a Merlin
//! transcript that commits to a caller-chosen signing context. Its keys are Ristretto255 keys, so the
//! keys tari_crypto manages sign directly, and signatures from any standard sr25519 signer (Polkadot
//! tooling, HSMs, MPC vaults) verify here.
//!
//! This crate implements signing and verification only, byte-compatible with schnorrkel, the
//! reference implementation. Signatures travel as [`Sr25519SignatureBytes`], the standard 64-byte
//! encoding. The tests check signatures in both directions against schnorrkel and against a vector
//! from `@scure/sr25519`, an independent implementation.
//!
//! Where it is stricter than schnorrkel:
//! - the identity public key is rejected, since anyone can forge a signature under it;
//! - deterministic batch verification draws its weights from a transcript that covers every signature's `s` (see
//!   [`SigningContext::verify_batch`]);
//! - an empty batch is rejected.

use curve25519_dalek::{
    RistrettoPoint,
    Scalar,
    constants::RISTRETTO_BASEPOINT_POINT,
    traits::{IsIdentity, VartimeMultiscalarMul},
};
use merlin::Transcript;
use rand::Rng;
use tari_crypto::{
    keys::{PublicKey as _, SecretKey as _},
    ristretto::{RistrettoPublicKey, RistrettoSchnorr, RistrettoSecretKey},
    tari_utilities::ByteArray,
};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;
pub use tari_template_lib_types::crypto::Sr25519SignatureBytes;
use zeroize::Zeroizing;

/// The marker bit sr25519 sets in the last byte of its 64-byte signature encoding.
const SR25519_MARKER: u8 = 0x80;

/// An sr25519 signing context: the label every signature made or checked through it commits to, so a
/// signature made under one context never verifies under another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigningContext<'a> {
    context: &'a [u8],
}

impl<'a> SigningContext<'a> {
    pub const fn new(context: &'a [u8]) -> Self {
        Self { context }
    }

    pub const fn as_bytes(&self) -> &'a [u8] {
        self.context
    }

    /// Signs `message` with `secret_key`.
    pub fn sign(&self, secret_key: &RistrettoSecretKey, message: &[u8]) -> Sr25519SignatureBytes {
        let public_key = RistrettoPublicKey::from_secret_key(secret_key);
        let nonce = self.signing_nonce(secret_key, message);
        let public_nonce = RistrettoPublicKey::from_secret_key(&nonce);
        let e = self.challenge(&public_nonce, &public_key, message);
        let s = &nonce + &(&e * secret_key);
        from_components(&public_nonce, &s)
    }

    /// True when `signature` is `public_key`'s signature over `message` under this context. The
    /// identity key is rejected.
    pub fn verify(
        &self,
        signature: &Sr25519SignatureBytes,
        public_key: &RistrettoPublicKeyBytes,
        message: &[u8],
    ) -> bool {
        let Some((public_nonce, s, public_key)) = decode(signature, public_key) else {
            return false;
        };
        let e = self.challenge(&public_nonce, &public_key, message);
        RistrettoSchnorr::new(public_nonce, s).verify_challenge_scalar(&public_key, &e)
    }

    /// True when every `(signature, public key, message)` item passes [`Self::verify`].
    ///
    /// Checks the single equation `Σ zᵢ·Rᵢ + Σ (zᵢ·eᵢ)·Pᵢ − (Σ zᵢ·sᵢ)·G == 0` as one variable-time
    /// multiscalar multiplication. The weights `zᵢ` stop individually invalid signatures from
    /// cancelling one another out. They are drawn from a transcript of the whole batch rather than
    /// local randomness, so every caller reaches the same verdict on the same bytes, which a validity
    /// predicate a committee votes on requires. The transcript covers every signature's `s`: weights
    /// a signer can compute without it let two invalid `s` values be chosen so their errors cancel.
    /// Variable time is safe because every input is public.
    ///
    /// A rejection does not say which item is at fault. An empty batch is rejected: a caller for whom
    /// no signatures is acceptable must say so before calling.
    pub fn verify_batch<B: AsRef<[u8]>>(
        &self,
        items: &[(&Sr25519SignatureBytes, &RistrettoPublicKeyBytes, B)],
    ) -> bool {
        if items.is_empty() {
            return false;
        }
        let weights = self.batch_weights(items);

        let mut scalars = Vec::with_capacity(2 * items.len() + 1);
        let mut points = Vec::with_capacity(2 * items.len() + 1);
        let mut signature_sum = Scalar::ZERO;

        for ((signature, public_key, message), weight) in items.iter().zip(weights) {
            let Some((public_nonce, s, public_key)) = decode(signature, public_key) else {
                return false;
            };
            let Some(s) = to_scalar(&s) else {
                return false;
            };
            let e =
                Scalar::from_bytes_mod_order_wide(&self.challenge_bytes(&public_nonce, &public_key, message.as_ref()));

            signature_sum += weight * s;
            scalars.push(weight);
            points.push(public_nonce.point());
            scalars.push(weight * e);
            points.push(public_key.point());
        }

        scalars.push(-signature_sum);
        points.push(RISTRETTO_BASEPOINT_POINT);

        RistrettoPoint::vartime_multiscalar_mul(&scalars, &points).is_identity()
    }

    /// The challenge scalar `e` of a signature with public nonce `public_nonce` by `public_key` over
    /// `message`, for signers that compute `s = r + e·k` themselves (e.g. adaptor signatures).
    pub fn challenge(
        &self,
        public_nonce: &RistrettoPublicKey,
        public_key: &RistrettoPublicKey,
        message: &[u8],
    ) -> RistrettoSecretKey {
        RistrettoSecretKey::from_uniform_bytes(&self.challenge_bytes(public_nonce, public_key, message))
            .expect("the challenge is 64 bytes")
    }

    /// The 64 challenge bytes, wide-reduced into `e`. The transcript is schnorrkel's
    /// `signing_context(ctx).bytes(message)` followed by its `Schnorr-sig` protocol labels, which
    /// commit to the public key before the public nonce.
    fn challenge_bytes(
        &self,
        public_nonce: &RistrettoPublicKey,
        public_key: &RistrettoPublicKey,
        message: &[u8],
    ) -> [u8; 64] {
        let mut transcript = Transcript::new(b"SigningContext");
        transcript.append_message(b"", self.context);
        transcript.append_message(b"sign-bytes", message);
        transcript.append_message(b"proto-name", b"Schnorr-sig");
        transcript.append_message(b"sign:pk", public_key.as_bytes());
        transcript.append_message(b"sign:R", public_nonce.as_bytes());
        let mut challenge = [0u8; 64];
        transcript.challenge_bytes(b"sign:c", &mut challenge);
        challenge
    }

    /// A hedged signing nonce, derived from the context, the secret key, the message and fresh
    /// randomness. With a working RNG it is uniformly random; with a failed or repeating one it still
    /// differs across messages, so a weak RNG alone cannot make one key sign two messages under the
    /// same nonce.
    fn signing_nonce(&self, secret_key: &RistrettoSecretKey, message: &[u8]) -> RistrettoSecretKey {
        let mut randomness = Zeroizing::new([0u8; 32]);
        rand::rng().fill_bytes(randomness.as_mut());
        let mut transcript = Transcript::new(b"tari-ootle-sr25519-signing-nonce");
        transcript.append_message(b"context", self.context);
        transcript.append_message(b"secret", secret_key.as_bytes());
        transcript.append_message(b"message", message);
        transcript.append_message(b"randomness", randomness.as_ref());
        let mut wide = Zeroizing::new([0u8; 64]);
        transcript.challenge_bytes(b"nonce", wide.as_mut());
        RistrettoSecretKey::from_uniform_bytes(wide.as_ref()).expect("the nonce is 64 bytes")
    }

    /// One non-zero 128-bit weight per item, drawn from a transcript committing to the context, the
    /// item count and every message, public key and full signature `R ‖ s`, in order.
    fn batch_weights<B: AsRef<[u8]>>(
        &self,
        items: &[(&Sr25519SignatureBytes, &RistrettoPublicKeyBytes, B)],
    ) -> Vec<Scalar> {
        let mut transcript = Transcript::new(b"tari-ootle-sr25519-batch-verify");
        transcript.append_message(b"context", self.context);
        transcript.append_u64(b"n", items.len() as u64);
        for (signature, public_key, message) in items {
            transcript.append_message(b"m", message.as_ref());
            transcript.append_message(b"P", public_key.as_bytes());
            transcript.append_message(b"sig", signature.as_bytes());
        }
        (0..items.len())
            .map(|_| {
                let mut bytes = [0u8; 16];
                transcript.challenge_bytes(b"z", &mut bytes);
                // The forced top bit keeps a weight from being zero, which would drop its term from the equation.
                Scalar::from(u128::from_le_bytes(bytes) | (1 << 127))
            })
            .collect()
    }
}

/// Builds a signature from its public nonce `R` and scalar `s`, for signers that compute the Schnorr
/// equation themselves (adaptor signatures, hardware signers that return the two halves).
pub fn from_components(public_nonce: &RistrettoPublicKey, s: &RistrettoSecretKey) -> Sr25519SignatureBytes {
    let mut bytes = [0u8; Sr25519SignatureBytes::LENGTH];
    bytes[..32].copy_from_slice(public_nonce.as_bytes());
    bytes[32..].copy_from_slice(s.as_bytes());
    bytes[63] |= SR25519_MARKER;
    Sr25519SignatureBytes::from_array(bytes)
}

/// The public nonce `R` and scalar `s` of a signature, the inverse of [`from_components`]. Returns
/// `None` when the marker bit is absent or either component is not canonically encoded.
pub fn components(signature: &Sr25519SignatureBytes) -> Option<(RistrettoPublicKey, RistrettoSecretKey)> {
    let bytes = signature.as_array();
    if bytes[63] & SR25519_MARKER == 0 {
        return None;
    }
    let public_nonce = RistrettoPublicKey::from_canonical_bytes(&bytes[..32]).ok()?;
    let mut s = [0u8; 32];
    s.copy_from_slice(&bytes[32..]);
    s[31] &= !SR25519_MARKER;
    let s = RistrettoSecretKey::from_canonical_bytes(&s).ok()?;
    Some((public_nonce, s))
}

fn decode(
    signature: &Sr25519SignatureBytes,
    public_key: &RistrettoPublicKeyBytes,
) -> Option<(RistrettoPublicKey, RistrettoSecretKey, RistrettoPublicKey)> {
    let public_key = RistrettoPublicKey::from_canonical_bytes(public_key.as_bytes()).ok()?;
    // Under the identity key the equation degenerates to `s·G == R`, which anyone can satisfy.
    if public_key.point().is_identity() {
        return None;
    }
    let (public_nonce, s) = components(signature)?;
    Some((public_nonce, s, public_key))
}

fn to_scalar(secret: &RistrettoSecretKey) -> Option<Scalar> {
    let bytes: [u8; 32] = secret.as_bytes().try_into().ok()?;
    Scalar::from_canonical_bytes(bytes).into_option()
}

#[cfg(test)]
mod tests {
    use schnorrkel::{ExpansionMode, MiniSecretKey, PublicKey, Signature, context::SigningTranscript, signing_context};

    use super::*;

    const CONTEXT: SigningContext<'static> = SigningContext::new(b"tari-ootle-sr25519-tests");

    fn message() -> [u8; 64] {
        let mut message = [0u8; 64];
        message.iter_mut().enumerate().for_each(|(i, b)| *b = i as u8);
        message
    }

    fn keypair() -> (RistrettoSecretKey, RistrettoPublicKeyBytes) {
        let secret = RistrettoSecretKey::random(&mut rand::rng());
        let public_key = RistrettoPublicKey::from_secret_key(&secret);
        (
            secret,
            RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes()).unwrap(),
        )
    }

    fn sr25519_keypair(seed: u8) -> (schnorrkel::Keypair, RistrettoPublicKeyBytes) {
        let keypair = MiniSecretKey::from_bytes(&[seed; 32])
            .unwrap()
            .expand_to_keypair(ExpansionMode::Ed25519);
        let public_key = RistrettoPublicKeyBytes::from(keypair.public.to_bytes());
        (keypair, public_key)
    }

    #[test]
    fn verifies_a_schnorrkel_signature() {
        let (keypair, public_key) = sr25519_keypair(1);
        let signature = keypair.sign(signing_context(CONTEXT.as_bytes()).bytes(&message()));
        let signature = Sr25519SignatureBytes::from_array(signature.to_bytes());

        assert!(CONTEXT.verify(&signature, &public_key, &message()));
        assert!(CONTEXT.verify_batch(&[(&signature, &public_key, message())]));
    }

    /// A signature from `@scure/sr25519` 2.4.0, an implementation independent of schnorrkel, over the
    /// test message under the `substrate` context: seed `0x42..0x61`, signing randomness `[7; 32]`.
    #[test]
    fn verifies_a_scure_sr25519_signature() {
        let context = SigningContext::new(b"substrate");
        let public_key =
            RistrettoPublicKeyBytes::from_hex("ccddafdb8188e3edb0984780053ac7f357f1741c59292884d9fe3e5832d2f90f")
                .unwrap();
        let signature: Sr25519SignatureBytes =
            "d8280e79fe66ee85d9f20cdbfccff261bbb7401140d098a54ec1b046bdfd670a50fa8e16adbe36b86fc1ac6e4c86921eae2817b5f86fdec12424d482077abc84"
                .parse()
                .unwrap();

        assert!(context.verify(&signature, &public_key, &message()));
        assert!(context.verify_batch(&[(&signature, &public_key, message())]));
        let mut other = message();
        other[0] ^= 1;
        assert!(!context.verify(&signature, &public_key, &other));
        assert!(!CONTEXT.verify(&signature, &public_key, &message()));
    }

    #[test]
    fn rejects_a_signature_under_another_context() {
        let (keypair, public_key) = sr25519_keypair(1);
        let signature = keypair.sign(signing_context(b"another-context").bytes(&message()));
        let signature = Sr25519SignatureBytes::from_array(signature.to_bytes());

        assert!(!CONTEXT.verify(&signature, &public_key, &message()));
    }

    #[test]
    fn schnorrkel_verifies_a_signature() {
        let (secret, public_key) = keypair();
        let signature = CONTEXT.sign(&secret, &message());

        let sr_public_key = PublicKey::from_bytes(public_key.as_bytes()).unwrap();
        let sr_signature = Signature::from_bytes(signature.as_bytes()).unwrap();
        sr_public_key
            .verify(signing_context(CONTEXT.as_bytes()).bytes(&message()), &sr_signature)
            .unwrap();
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let (secret, public_key) = keypair();
        let signature = CONTEXT.sign(&secret, &message());

        assert!(CONTEXT.verify(&signature, &public_key, &message()));
        let mut other = message();
        other[0] ^= 1;
        assert!(!CONTEXT.verify(&signature, &public_key, &other));
    }

    #[test]
    fn components_match_the_challenge() {
        let (secret, public_key) = keypair();
        let nonce = RistrettoSecretKey::random(&mut rand::rng());
        let public_nonce = RistrettoPublicKey::from_secret_key(&nonce);
        let e = CONTEXT.challenge(&public_nonce, &RistrettoPublicKey::from_secret_key(&secret), &message());
        let s = nonce + e * &secret;
        let signature = from_components(&public_nonce, &s);

        assert!(CONTEXT.verify(&signature, &public_key, &message()));
        assert_eq!(components(&signature), Some((public_nonce, s)));
    }

    #[test]
    fn batch_rejects_any_invalid_item() {
        let keys = (0..4).map(|_| keypair()).collect::<Vec<_>>();
        let signatures = keys
            .iter()
            .map(|(secret, _)| CONTEXT.sign(secret, &message()))
            .collect::<Vec<_>>();
        let items = signatures
            .iter()
            .zip(&keys)
            .map(|(signature, (_, public_key))| (signature, public_key, message()))
            .collect::<Vec<_>>();
        assert!(CONTEXT.verify_batch(&items));
        assert!(!SigningContext::new(b"another-context").verify_batch(&items));

        let mut other = message();
        other[0] ^= 1;
        let mut tampered = items.clone();
        tampered[2].2 = other;
        assert!(!CONTEXT.verify_batch(&tampered));

        let mut swapped = items.clone();
        swapped[1].1 = &keys[0].1;
        assert!(!CONTEXT.verify_batch(&swapped));
    }

    /// The weights schnorrkel's `verify_batch_deterministic` derives for `items`, reproducing
    /// `prepare_batch` in schnorrkel 0.11.5: a transcript over every key, every nonce `R` and 16
    /// witness bytes per message, read through an all-zero RNG. Nothing in it depends on `s`.
    fn schnorrkel_deterministic_weights(
        items: &[(Sr25519SignatureBytes, RistrettoPublicKeyBytes)],
    ) -> Vec<RistrettoSecretKey> {
        struct ZeroRng;
        impl rand_core_06::RngCore for ZeroRng {
            fn next_u32(&mut self) -> u32 {
                0
            }

            fn next_u64(&mut self) -> u64 {
                0
            }

            fn fill_bytes(&mut self, dest: &mut [u8]) {
                dest.fill(0);
            }

            fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core_06::Error> {
                dest.fill(0);
                Ok(())
            }
        }
        impl rand_core_06::CryptoRng for ZeroRng {}

        let mut weights = Transcript::new(b"V-RNG");
        for (_, public_key) in items {
            weights.append_message(b"", public_key.as_bytes());
        }
        for (signature, _) in items {
            weights.append_message(b"", &signature.as_bytes()[..32]);
        }
        for _ in items {
            let transcript = signing_context(CONTEXT.as_bytes()).bytes(&message());
            let mut witness = [0u8; 16];
            transcript.witness_bytes_rng(b"", &mut witness, &[&[]], ZeroRng);
            weights.append_message(b"", &witness);
        }
        let mut rng = weights.build_rng().finalize(&mut ZeroRng);
        items
            .iter()
            .map(|_| {
                let mut z = [0u8; 32];
                rand_core_06::RngCore::fill_bytes(&mut rng, &mut z[..16]);
                RistrettoSecretKey::from_canonical_bytes(&z).unwrap()
            })
            .collect()
    }

    /// The attack the weights defend against. When the weights do not depend on `s`, a signer who
    /// can compute them shifts two valid signatures' scalars by `+z₂·δ` and `-z₁·δ`: each signature
    /// is then invalid, but the weighted errors `z₁·z₂·δ - z₂·z₁·δ` cancel and the batch balances.
    /// schnorrkel's deterministic batch accepts such a pair; this one must not.
    #[test]
    fn batch_rejects_offsetting_scalars() {
        let pair = [keypair(), keypair()];
        let valid = pair
            .iter()
            .map(|(secret, public_key)| (CONTEXT.sign(secret, &message()), *public_key))
            .collect::<Vec<_>>();
        let weights = schnorrkel_deterministic_weights(&valid);
        let delta = RistrettoSecretKey::random(&mut rand::rng());

        let (nonce_0, s_0) = components(&valid[0].0).unwrap();
        let (nonce_1, s_1) = components(&valid[1].0).unwrap();
        let forged = [
            (from_components(&nonce_0, &(&s_0 + &(&weights[1] * &delta))), valid[0].1),
            (from_components(&nonce_1, &(&s_1 - &(&weights[0] * &delta))), valid[1].1),
        ];

        for (signature, public_key) in &forged {
            assert!(
                !CONTEXT.verify(signature, public_key, &message()),
                "each forged signature is invalid alone"
            );
        }
        let schnorrkel_accepts = schnorrkel::verify_batch_deterministic(
            forged
                .iter()
                .map(|_| signing_context(CONTEXT.as_bytes()).bytes(&message())),
            &forged
                .iter()
                .map(|(signature, _)| Signature::from_bytes(signature.as_bytes()).unwrap())
                .collect::<Vec<_>>(),
            &forged
                .iter()
                .map(|(_, public_key)| PublicKey::from_bytes(public_key.as_bytes()).unwrap())
                .collect::<Vec<_>>(),
            false,
        )
        .is_ok();
        assert!(
            schnorrkel_accepts,
            "the forged pair must fool weights that ignore s, or this test does not exercise the attack",
        );

        let items = forged
            .iter()
            .map(|(signature, public_key)| (signature, public_key, message()))
            .collect::<Vec<_>>();
        assert!(!CONTEXT.verify_batch(&items));
    }

    #[test]
    fn batch_rejects_an_empty_set() {
        assert!(!CONTEXT.verify_batch::<[u8; 64]>(&[]));
    }

    #[test]
    fn rejects_the_identity_key() {
        let identity = RistrettoPublicKeyBytes::zero();
        let signature = from_components(&RistrettoPublicKey::default(), &RistrettoSecretKey::default());

        assert!(!CONTEXT.verify(&signature, &identity, &message()));
        assert!(!CONTEXT.verify_batch(&[(&signature, &identity, message())]));
    }

    #[test]
    fn rejects_a_signature_without_the_marker() {
        let (secret, public_key) = keypair();
        let mut bytes = *CONTEXT.sign(&secret, &message()).as_array();
        bytes[63] &= !SR25519_MARKER;
        let signature = Sr25519SignatureBytes::from_array(bytes);

        assert!(!CONTEXT.verify(&signature, &public_key, &message()));
        assert!(components(&signature).is_none());
    }
}
