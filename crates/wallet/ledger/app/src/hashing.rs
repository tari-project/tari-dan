//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! On-device reproduction of the Ootle transaction signing message and its sr25519 signature.
//!
//! The host streams the canonical preimage bytes (see `tari_ootle_transaction`'s
//! `signing_preimage_v1`); the device feeds them, in order, into a domain-separated Blake2b-512
//! hasher seeded here, recomputes the 64-byte message itself, and signs it as sr25519 under the
//! transaction signing context. The byte recipe is pinned by the host reference test in the
//! ledger client crate and the constants in `ootle_ledger_common::signing`.

use alloc::format;

use curve25519_dalek::{Scalar, ristretto::CompressedRistretto, traits::IsIdentity};
use ledger_device_sdk::hash::{HashError, HashInit, blake2::Blake2b_512};
use merlin::Transcript;
use ootle_ledger_common::{
    OotleStatusWord,
    arg_types::SignMode,
    signing::{
        NONCE_DOMAIN,
        SIGNING_CONTEXT,
        STEALTH_OWNER_LABEL,
        TX_DOMAIN,
        TX_DOMAIN_VERSION,
        TX_LABEL_SEAL,
        TX_LABEL_SIGNATURE,
        WALLET_DOMAIN,
        WALLET_DOMAIN_VERSION,
    },
};
use zeroize::Zeroizing;

use crate::{crypto::public_key_from_scalar, status::AppStatus};

/// Streaming Blake2b-512 hasher seeded with the transaction message domain-separation preamble.
///
/// Wraps the SDK hasher so the streaming state machine can feed preimage segments incrementally
/// across multiple APDUs and finalize the 64-byte message at the end.
pub struct MessageHasher {
    hasher: Blake2b_512,
}

impl MessageHasher {
    /// Start a message hasher for the given signing procedure, writing the domain-separation
    /// preamble for the corresponding label (`"Signature"` or `"SealSignature"`).
    pub fn new(mode: SignMode) -> Result<Self, HashError> {
        let label = match mode {
            SignMode::AddSigner => TX_LABEL_SIGNATURE,
            SignMode::Seal => TX_LABEL_SEAL,
        };
        let mut hasher = Blake2b_512::new();
        add_domain_separation_tag(&mut hasher, TX_DOMAIN, TX_DOMAIN_VERSION, label)?;
        Ok(Self { hasher })
    }

    /// Feed raw canonical (borsh) preimage bytes into the message digest.
    pub fn update(&mut self, data: &[u8]) -> Result<(), HashError> {
        self.hasher.update(data)
    }

    /// Finalize and return the 64-byte signing message.
    pub fn finalize(mut self) -> Result<[u8; 64], HashError> {
        let mut out = [0u8; 64];
        self.hasher.finalize(&mut out)?;
        Ok(out)
    }
}

/// Sign a 64-byte transaction message with `secret`, producing the compressed public key and the
/// standard sr25519 signature encoding `R(32) || s(32)` with the marker bit set in the last byte.
pub fn sign_message(secret: &Scalar, message: &[u8; 64]) -> Result<([u8; 32], [u8; 64]), AppStatus> {
    let public_key = public_key_from_scalar(secret).compress().0;

    // Deterministic (synthetic) nonce: immune to a weak RNG, never reused for a given (key, message).
    let nonce = Zeroizing::new(deterministic_nonce(secret, message).map_err(hash_err)?);
    let public_nonce = public_key_from_scalar(&nonce).compress().0;

    // e = sr25519 challenge(R, P, m); s = e*k + r
    let e = schnorr_challenge(&public_nonce, &public_key, message);
    let s = e * secret + *nonce;

    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(&public_nonce);
    signature[32..].copy_from_slice(s.as_bytes());
    // sr25519 marks its signatures by setting the top bit of `s`, which a canonical scalar leaves clear.
    signature[63] |= 0x80;

    Ok((public_key, signature))
}

