//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::SystemTime;

use serde_json::Value;
use tari_dan_wallet_sdk::models::TransactionStatus;
use tari_engine_types::{commit_result::FinalizeResult, substate::SubstateAddress};
use tari_template_lib::models::Amount;
use tari_transaction::TransactionId;

#[derive(Debug, Clone)]
pub enum WalletEvent {
    TransactionSubmitted(TransactionSubmittedEvent),
    TransactionFinalized(TransactionFinalizedEvent),
    TransactionInvalid(TransactionInvalidEvent),
    AccountChanged(AccountChangedEvent),
    AuthLoginRequest(AuthLoginRequestEvent),
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

#[derive(Debug, Clone)]
pub struct TransactionSubmittedEvent {
    pub transaction_id: TransactionId,
    /// Set to Some if this transaction results in a new account
    pub new_account: Option<NewAccountInfo>,
}

#[derive(Debug, Clone)]
pub struct NewAccountInfo {
    pub name: Option<String>,
    pub key_index: u64,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct TransactionFinalizedEvent {
    pub transaction_id: TransactionId,
    pub finalize: FinalizeResult,
    pub final_fee: Amount,
    pub status: TransactionStatus,
    pub json_result: Option<Vec<Value>>,
}

#[derive(Debug, Clone)]
pub struct AccountChangedEvent {
    pub account_address: SubstateAddress,
}

#[derive(Debug, Clone)]
pub struct TransactionInvalidEvent {
    pub transaction_id: TransactionId,
    pub status: TransactionStatus,
    pub finalize: Option<FinalizeResult>,
    pub final_fee: Option<Amount>,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct AuthLoginRequestEvent {
    pub auth_token: String,
    pub valid_till: SystemTime,
}
