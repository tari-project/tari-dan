//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use tari_ootle_transaction::{TransactionId, TransactionSignature, UnsignedTransaction};
use time::{OffsetDateTime, PrimitiveDateTime};

use crate::models::{KeyId, WalletLockId};

/// Primary key of a persisted transaction request, and the handle a caller
/// uses to fetch, approve or submit it.
pub type TransactionRequestId = i32;

/// The persisted state of a transaction request.
///
/// `Expired` is deliberately absent: expiry is a function of `expires_at` and
/// the current time, derived by [`TransactionRequestModel::effective_status`]
/// on read. Storing it would need a reaper to write it, and a reaper racing
/// the approve path is exactly the bug that "derive it" avoids.
// NOT exported to TypeScript: this is the *stored* status and has no `Expired`
// variant, because expiry is derived on read. `EffectiveStatus` is the wire
// type -- a UI reading this one could not represent an expired request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TransactionRequestStatus {
    /// Created, awaiting a principal holding `transaction_requests:approve`.
    Pending,
    /// Approved by an approver and not yet submitted.
    Approved,
    /// Refused by an approver. Terminal; the request's locks are released.
    Rejected,
    /// Claimed by a submitter that is sealing and broadcasting. The claim is
    /// what makes concurrent `transaction_requests.submit` calls resolve to a
    /// single sealer: submit is not atomic (seal + broadcast is async work),
    /// so the guard must be taken *before* sealing, not when recording the
    /// result.
    Submitting,
    /// Sealed and handed to the transaction service. Terminal.
    Submitted,
}

impl TransactionRequestStatus {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Approved => "Approved",
            Self::Rejected => "Rejected",
            Self::Submitting => "Submitting",
            Self::Submitted => "Submitted",
        }
    }
}

impl FromStr for TransactionRequestStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Pending" => Ok(Self::Pending),
            "Approved" => Ok(Self::Approved),
            "Rejected" => Ok(Self::Rejected),
            "Submitting" => Ok(Self::Submitting),
            "Submitted" => Ok(Self::Submitted),
            _ => Err(()),
        }
    }
}

/// What a caller sees for a request: its stored status, or `Expired` when the
/// approval window has closed on a request that never reached a terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum EffectiveStatus {
    Pending,
    Approved,
    Rejected,
    /// A submitter holds the claim and is sealing/broadcasting.
    Submitting,
    Submitted,
    /// `expires_at` has passed while the request had not reached a terminal
    /// state. Nothing writes this; it is derived on read. A `Submitting`
    /// claim expires too — that is the backstop for a daemon that died
    /// mid-submit, whose claim would otherwise be held forever.
    Expired,
}

#[derive(Debug, Clone)]
pub struct TransactionRequestModel {
    pub id: TransactionRequestId,
    /// The frozen transaction. Immutable once stored: the approver views this,
    /// and submit seals exactly it.
    pub unsigned_transaction: UnsignedTransaction,
    /// Who pays, and seals last. Always a wallet key: the seal signature
    /// commits to every other signature, so it cannot be sourced externally
    /// before those exist.
    pub seal_signer: KeyId,
    pub other_signers: Vec<KeyId>,
    /// Signatures collected out of band, attached verbatim at submit.
    pub signatures: Vec<TransactionSignature>,
    /// Locks holding this request's inputs. The stealth spend keys are derived
    /// from these at submit rather than named by the caller.
    pub lock_ids: Vec<WalletLockId>,
    /// Admin-assigned name of the API key that created this request, `"local-cookie"`
    /// for a local `.cookie` bearer, or `None` for a wallet session. Display and audit only.
    pub requested_by: Option<String>,
    pub status: TransactionRequestStatus,
    /// The transaction this became. Set only on the move to `Submitted`.
    pub transaction_id: Option<TransactionId>,
    pub expires_at: PrimitiveDateTime,
    pub approved_at: Option<PrimitiveDateTime>,
    pub created_at: PrimitiveDateTime,
    pub updated_at: PrimitiveDateTime,
}

impl TransactionRequestModel {
    /// True once `expires_at` has passed, regardless of stored status.
    pub fn is_past_expiry(&self, now: PrimitiveDateTime) -> bool {
        now > self.expires_at
    }

    /// The status a caller sees. A request that reached a terminal state keeps
    /// it — an approved-and-submitted request does not become Expired just
    /// because its window later closed.
    pub fn effective_status(&self, now: PrimitiveDateTime) -> EffectiveStatus {
        match self.status {
            TransactionRequestStatus::Rejected => EffectiveStatus::Rejected,
            TransactionRequestStatus::Submitted => EffectiveStatus::Submitted,
            _ if self.is_past_expiry(now) => EffectiveStatus::Expired,
            TransactionRequestStatus::Pending => EffectiveStatus::Pending,
            TransactionRequestStatus::Approved => EffectiveStatus::Approved,
            TransactionRequestStatus::Submitting => EffectiveStatus::Submitting,
        }
    }

    pub fn effective_status_now(&self) -> EffectiveStatus {
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
            Date::from_calendar_date(2026, Month::July, day).unwrap(),
            Time::MIDNIGHT,
        )
    }

    fn request(status: TransactionRequestStatus) -> TransactionRequestModel {
        TransactionRequestModel {
            id: 1,
            unsigned_transaction: UnsignedTransaction::new(0u8, tari_ootle_common_types::Epoch(1)),
            seal_signer: KeyId::Derived {
                key_branch: KeyBranch::Account,
                index: 0u64,
            },
            other_signers: vec![],
            signatures: vec![],
            lock_ids: vec![],
            requested_by: None,
            status,
            transaction_id: None,
            // The approval window closes on the 15th.
            expires_at: at(15),
            approved_at: None,
            created_at: at(14),
            updated_at: at(14),
        }
    }

    #[test]
    fn inside_the_window_the_stored_status_stands() {
        assert_eq!(
            request(TransactionRequestStatus::Pending).effective_status(at(14)),
            EffectiveStatus::Pending
        );
        assert_eq!(
            request(TransactionRequestStatus::Approved).effective_status(at(14)),
            EffectiveStatus::Approved
        );
    }

    #[test]
    fn past_the_window_an_unresolved_request_is_expired() {
        // Derived, not stored: nothing writes Expired, so no reaper can race
        // the approve path.
        assert_eq!(
            request(TransactionRequestStatus::Pending).effective_status(at(16)),
            EffectiveStatus::Expired
        );
        assert_eq!(
            request(TransactionRequestStatus::Approved).effective_status(at(16)),
            EffectiveStatus::Expired,
            "approved-but-never-submitted expires too, so submit cannot use a stale approval"
        );
        assert_eq!(
            request(TransactionRequestStatus::Submitting).effective_status(at(16)),
            EffectiveStatus::Expired,
            "a claim held past the window expires -- the backstop for a daemon that died mid-submit"
        );
    }

    #[test]
    fn a_terminal_request_keeps_its_status_forever() {
        // A submitted transaction does not become Expired just because the
        // window later closed -- it already happened.
        assert_eq!(
            request(TransactionRequestStatus::Submitted).effective_status(at(16)),
            EffectiveStatus::Submitted
        );
        assert_eq!(
            request(TransactionRequestStatus::Rejected).effective_status(at(16)),
            EffectiveStatus::Rejected
        );
    }
}
