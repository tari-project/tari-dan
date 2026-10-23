//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Shared definitions of the APDU protocol spoken between the Tari Ootle Ledger device app and a
//! host client (`ootle_ledger_client`).
//!
//! Both sides depend on this crate so the wire format is defined exactly once:
//!
//! - [`Instruction`] — the APDU instruction set (sent under class byte `0x80`).
//! - [`OotleStatusWord`] — app-specific error status words returned by the device.
//! - [`arg_types`] — borsh-encoded request/response bodies and the framing of the streamed `SignTransaction` exchange.
//! - [`signing`] — domain-separation constants the device uses to recompute the transaction signing message, and the
//!   sr25519 signing context of the signature over it.
//!
//! The crate is `no_std` by default so it can build inside the Ledger embedded app; the host
//! enables the `std` feature.

#![cfg_attr(not(feature = "std"), no_std)]

pub mod arg_types;

/// Constants defining the exact byte recipe a hardware signer reproduces to recompute a
/// transaction's signing message and sign it. These mirror the domain used by
/// `tari_ootle_transaction`'s `transaction_hasher_v1` and the signing context of
/// `tari_ootle_common_types::signature`, and are shared by the device implementation and the host
/// reference test so both stay aligned.
pub mod signing {
    /// Domain for the Ootle transaction message digest (`transaction_hasher_v1`).
    pub const TX_DOMAIN: &str = "com.tari.ootle.transaction";
    pub const TX_DOMAIN_VERSION: u8 = 1;
    /// Label for an authorization ("add signer") signature message.
    pub const TX_LABEL_SIGNATURE: &str = "Signature";
    /// Label for a seal signature message.
    pub const TX_LABEL_SEAL: &str = "SealSignature";

    /// sr25519 signing context of transaction signatures (`TRANSACTION_SIGNING_CONTEXT`).
    pub const SIGNING_CONTEXT: &[u8] = b"substrate";

    /// Domain for Ootle wallet key-derivation KDFs, mirroring `OotleWalletHashDomain` in
    /// `tari_ootle_wallet_crypto::hashers`. Used to derive the stealth owner signing key on-device.
    pub const WALLET_DOMAIN: &str = "com.tari.ootle.wallet";
    pub const WALLET_DOMAIN_VERSION: u8 = 1;
    /// Base label for the stealth owner-secret KDF (`stealth_owner_hasher64`). The full label
    /// appends `.n{network_byte}`, matching `wallet_hasher64`.
    pub const STEALTH_OWNER_LABEL: &str = "stealth_owner";

    /// Device-internal domain tag for deterministic (synthetic) nonce derivation. Not part of any
    /// network format — only the resulting signature must verify — so this is chosen freely.
    pub const NONCE_DOMAIN: &[u8] = b"com.tari.ootle.ledger.schnorr_nonce.v1";
}

/// APDU instruction set for the Ootle Ledger app.
/// Byte values must match what `ootle_ledger_client` sends (CLA = 0x80).
#[repr(u8)]
#[derive(Debug)]
pub enum Instruction {
    /// Return the app version (`CARGO_PKG_VERSION`) as UTF-8 bytes.
    GetVersion = 0x01,
    /// Return the app name as UTF-8 bytes.
    GetAppName = 0x02,
    /// Derive and return a public key. Body: [`arg_types::GetPublicKeyRequest`], response:
    /// [`arg_types::GetPublicKeyResponse`].
    GetPublicKey = 0x03,
    /// Sign a transaction, streamed as a sequence of frames (see [`arg_types::FrameKind`]).
    /// Concludes with the user review on-device and an [`arg_types::SignTransactionResponse`].
    SignTransaction = 0x04,
}

impl TryFrom<u8> for Instruction {
    type Error = ();

    fn try_from(ins: u8) -> Result<Self, Self::Error> {
        match ins {
            0x01 => Ok(Instruction::GetVersion),
            0x02 => Ok(Instruction::GetAppName),
            0x03 => Ok(Instruction::GetPublicKey),
            0x04 => Ok(Instruction::SignTransaction),

            _ => Err(()),
        }
    }
}

/// App-specific error status words returned in the APDU status bytes (`SW1SW2`).
///
/// On the wire these are offset by [`OOTLE_STATUS_BASE`] (see [`Self::to_status`]) to keep them
/// out of the ISO 7816 / Ledger SDK status ranges. A successful exchange returns the standard
/// `0x9000` instead.
#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum OotleStatusWord {
    /// The request body could not be decoded, or the APDU parameters were invalid.
    BadRequest = 0x01,
    /// The device failed to encode the response body.
    EncodeResponseFail = 0x02,
    /// Deriving the requested key on-device failed.
    KeyDeriveFail = 0x03,
    /// A streamed `SignTransaction` chunk arrived out of order or in an unexpected state.
    SignStreamError = 0x04,
    /// Hashing the transaction message or Schnorr challenge failed on-device.
    HashFail = 0x05,
    /// The user rejected the transaction on the device.
    UserRejected = 0x06,
}

/// Base offset for [`OotleStatusWord`] values on the wire, keeping app-specific errors clear of
/// the ISO 7816 and Ledger SDK status word ranges.
pub const OOTLE_STATUS_BASE: u16 = 0xB000;

impl OotleStatusWord {
    /// The 16-bit status word as sent on the wire: [`OOTLE_STATUS_BASE`] plus the error code.
    pub fn to_status(self) -> u16 {
        OOTLE_STATUS_BASE | self as u16
    }
}

impl TryFrom<u16> for OotleStatusWord {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if value & 0xFF00 != OOTLE_STATUS_BASE {
            return Err(value);
        }
        match value & 0x00FF {
            c if c == Self::BadRequest as u16 => Ok(Self::BadRequest),
            c if c == Self::EncodeResponseFail as u16 => Ok(Self::EncodeResponseFail),
            c if c == Self::KeyDeriveFail as u16 => Ok(Self::KeyDeriveFail),
            c if c == Self::SignStreamError as u16 => Ok(Self::SignStreamError),
            c if c == Self::HashFail as u16 => Ok(Self::HashFail),
            c if c == Self::UserRejected as u16 => Ok(Self::UserRejected),
            _ => Err(value),
        }
    }
}
