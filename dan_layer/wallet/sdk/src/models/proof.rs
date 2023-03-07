//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub type ConfidentialProofId = u64;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfidentialProofEntry {
    pub id: ConfidentialProofId,
    pub account_name: String,
}
