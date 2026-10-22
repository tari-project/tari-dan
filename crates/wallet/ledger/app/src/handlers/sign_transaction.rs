//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Device-agnostic `SignTransaction` streaming state machine.
//!
//! A signing exchange is a sequence of APDUs:
//!   1. `Header` (P2 = [`FrameKind::Header`]) — derivation params + signing [`SignMode`].
//!   2. one `Segment` per canonical preimage field, in order (P2 = [`FrameKind::Segment`], P1 = field tag, high bit set
//!      on the field's last chunk). Bytes are fed verbatim into the message digest and the display summary is parsed
//!      from the same bytes.
//!   3. `Finalize` (P2 = [`FrameKind::Finalize`]) — yields the message + summary for user review.
//!
//! After the user approves, [`sign_approved`] derives the key and produces the signature. The
//! device-specific dispatch (bagl/nbgl) owns the transport and the review UI.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use borsh::BorshDeserialize;
use ootle_ledger_common::{
    OotleStatusWord,
    arg_types::{
        FrameKind,
        KeyType,
        SEGMENT_LAST_CHUNK,
        SignMode,
        SignTransactionHeader,
        SignTransactionResponse,
        SigningField,
    },
};

use crate::{
    hashing::{MessageHasher, derive_stealth_secret, sign_message},
    key_derive::derive_from_bip32_key,
    state::{SigningState, State, TxDisplay},
    status::AppStatus,
};

/// Canonical field order for an authorization signature. Matches
/// `tari_ootle_transaction::TransactionSignature::signing_preimage_v1`.
const ADD_SIGNER_SEQ: &[SigningField] = &[
    SigningField::SchemaVersion,
    SigningField::SealSigner,
    SigningField::Network,
    SigningField::FeeInstructions,
    SigningField::Instructions,
    SigningField::Inputs,
    SigningField::MinEpoch,
    SigningField::MaxEpoch,
    SigningField::MainSigner,
    SigningField::DryRun,
    SigningField::BlobHashes,
];

/// Canonical field order for a seal signature. Matches
/// `tari_ootle_transaction::TransactionSealSignature::signing_preimage_v1`.
const SEAL_SEQ: &[SigningField] = &[
    SigningField::SchemaVersion,
    SigningField::Network,
    SigningField::FeeInstructions,
    SigningField::Instructions,
    SigningField::Inputs,
    SigningField::MinEpoch,
    SigningField::MaxEpoch,
    SigningField::MainSigner,
    SigningField::DryRun,
    SigningField::BlobHashes,
    SigningField::Signatures,
];

fn expected_seq(mode: SignMode) -> &'static [SigningField] {
    match mode {
        SignMode::AddSigner => ADD_SIGNER_SEQ,
        SignMode::Seal => SEAL_SEQ,
    }
}

/// Everything needed to render the review screen and sign once approved.
pub struct SignReview {
    pub message: [u8; 64],
    pub account: u64,
    pub index: u64,
    pub key_type: KeyType,
    pub mode: SignMode,
    pub stealth_public_nonce: Option<[u8; 32]>,
    pub display: TxDisplay,
}

pub enum ChunkResult {
    /// Intermediate chunk consumed; reply with an empty OK.
    Ack,
    /// Stream complete; show the review UI, then call [`sign_approved`] if accepted.
    ReadyToSign(SignReview),
}

const fn stream_err() -> AppStatus {
    AppStatus::OotleStatusWord(OotleStatusWord::SignStreamError)
}

const fn bad_request() -> AppStatus {
    AppStatus::OotleStatusWord(OotleStatusWord::BadRequest)
}

const fn hash_fail() -> AppStatus {
    AppStatus::OotleStatusWord(OotleStatusWord::HashFail)
}

/// Process one `SignTransaction` APDU, identified by its `P1`/`P2` and data payload.
pub fn process_chunk(state: &mut State, p1: u8, p2: u8, data: &[u8]) -> Result<ChunkResult, AppStatus> {
    match FrameKind::try_from(p2).map_err(|_| bad_request())? {
        FrameKind::Header => {
            start_stream(state, data)?;
            Ok(ChunkResult::Ack)
        },
        FrameKind::Segment => {
            process_segment(state, p1, data)?;
            Ok(ChunkResult::Ack)
        },
        FrameKind::Finalize => finalize(state),
    }
}

fn start_stream(state: &mut State, data: &[u8]) -> Result<(), AppStatus> {
    let header = SignTransactionHeader::try_from_slice(data).map_err(|_| bad_request())?;
    let hasher = MessageHasher::new(header.mode).map_err(|_| hash_fail())?;
    *state = State::SigningTransaction(SigningState {
        hasher,
        account: header.account,
        index: header.index,
        key_type: header.key_type,
        mode: header.mode,
        stealth_public_nonce: header.stealth_public_nonce,
        field_cursor: 0,
        in_field: None,
        display: TxDisplay::default(),
    });
    Ok(())
}

fn process_segment(state: &mut State, p1: u8, data: &[u8]) -> Result<(), AppStatus> {
    let signing = match state {
        State::SigningTransaction(s) => s,
        _ => return Err(stream_err()),
    };

    let is_last = (p1 & SEGMENT_LAST_CHUNK) != 0;
    let field = SigningField::try_from(p1 & !SEGMENT_LAST_CHUNK).map_err(|_| bad_request())?;
    let seq = expected_seq(signing.mode);

    match signing.in_field {
        None => {
            // First chunk of the next expected field.
            let expected = seq.get(signing.field_cursor).copied().ok_or_else(stream_err)?;
            if field != expected {
                return Err(stream_err());
            }
            capture_display(&mut signing.display, field, data)?;
            signing.hasher.update(data).map_err(|_| hash_fail())?;
            if is_last {
                signing.field_cursor += 1;
            } else {
                signing.in_field = Some(field);
            }
        },
        Some(current) => {
            // Continuation of a large field.
            if field != current {
                return Err(stream_err());
            }
            signing.hasher.update(data).map_err(|_| hash_fail())?;
            if is_last {
                signing.field_cursor += 1;
                signing.in_field = None;
            }
        },
    }
    Ok(())
}

