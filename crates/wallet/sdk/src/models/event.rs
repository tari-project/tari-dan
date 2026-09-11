//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use tari_engine_types::commit_result::FinalizeResult;
use tari_ootle_transaction::TransactionId;
use tari_template_lib::types::{ComponentAddress, UtxoAddress};

use crate::models::{Account, TransactionContext, TransactionRequestId, TransactionStatus};

#[derive(Debug, Clone)]
pub enum WalletEvent {
    TransactionSubmitted(TransactionSubmittedEvent),
    TransactionFinalized(TransactionFinalizedEvent),
    TransactionInvalid(TransactionInvalidEvent),
    AccountCreatedOnChain(AccountCreatedEvent),
    AccountChangedOnChain(AccountChangedEvent),
    AuthLoginRequest(AuthLoginRequestEvent),
    UtxoRecoveryStarted(UtxoRecoveryStartedEvent),
    UtxoRecovered(UtxoRecoveredEvent),
    UtxoRecoveryCompleted(UtxoRecoveryCompletedEvent),
    UtxoSpent(UtxoSpentEvent),
    TransactionRequestCreated(TransactionRequestCreatedEvent),
}

impl WalletEvent {
    pub fn as_event_type(&self) -> WalletEventType {
        self.into()
    }
}

impl From<TransactionSubmittedEvent> for WalletEvent {
    fn from(value: TransactionSubmittedEvent) -> Self {
        Self::TransactionSubmitted(value)
    }
}

impl From<TransactionFinalizedEvent> for WalletEvent {
    fn from(value: TransactionFinalizedEvent) -> Self {
        Self::TransactionFinalized(value)
    }
}

impl From<AccountChangedEvent> for WalletEvent {
    fn from(value: AccountChangedEvent) -> Self {
        Self::AccountChangedOnChain(value)
    }
}

impl From<TransactionInvalidEvent> for WalletEvent {
    fn from(value: TransactionInvalidEvent) -> Self {
        Self::TransactionInvalid(value)
    }
}

impl From<AuthLoginRequestEvent> for WalletEvent {
    fn from(value: AuthLoginRequestEvent) -> Self {
        Self::AuthLoginRequest(value)
    }
}

impl From<AccountCreatedEvent> for WalletEvent {
    fn from(value: AccountCreatedEvent) -> Self {
        Self::AccountCreatedOnChain(value)
    }
}

impl From<UtxoRecoveredEvent> for WalletEvent {
    fn from(value: UtxoRecoveredEvent) -> Self {
        Self::UtxoRecovered(value)
    }
}

impl From<UtxoRecoveryStartedEvent> for WalletEvent {
    fn from(value: UtxoRecoveryStartedEvent) -> Self {
        Self::UtxoRecoveryStarted(value)
    }
}

impl From<UtxoRecoveryCompletedEvent> for WalletEvent {
    fn from(value: UtxoRecoveryCompletedEvent) -> Self {
        Self::UtxoRecoveryCompleted(value)
    }
}

impl From<UtxoSpentEvent> for WalletEvent {
    fn from(value: UtxoSpentEvent) -> Self {
        Self::UtxoSpent(value)
    }
}

impl Display for WalletEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        WalletEventType::from(self).fmt(f)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TransactionSubmittedEvent {
    pub transaction_id: TransactionId,
    pub context: Option<TransactionContext>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TransactionFinalizedEvent {
    pub transaction_id: TransactionId,
    pub finalize: FinalizeResult,
    pub final_fee: u64,
    pub status: TransactionStatus,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AccountCreatedEvent {
    pub account: Account,
    pub _created_by_tx: TransactionId,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AccountChangedEvent {
    pub account_address: ComponentAddress,
    pub version: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TransactionInvalidEvent {
    pub transaction_id: TransactionId,
    pub status: TransactionStatus,
    pub finalize: Option<FinalizeResult>,
    pub final_fee: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuthLoginRequestEvent;

#[derive(Debug, Clone, serde::Serialize)]
pub struct UtxoRecoveredEvent {
    pub address: UtxoAddress,
    pub account_address: ComponentAddress,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UtxoRecoveryStartedEvent {
    pub round_id: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UtxoRecoveryCompletedEvent {
    pub round_id: usize,
    pub num_recovered: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UtxoSpentEvent {
    pub account_address: ComponentAddress,
    pub address: UtxoAddress,
}

/// A transaction request is waiting for a person to approve it.
///
/// Carries only the id: an approver must fetch the request to see what it does,
/// and the payload is broadcast to every subscriber. Submitters poll rather
/// than wait on this -- the notifier is fire-and-forget and does not survive a
/// restart.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransactionRequestCreatedEvent {
    pub request_id: TransactionRequestId,
    /// Admin-assigned name of the API key that asked, or `None` for a wallet
    /// session. Display only.
    pub requested_by: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum WalletEventType {
    TransactionRequestCreated,
    TransactionSubmitted,
    TransactionFinalized,
    TransactionInvalid,
    AccountCreatedOnChain,
    AccountChangedOnChain,
    AuthLoginRequest,
    UtxoRecoveryStarted,
    UtxoRecovered,
    UtxoRecoveryCompleted,
    UtxoSpent,
}

impl Display for WalletEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            WalletEventType::TransactionSubmitted => "TransactionSubmitted",
            WalletEventType::TransactionFinalized => "TransactionFinalized",
            WalletEventType::TransactionInvalid => "TransactionInvalid",
            WalletEventType::AccountCreatedOnChain => "AccountCreatedOnChain",
            WalletEventType::AccountChangedOnChain => "AccountChangedOnChain",
            WalletEventType::AuthLoginRequest => "AuthLoginRequest",
            WalletEventType::UtxoRecoveryStarted => "UtxoRecoveryStarted",
            WalletEventType::UtxoRecovered => "UtxoRecovered",
            WalletEventType::UtxoRecoveryCompleted => "UtxoRecoveryCompleted",
            WalletEventType::UtxoSpent => "UtxoSpent",
            WalletEventType::TransactionRequestCreated => "TransactionRequestCreated",
        };
        write!(f, "{}", s)
    }
}

impl From<&WalletEvent> for WalletEventType {
    fn from(event: &WalletEvent) -> Self {
        match event {
            WalletEvent::TransactionSubmitted(_) => Self::TransactionSubmitted,
            WalletEvent::TransactionFinalized(_) => Self::TransactionFinalized,
            WalletEvent::TransactionInvalid(_) => Self::TransactionInvalid,
            WalletEvent::AccountCreatedOnChain(_) => Self::AccountCreatedOnChain,
            WalletEvent::AccountChangedOnChain(_) => Self::AccountChangedOnChain,
            WalletEvent::AuthLoginRequest(_) => Self::AuthLoginRequest,
            WalletEvent::UtxoRecoveryStarted(_) => Self::UtxoRecoveryStarted,
            WalletEvent::UtxoRecovered(_) => Self::UtxoRecovered,
            WalletEvent::UtxoRecoveryCompleted(_) => Self::UtxoRecoveryCompleted,
            WalletEvent::UtxoSpent(_) => Self::UtxoSpent,
            WalletEvent::TransactionRequestCreated(_) => Self::TransactionRequestCreated,
        }
    }
}
