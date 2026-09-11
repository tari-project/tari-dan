//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use tari_ootle_transaction::TransactionId;
use tari_template_lib::types::{Amount, ComponentAddress, ResourceAddress, VaultId};
use time::PrimitiveDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum BalanceChangeSource {
    /// Observed while processing this wallet transaction's finalized substate diff.
    Transaction {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        transaction_id: TransactionId,
    },
    /// Observed while synchronizing the account from on-chain state.
    Scan,
    /// Observed while restoring the account from wallet recovery.
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub enum BalanceChangeSourceType {
    Transaction,
    Scan,
    Recovery,
}

impl BalanceChangeSourceType {
    pub const fn as_key_str(self) -> &'static str {
        match self {
            Self::Transaction => "transaction",
            Self::Scan => "scan",
            Self::Recovery => "recovery",
        }
    }
}

impl FromStr for BalanceChangeSourceType {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "transaction" => Ok(Self::Transaction),
            "scan" => Ok(Self::Scan),
            "recovery" => Ok(Self::Recovery),
            _ => Err("unknown balance change source type"),
        }
    }
}

impl BalanceChangeSource {
    pub fn as_key_str(self) -> &'static str {
        BalanceChangeSourceType::from(self).as_key_str()
    }

    pub fn transaction_id(self) -> Option<TransactionId> {
        match self {
            Self::Transaction { transaction_id } => Some(transaction_id),
            Self::Scan | Self::Recovery => None,
        }
    }
}

impl From<BalanceChangeSource> for BalanceChangeSourceType {
    fn from(source: BalanceChangeSource) -> Self {
        match source {
            BalanceChangeSource::Transaction { .. } => Self::Transaction,
            BalanceChangeSource::Scan => Self::Scan,
            BalanceChangeSource::Recovery => Self::Recovery,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, export_to = "wallet-types/"))]
pub struct BalanceChange {
    pub id: i32,
    pub account_address: ComponentAddress,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub vault_address: Option<VaultId>,
    pub resource_address: ResourceAddress,
    pub token_symbol: Option<String>,
    pub divisibility: u8,
    pub revealed_before: Amount,
    pub revealed_after: Amount,
    pub confidential_before: Amount,
    pub confidential_after: Amount,
    pub revealed_delta: String,
    pub confidential_delta: String,
    pub source: BalanceChangeSource,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub transaction_id: Option<TransactionId>,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub created_at: PrimitiveDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceChangeSnapshot {
    pub account_address: ComponentAddress,
    pub vault_address: Option<VaultId>,
    pub vault_version: Option<u64>,
    pub resource_address: ResourceAddress,
    pub token_symbol: Option<String>,
    pub divisibility: u8,
    pub revealed_before: Amount,
    pub revealed_after: Amount,
    pub confidential_before: Amount,
    pub confidential_after: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceChangePage {
    pub changes: Vec<BalanceChange>,
    pub total: u64,
}

impl BalanceChange {
    pub fn signed_delta(before: Amount, after: Amount) -> String {
        let before = before.to_u128();
        let after = after.to_u128();
        if after >= before {
            (after - before).to_string()
        } else {
            format!("-{}", before - after)
        }
    }
}