/// Derive the stealth signing scalar `c + k` for a confidential transfer, where
/// `c = H_stealth_owner(network, v·R)`, `v` is the account's view secret and `R` is the spent UTXO's
/// sender public nonce. Mirrors `tari_ootle_wallet_crypto::kdfs::owner_stealth_dh_secret`: the DH
/// point `v·R` is compressed and fed borsh-encoded (`u32_le(len) || bytes`) into a `com.tari.ootle.wallet` /
/// `stealth_owner.n{net}` domain-separated Blake2b-512. Pinned by the host reference test in the ledger client crate.
pub fn derive_stealth_secret(
    network_byte: u8,
    account_secret: &Scalar,
    view_secret: &Scalar,
    public_nonce: &[u8; 32],
) -> Result<Scalar, AppStatus> {
    let r = CompressedRistretto(*public_nonce)
        .decompress()
        .ok_or(AppStatus::OotleStatusWord(OotleStatusWord::BadRequest))?;
    // The identity yields a DH point independent of the view key, so the resulting key belongs to no UTXO.
    if r.is_identity() {
        return Err(AppStatus::OotleStatusWord(OotleStatusWord::BadRequest));
    }
    let dh = (view_secret * r).compress().0;

    let label = format!("{STEALTH_OWNER_LABEL}.n{network_byte}");
    let mut hasher = Blake2b_512::new();
    add_domain_separation_tag(&mut hasher, WALLET_DOMAIN, WALLET_DOMAIN_VERSION, &label).map_err(hash_err)?;
    // borsh length-prefixes a byte slice with a little-endian u32.
    hasher.update(&(dh.len() as u32).to_le_bytes()).map_err(hash_err)?;
    hasher.update(&dh).map_err(hash_err)?;
    let mut out = Zeroizing::new([0u8; 64]);
    hasher.finalize(out.as_mut()).map_err(hash_err)?;
    let c = Scalar::from_bytes_mod_order_wide(&out);

    Ok(c + account_secret)
}

/// The sr25519 (schnorrkel) challenge for `message` under [`SIGNING_CONTEXT`], reproducing
/// `tari_ootle_sr25519::SigningContext::challenge`.
fn schnorr_challenge(public_nonce: &[u8; 32], public_key: &[u8; 32], message: &[u8; 64]) -> Scalar {
    let mut transcript = Transcript::new(b"SigningContext");
    transcript.append_message(b"", SIGNING_CONTEXT);
    transcript.append_message(b"sign-bytes", message);
    transcript.append_message(b"proto-name", b"Schnorr-sig");
    transcript.append_message(b"sign:pk", public_key);
    transcript.append_message(b"sign:R", public_nonce);
    let mut out = [0u8; 64];
    transcript.challenge_bytes(b"sign:c", &mut out);
    Scalar::from_bytes_mod_order_wide(&out)
}

fn deterministic_nonce(secret: &Scalar, message: &[u8; 64]) -> Result<Scalar, HashError> {
    let mut hasher = Blake2b_512::new();
    hasher.update(NONCE_DOMAIN)?;
    hasher.update(secret.as_bytes())?;
    hasher.update(message)?;
    let mut out = Zeroizing::new([0u8; 64]);
    hasher.finalize(out.as_mut())?;
    Ok(Scalar::from_bytes_mod_order_wide(&out))
}

/// Writes a `tari_crypto` domain-separation tag: `u64_le(len) || domain || ".v" || version_ascii
/// || ["." || label]`. Mirrors `tari_crypto::hashing::DomainSeparation::add_domain_separation_tag`
/// byte-for-byte.
fn add_domain_separation_tag(
    hasher: &mut Blake2b_512,
    domain: &str,
    version: u8,
    label: &str,
) -> Result<(), HashError> {
    let (version_offset, version_ascii) = byte_to_decimal_ascii_bytes(version);
    let version_len = 3 - version_offset;
    let len = if label.is_empty() {
        domain.len() + version_len + 2
    } else {
        domain.len() + version_len + label.len() + 3
    };
    hasher.update(&(len as u64).to_le_bytes())?;
    hasher.update(domain.as_bytes())?;
    hasher.update(b".v")?;
    hasher.update(&version_ascii[version_offset..])?;
    if !label.is_empty() {
        hasher.update(b".")?;
        hasher.update(label.as_bytes())?;
    }
    Ok(())
}

/// Big-endian decimal ASCII of a byte. Returns `(offset_of_most_significant_digit, [_, _, _])`.
/// Mirrors `tari_crypto`'s private helper of the same name.
fn byte_to_decimal_ascii_bytes(mut byte: u8) -> (usize, [u8; 3]) {
    const ZERO_ASCII_CHAR: u8 = 48;
    let mut bytes = [0u8, 0u8, ZERO_ASCII_CHAR];
    let mut pos = 3usize;
    if byte == 0 {
        return (2, bytes);
    }
    while byte > 0 {
        let rem = byte % 10;
        byte /= 10;
        bytes[pos - 1] = ZERO_ASCII_CHAR + rem;
        pos -= 1;
    }
    (pos, bytes)
}

fn hash_err(_e: HashError) -> AppStatus {
    AppStatus::OotleStatusWord(ootle_ledger_common::OotleStatusWord::HashFail)
}