/// Parse the displayable summary fields from a field's first chunk. All values are derived from
/// the exact bytes hashed, never from a separate host-declared header. A first chunk too short to
/// hold the field's header is rejected, so a truncated stream cannot spoof the displayed summary
/// (show `0`/`None`) while the full data is hashed in later chunks.
fn capture_display(display: &mut TxDisplay, field: SigningField, data: &[u8]) -> Result<(), AppStatus> {
    match field {
        SigningField::Network => display.network = *data.first().ok_or_else(bad_request)?,
        // Vec/IndexSet borsh: leading u32 little-endian length is the element count.
        SigningField::FeeInstructions => display.fee_instruction_count = read_u32(data)?,
        SigningField::Instructions => display.instruction_count = read_u32(data)?,
        SigningField::Inputs => display.input_count = read_u32(data)?,
        // Option<Epoch> borsh: 1-byte tag, then a u64 little-endian if present.
        SigningField::MinEpoch => display.min_epoch = read_option_u64(data)?,
        // Epoch borsh: a bare u64 little-endian — max_epoch is mandatory, so there is no tag.
        SigningField::MaxEpoch => display.max_epoch = read_u64(data)?,
        _ => {},
    }
    Ok(())
}

fn read_u32(data: &[u8]) -> Result<u32, AppStatus> {
    let bytes: [u8; 4] = data
        .get(..4)
        .ok_or_else(bad_request)?
        .try_into()
        .map_err(|_| bad_request())?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(data: &[u8]) -> Result<u64, AppStatus> {
    let bytes: [u8; 8] = data
        .get(..8)
        .ok_or_else(bad_request)?
        .try_into()
        .map_err(|_| bad_request())?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_option_u64(data: &[u8]) -> Result<Option<u64>, AppStatus> {
    match data.first() {
        Some(0) => Ok(None),
        Some(1) => {
            let bytes: [u8; 8] = data
                .get(1..9)
                .ok_or_else(bad_request)?
                .try_into()
                .map_err(|_| bad_request())?;
            Ok(Some(u64::from_le_bytes(bytes)))
        },
        _ => Err(bad_request()),
    }
}

fn finalize(state: &mut State) -> Result<ChunkResult, AppStatus> {
    // Take ownership and reset state up-front: a rejection or error must leave a clean slate.
    let signing = match core::mem::take(state) {
        State::SigningTransaction(s) => s,
        State::None => return Err(stream_err()),
    };

    if signing.in_field.is_some() || signing.field_cursor != expected_seq(signing.mode).len() {
        return Err(stream_err());
    }

    let message = signing.hasher.finalize().map_err(|_| hash_fail())?;
    Ok(ChunkResult::ReadyToSign(SignReview {
        message,
        account: signing.account,
        index: signing.index,
        key_type: signing.key_type,
        mode: signing.mode,
        stealth_public_nonce: signing.stealth_public_nonce,
        display: signing.display,
    }))
}

/// Build the `(label, value)` rows shown on the review screen. Device-agnostic; the bagl/nbgl
/// layers render these into their respective field widgets.
pub fn review_fields(review: &SignReview) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let operation = match (review.mode, review.stealth_public_nonce.is_some()) {
        (SignMode::AddSigner, false) => "Authorize",
        (SignMode::AddSigner, true) => "Authorize (confidential)",
        (SignMode::Seal, false) => "Seal",
        (SignMode::Seal, true) => "Seal (confidential)",
    };
    fields.push(("Operation".to_string(), operation.to_string()));
    fields.push(("Network".to_string(), review.display.network.to_string()));
    fields.push((
        "Fee instructions".to_string(),
        review.display.fee_instruction_count.to_string(),
    ));
    fields.push(("Instructions".to_string(), review.display.instruction_count.to_string()));
    fields.push(("Inputs".to_string(), review.display.input_count.to_string()));
    if let Some(epoch) = review.display.min_epoch {
        fields.push(("Min epoch".to_string(), epoch.to_string()));
    }
    fields.push(("Max epoch".to_string(), review.display.max_epoch.to_string()));
    fields.push(("Tx digest".to_string(), to_hex(&review.message)));
    fields
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Derive the signing key and sign the reviewed message. Called only after the user approves. For a
/// confidential transfer (`stealth_public_nonce` set) the account key is tweaked to the stealth key
/// `c + k` before signing, so the returned public key is the stealth address.
pub fn sign_approved(review: &SignReview) -> Result<SignTransactionResponse, AppStatus> {
    let account_secret = zeroize::Zeroizing::new(derive_from_bip32_key(review.account, review.index, review.key_type)?);
    let secret = match review.stealth_public_nonce {
        Some(nonce) => {
            let view_secret = zeroize::Zeroizing::new(derive_from_bip32_key(
                review.account,
                review.index,
                KeyType::ViewOnlyKey,
            )?);
            zeroize::Zeroizing::new(derive_stealth_secret(
                review.display.network,
                &account_secret,
                &view_secret,
                &nonce,
            )?)
        },
        None => account_secret,
    };
    let (public_key, signature) = sign_message(&secret, &review.message)?;
    Ok(SignTransactionResponse { public_key, signature })
}
