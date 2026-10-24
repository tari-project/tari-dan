//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, str::FromStr, time::Duration};

use tari_ootle_transaction::{TransactionSignature, UnsignedTransaction};
use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::models::KeyId;

/// Primary key of a persisted signing request, and the handle a caller uses to
/// fetch, approve or reject it.
pub type SigningRequestId = i32;

/// The persisted state of a signing request.
///
/// Expiry is derived on read from `expires_at` (see
/// [`SigningRequestModel::effective_status`]), so it has no stored variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SigningRequestStatus {
    /// Created, awaiting a person holding `signing_requests:approve`.
    Pending,
    /// Approved; the signature is stored on the request. Terminal.
    Signed,
    /// Refused. Terminal.
    Rejected,
}

impl SigningRequestStatus {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Signed => "Signed",
            Self::Rejected => "Rejected",
        }
    }
}

impl FromStr for SigningRequestStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(Self::Pending),
            "Signed" => Ok(Self::Signed),
            "Rejected" => Ok(Self::Rejected),
            _ => Err(()),
        }
    }
}

/// What a caller sees for a signing request: its stored status, or `Expired`
/// when the approval window closed while it was still pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum SigningRequestEffectiveStatus {
    Pending,
    Signed,
    Rejected,
    Expired,
}

/// Who created a signing request, as the wallet authenticated it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum SigningRequester {
    /// The wallet's own interactive session.
    WalletSession,
    /// A local process authenticating with the daemon's `.cookie` token.
    LocalClient,
    /// An API key, by the name an Admin gave it when minting it.
    ApiKey { name: String },
    /// An app connected over WebRTC, holding a token delegated by the wallet.
    /// The wallet knows nothing about which app it is.
    ConnectedApp,
}

impl fmt::Display for SigningRequester {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WalletSession => write!(f, "a wallet session"),
            Self::LocalClient => write!(f, "a local client"),
            Self::ApiKey { name } => write!(f, "API key \"{name}\""),
            Self::ConnectedApp => write!(f, "a connected app"),
        }
    }
}

/// Insert shape for a signing request. A request is always born
/// [`SigningRequestStatus::Pending`] and expires `ttl` from insertion.
#[derive(Debug, Clone, Copy)]
pub struct NewSigningRequest<'a> {
    pub unsigned_transaction: &'a UnsignedTransaction,
    pub seal_public_key: &'a RistrettoPublicKeyBytes,
    pub key_id: KeyId,
    pub signer_public_key: &'a RistrettoPublicKeyBytes,
    pub message_hash: &'a [u8; 64],
    pub memo: &'a str,
    pub requester: &'a SigningRequester,
    pub ttl: Duration,
}

#[derive(Debug, Clone)]
pub struct SigningRequestModel {
    pub id: SigningRequestId,
    /// The transaction body the signature commits to. Immutable once stored.
    pub unsigned_transaction: UnsignedTransaction,
    /// Public key of whoever seals the transaction. Every co-signature commits
    /// to it, so it is fixed before anyone signs.
    pub seal_public_key: RistrettoPublicKeyBytes,
    /// The wallet key that signs on approval.
    pub key_id: KeyId,
    /// Public key of `key_id`, resolved when the request was created.
    pub signer_public_key: RistrettoPublicKeyBytes,
    /// `TransactionSignature::create_message(seal_public_key, unsigned_transaction)`:
    /// exactly what the signature signs.
    pub message_hash: [u8; 64],
    /// Free text from the requester. Display only; nothing verifies it.
    pub memo: String,
    /// Who created the request. Display and audit only.
    pub requester: SigningRequester,
    pub status: SigningRequestStatus,
    /// Set exactly when `status` is `Signed`.
    pub signature: Option<TransactionSignature>,
    pub expires_at: PrimitiveDateTime,
    /// When the request was signed or rejected.
    pub decided_at: Option<PrimitiveDateTime>,
    pub created_at: PrimitiveDateTime,
    pub updated_at: PrimitiveDateTime,
}

impl SigningRequestModel {
    /// The status a caller sees. A signed or rejected request keeps that status
    /// after its approval window closes.
    pub fn effective_status(&self, now: PrimitiveDateTime) -> SigningRequestEffectiveStatus {
        match self.status {
            SigningRequestStatus::Signed => SigningRequestEffectiveStatus::Signed,
            SigningRequestStatus::Rejected => SigningRequestEffectiveStatus::Rejected,
            SigningRequestStatus::Pending if now > self.expires_at => SigningRequestEffectiveStatus::Expired,
            SigningRequestStatus::Pending => SigningRequestEffectiveStatus::Pending,
        }
    }

    pub fn effective_status_now(&self) -> SigningRequestEffectiveStatus {
        let now = OffsetDateTime::now_utc();
        self.effective_status(PrimitiveDateTime::new(now.date(), now.time()))
    }
}

#[cfg(test)]
mod tests {
    use time::{Date, Month, Time};

    use super::*;
    use crate::models::KeyBranch;

    fn at(day: u8) -> PrimitiveDateTime {
        PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::October, day).unwrap(),
            Time::MIDNIGHT,
        )
    }

    fn request(status: SigningRequestStatus) -> SigningRequestModel {
        SigningRequestModel {
            id: 1,
            unsigned_transaction: UnsignedTransaction::new(0u8, tari_ootle_common_types::Epoch(1)),
            seal_public_key: RistrettoPublicKeyBytes::default(),
            key_id: KeyId::Derived {
                key_branch: KeyBranch::Account,
                index: 0u64,
            },
            signer_public_key: RistrettoPublicKeyBytes::default(),
            message_hash: [0u8; 64],
            memo: String::new(),
            requester: SigningRequester::WalletSession,
            status,
            signature: None,
            // The approval window closes on the 15th.
            expires_at: at(15),
            decided_at: None,
            created_at: at(14),
            updated_at: at(14),
        }
    }

    #[test]
    fn a_pending_request_expires_when_its_window_closes() {
        assert_eq!(
            request(SigningRequestStatus::Pending).effective_status(at(14)),
            SigningRequestEffectiveStatus::Pending
        );
        assert_eq!(
            request(SigningRequestStatus::Pending).effective_status(at(16)),
            SigningRequestEffectiveStatus::Expired
        );
    }

    #[test]
    fn a_decided_request_keeps_its_status_after_the_window_closes() {
        assert_eq!(
            request(SigningRequestStatus::Signed).effective_status(at(16)),
            SigningRequestEffectiveStatus::Signed
        );
        assert_eq!(
            request(SigningRequestStatus::Rejected).effective_status(at(16)),
            SigningRequestEffectiveStatus::Rejected
        );
    }
}
