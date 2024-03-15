//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use tari_common_types::types::{Commitment, PublicKey};
use tari_engine_types::substate::SubstateId;
use tari_template_lib::models::EncryptedData;

use crate::models::ConfidentialProofId;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfidentialOutputModel {
    pub account_address: SubstateId,
    pub vault_address: SubstateId,
    pub commitment: Commitment,
    pub value: u64,
    pub sender_public_nonce: Option<PublicKey>,
    pub encryption_secret_key_index: u64,
    pub encrypted_data: EncryptedData,
    pub public_asset_tag: Option<PublicKey>,
    pub status: OutputStatus,
    pub locked_by_proof: Option<ConfidentialProofId>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum OutputStatus {
    /// The output is available for spending
    Unspent,
    /// The output has been spent.
    Spent,
    /// The output is locked for spending. Once the transaction has been accepted, this output becomes Spent.
    Locked,
    /// The output is locked as an unconfirmed output. Once the transaction has been accepted, this output becomes
    /// Unspent.
    LockedUnconfirmed,
    /// This output existing in the vault but could not be validated successfully, meaning the encrypted value and/or
    /// mask were not constructed correctly by the sender. This output will not "be counted" in the confidential
    /// balance.
    Invalid,
}

impl OutputStatus {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            Self::Unspent => "Unspent",
            Self::Spent => "Spent",
            Self::Locked => "Locked",
            Self::LockedUnconfirmed => "LockedUnconfirmed",
            Self::Invalid => "Invalid",
        }
    }
}

impl FromStr for OutputStatus {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Unspent" => Ok(Self::Unspent),
            "Spent" => Ok(Self::Spent),
            "Locked" => Ok(Self::Locked),
            "LockedUnconfirmed" => Ok(Self::LockedUnconfirmed),
            "Invalid" => Ok(Self::Invalid),
            _ => Err(()),
        }
    }
}
