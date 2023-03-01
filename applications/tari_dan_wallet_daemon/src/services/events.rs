//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::FixedHash;
use tari_dan_common_types::QuorumCertificate;
use tari_dan_wallet_sdk::models::TransactionStatus;
use tari_engine_types::commit_result::FinalizeResult;

#[derive(Debug, Clone)]
pub enum WalletEvent {
    TransactionSubmitted(TransactionSubmittedEvent),
    TransactionFinalized(TransactionFinalizedEvent),
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

#[derive(Debug, Clone)]
pub struct TransactionSubmittedEvent {
    pub hash: FixedHash,
}

#[derive(Debug, Clone)]
pub struct TransactionFinalizedEvent {
    pub hash: FixedHash,
    pub result: FinalizeResult,
    pub qcs: Vec<QuorumCertificate>,
    pub status: TransactionStatus,
}
