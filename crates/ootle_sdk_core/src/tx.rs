//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Sign → seal → BOR-encode an [`UnsignedTransaction`] into submit-ready bytes + transaction id.
//!
//! Signing always uses a fresh random Schnorr nonce (`RistrettoSchnorr::sign(secret, msg,
//! &mut rand::rng())` inside `TransactionSignature::sign_v1` / `TransactionSealSignature::sign_v1`),
//! so the sealed bytes and id are unique per call — the safe default for real submission. Nothing here
//! pins a signature nonce from a seed.
//!
//! ### The `main_signer` rule
//!
//! The field defaults to [`MainSigner::SealSigner`], is only changed by explicit choice, and is
//! committed to by every signing message — `seal()` never mutates it. The
//! [`seal_public_transfer_with_auth`] path (which supplies its own authorizations) settles it
//! explicitly ([`MainSigner::SealSigner`] iff there are zero authorization signatures, otherwise
//! [`MainSigner::None`]) before computing any message, so the value the co-signers signed over is the
//! one the seal commits to.

use tari_ootle_transaction::{
    MainSigner,
    Transaction,
    TransactionEnvelope,
    TransactionId,
    TransactionSignature,
    UnsealedTransactionV1,
    UnsignedTransaction,
};
use tari_template_lib_types::crypto::RistrettoPublicKeyBytes;

use crate::{
    keys::{PublicTransferKeys, parse_secret_key, public_key_bytes_from_secret},
    types::error::OotleSdkError,
};

/// Seals an [`UnsignedTransaction`] with the account key (random nonce).
///
/// Single-key transfer: the account key both authorizes and seals. Returns the sealed
/// [`Transaction`] and its [`TransactionId`]. The bytes/id are **not** reproducible (random nonces).
pub fn seal_public_transfer(
    unsigned: UnsignedTransaction,
    keys: &PublicTransferKeys,
) -> Result<(Transaction, TransactionId), OotleSdkError> {
    let account_secret = parse_secret_key(&keys.account_secret)?;
    let seal_signer_pk = public_key_bytes_from_secret(&account_secret);

    // Authorization signature (stock random nonce), then seal (stock random nonce).
    let unsealed = unsigned.add_signer(&seal_signer_pk, &account_secret);
    let transaction = unsealed.seal(&account_secret);
    let id = transaction.calculate_id();
    Ok((transaction, id))
}

/// Seals an [`UnsignedTransaction`] with **externally-supplied authorization signatures** (the co-sign
/// path), using the stock random-nonce seal. The seal signer is derived from `keys.account_secret`;
/// the supplied `auth_signatures` are attached verbatim as the authorization signatures (they were
/// produced by other parties over the auth message committing to this seal signer).
///
/// `main_signer` is set to [`MainSigner::SealSigner`] iff there are zero authorization signatures,
/// before the seal message is computed. With auth signatures present it is [`MainSigner::None`] (the
/// value the co-sign record shipped, and the one the authorizations signed over), or the bytes would
/// diverge / the engine would reject.
pub fn seal_public_transfer_with_auth(
    unsigned: UnsignedTransaction,
    keys: &PublicTransferKeys,
    auth_signatures: Vec<TransactionSignature>,
) -> Result<(Transaction, TransactionId), OotleSdkError> {
    let seal_secret = parse_secret_key(&keys.account_secret)?;
    let UnsignedTransaction::V1(mut unsigned_v1) = unsigned;

    // With authorizations present the seal signer is not authorized and the transaction has no main
    // signer; with zero authorizations the seal signer is the only authority. Set it explicitly for
    // both cases so the value the seal commits to is the same one the supplied authorizations signed
    // over (the co-sign record settles it before A ships it to B).
    unsigned_v1.main_signer = if auth_signatures.is_empty() {
        MainSigner::SealSigner
    } else {
        MainSigner::None
    };

    let unsealed = UnsealedTransactionV1::new(unsigned_v1, auth_signatures);
    // Stock random-nonce seal — the bytes/id are intentionally non-reproducible.
    let transaction = unsealed.seal(&seal_secret);
    let id = transaction.calculate_id();
    Ok((transaction, id))
}

/// Re-derives the authorization signing message a co-signer must sign: the digest committing to the
/// seal signer's public key and the unsigned transaction (`TransactionSignature::create_message_v1`).
/// A and B both compute this from the identical unsigned record, so they agree on the message.
pub fn cosign_auth_message(seal_signer_pk: &RistrettoPublicKeyBytes, unsigned: &UnsignedTransaction) -> [u8; 64] {
    let UnsignedTransaction::V1(v1) = unsigned;
    TransactionSignature::create_message_v1(seal_signer_pk, v1)
}

/// BOR-encodes a sealed [`Transaction`] into the raw `TransactionEnvelope` wire bytes.
pub fn bor_encode(transaction: Transaction) -> Result<Vec<u8>, OotleSdkError> {
    let envelope = TransactionEnvelope::encode(transaction)
        .map_err(|e| OotleSdkError::Encoding(format!("BOR encode failed: {e}")))?;
    Ok(envelope.0.into_vec())
}

/// Verifies every signature on a sealed transaction (the seal signature and all authorization
/// signatures). A thin wrapper so tests need not reach into the internal type.
#[cfg(test)]
pub(crate) fn verify_all_signatures(transaction: &Transaction) -> bool {
    transaction.verify_all_signatures()
}
