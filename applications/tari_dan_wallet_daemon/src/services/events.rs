//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_wallet_sdk::models::{Account, NewAccountInfo, TransactionStatus};
use tari_engine_types::{commit_result::FinalizeResult, substate::SubstateId};
use tari_template_lib::models::Amount;
use tari_transaction::TransactionId;

#[derive(Debug, Clone)]
pub enum WalletEvent {
    TransactionSubmitted(TransactionSubmittedEvent),
    TransactionFinalized(TransactionFinalizedEvent),
    TransactionInvalid(TransactionInvalidEvent),
    AccountCreated(AccountCreatedEvent),
    AccountChanged(AccountChangedEvent),
    AuthLoginRequest(#[allow(dead_code)] AuthLoginRequestEvent),
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
        Self::AccountChanged(value)
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
        Self::AccountCreated(value)
    }
}

#[derive(Debug, Clone)]
pub struct TransactionSubmittedEvent {
    pub transaction_id: TransactionId,
    /// Set to Some if this transaction results in a new account
    pub new_account: Option<NewAccountInfo>,
}

#[derive(Debug, Clone)]
pub struct TransactionFinalizedEvent {
    pub transaction_id: TransactionId,
    pub finalize: FinalizeResult,
    pub final_fee: Amount,
    pub status: TransactionStatus,
}

#[derive(Debug, Clone)]
pub struct AccountCreatedEvent {
    pub account: Account,
    #[allow(dead_code)]
    pub created_by_tx: TransactionId,
}

#[derive(Debug, Clone)]
pub struct AccountChangedEvent {
    pub account_address: SubstateId,
}

#[derive(Debug, Clone)]
pub struct TransactionInvalidEvent {
    pub transaction_id: TransactionId,
    pub status: TransactionStatus,
    pub finalize: Option<FinalizeResult>,
    pub final_fee: Option<Amount>,
}

#[derive(Debug, Clone)]
pub struct AuthLoginRequestEvent;
