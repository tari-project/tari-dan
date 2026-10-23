//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Adaptor-signature authorizations ("scriptless scripts") for stealth transactions.
//!
//! These helpers bridge the pure Schnorr adaptor primitives in [`tari_ootle_wallet_crypto::adaptor`] to a
//! [`TransactionAuthorization`] that attaches to a transaction via
//! [`TransactionRequest::add_authorization`](crate::TransactionRequest::add_authorization). The message every
//! authorization signs is obtained from
//! [`WalletStealthAuthorizer::authorization_message`](crate::wallet::WalletStealthAuthorizer::authorization_message).
//!
//! The canonical flow for one leg of an atomic swap, spending a 2-of-2 `AllOf[P_me, P_cp]` script-path leaf:
//! 1. build the unsigned claim and take its `authorization_message`,
//! 2. the counterparty produces an adaptor pre-signature for `P_cp` under `T = t·G` ([`pre_sign_authorization`]),
//! 3. we verify it, obtain `t` from the other chain, and [`complete_authorization`] it,
//! 4. we [`sign_authorization`] for our own key `P_me`,
//! 5. attach both authorizations and seal.

use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_common_types::signature::{Sr25519SignatureBytes, TRANSACTION_SIGNING_CONTEXT};
use tari_ootle_transaction::TransactionSignature;
pub use tari_ootle_wallet_crypto::adaptor::{RistrettoAdaptorSignature, extract, verify};
use tari_ootle_wallet_crypto::adaptor::{complete, sign};

use crate::wallet::TransactionAuthorization;

/// Signs `message` with `secret`, producing an authorization ready to attach for the signer's public key. Use for a
/// key-path or `AccessRule` co-signer whose secret this party holds outright (e.g. our own side of a 2-of-2).
pub fn sign_authorization(secret: &RistrettoSecretKey, message: &[u8]) -> TransactionAuthorization {
    let public_key = RistrettoPublicKey::from_secret_key(secret);
    let signature = TRANSACTION_SIGNING_CONTEXT.sign(secret, message);
    authorization_from_parts(&public_key, signature)
}

/// Produces an adaptor pre-signature over `message` for `secret`, encrypted under `adaptor_point` (`T = t·G`). The
/// holder cannot complete it without the discrete log `t`; the counterparty verifies it with [`verify`] and later
/// completes it with [`complete_authorization`]. The nonce is sampled internally, so it cannot be reused.
pub fn pre_sign_authorization(
    secret: &RistrettoSecretKey,
    adaptor_point: &RistrettoPublicKey,
    message: &[u8],
) -> RistrettoAdaptorSignature {
    sign(secret, adaptor_point, message)
}

/// Completes an adaptor pre-signature with the adaptor secret `t`, yielding an authorization ready to attach for
/// `public_key` (the pre-signer's public key — the badge the completed signature proves). The completed signature is
/// an ordinary transaction signature, so the engine verifies it with no adaptor awareness.
pub fn complete_authorization(
    pre: &RistrettoAdaptorSignature,
    t: &RistrettoSecretKey,
    public_key: &RistrettoPublicKey,
) -> TransactionAuthorization {
    let signature = complete(pre, t);
    authorization_from_parts(public_key, signature)
}

fn authorization_from_parts(
    public_key: &RistrettoPublicKey,
    signature: Sr25519SignatureBytes,
) -> TransactionAuthorization {
    TransactionAuthorization::new(TransactionSignature::new(public_key.to_byte_type(), signature))
}
