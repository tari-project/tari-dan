//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_common_types::types::{Commitment, PublicKey};

use crate::models::Account;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Output {
    pub account: Account,
    pub commitment: Commitment,
    pub value: u64,
    pub sender_public_nonce: PublicKey,
    pub secret_key_index: u64,
    pub public_asset_tag: Option<PublicKey>,
    pub status: OutputStatus,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum OutputStatus {
    Unspent,
    Spent,
    Pending,
}
