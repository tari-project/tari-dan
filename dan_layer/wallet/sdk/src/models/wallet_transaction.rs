//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tari_dan_common_types::QuorumCertificate;
use tari_engine_types::commit_result::{FinalizeResult, RejectReason};
use tari_template_lib::models::Amount;
use tari_transaction::Transaction;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletTransaction {
    pub transaction: Transaction,
    pub status: TransactionStatus,
    pub result: Option<FinalizeResult>,
    pub transaction_failure: Option<RejectReason>,
    pub final_fee: Option<Amount>,
    pub qcs: Vec<QuorumCertificate>,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
pub enum TransactionStatus {
    #[default]
    New,
    DryRun,
    Pending,
    Accepted,
    Rejected,
}

impl TransactionStatus {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            TransactionStatus::New => "New",
            TransactionStatus::DryRun => "DryRun",
            TransactionStatus::Pending => "Pending",
            TransactionStatus::Accepted => "Accepted",
            TransactionStatus::Rejected => "Rejected",
        }
    }
}

impl FromStr for TransactionStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "New" => Ok(TransactionStatus::New),
            "DryRun" => Ok(TransactionStatus::DryRun),
            "Pending" => Ok(TransactionStatus::Pending),
            "Accepted" => Ok(TransactionStatus::Accepted),
            "Rejected" => Ok(TransactionStatus::Rejected),
            _ => Err(anyhow!("Invalid TransactionStatus: {}", s)),
        }
    }
}
