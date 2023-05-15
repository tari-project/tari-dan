//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::SystemTime;

use tari_common_types::types::{FixedHash, PublicKey};
use tari_dan_common_types::QuorumCertificate;
use tari_dan_wallet_sdk::models::TransactionStatus;
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason},
    substate::SubstateAddress,
};
use tari_template_lib::models::Amount;

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
    pub hash: FixedHash,
}

#[derive(Debug, Clone)]
pub struct TransactionFinalizedEvent {
    pub hash: FixedHash,
    pub finalize: FinalizeResult,
    pub transaction_failure: Option<RejectReason>,
    pub final_fee: Amount,
    pub qcs: Vec<QuorumCertificate<PublicKey>>,
    pub status: TransactionStatus,
}

#[derive(Debug, Clone)]
pub struct AccountChangedEvent {
    pub account_address: SubstateAddress,
}

#[derive(Debug, Clone)]
pub struct TransactionInvalidEvent {
    pub hash: FixedHash,
    pub status: TransactionStatus,
    pub final_fee: Amount,
}

#[derive(Debug, Clone)]
pub struct AuthLoginRequestEvent {
    pub auth_token: String,
    pub valid_till: SystemTime,
}
