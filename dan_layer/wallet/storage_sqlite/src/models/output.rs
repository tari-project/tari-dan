//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::schema::outputs;

#[derive(Debug, Clone, Identifiable)]
#[diesel(table_name = outputs)]
pub struct Output {
    pub id: i32,
    pub account_id: i32,
    pub commitment: String,
    pub value: i64,
    pub sender_public_nonce: String,
    pub secret_key_index: i64,
    pub public_asset_tag: Option<String>,
    pub status: String,
}
