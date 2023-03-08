//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use tari_common_types::types::{Commitment, PrivateKey, PublicKey};

use crate::models::ConfidentialProofId;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfidentialOutput {
    pub account_name: String,
    pub commitment: Commitment,
    pub value: u64,
    pub sender_public_nonce: Option<PublicKey>,
    pub secret_key_index: u64,
    pub public_asset_tag: Option<PublicKey>,
    pub status: OutputStatus,
    pub locked_by_proof: Option<ConfidentialProofId>,
}

// TODO: Better name?
#[derive(Debug, Clone)]
pub struct ConfidentialOutputWithMask {
    pub account_name: String,
    pub commitment: Commitment,
    pub value: u64,
    pub mask: PrivateKey,
    pub public_asset_tag: Option<PublicKey>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum OutputStatus {
    Unspent,
    Spent,
    Locked,
    LockedUnconfirmed,
}

impl OutputStatus {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            Self::Unspent => "Unspent",
            Self::Spent => "Spent",
            Self::Locked => "Locked",
            Self::LockedUnconfirmed => "LockedUnconfirmed",
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
            _ => Err(()),
        }
    }
}
