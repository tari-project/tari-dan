//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;

#[derive(Debug, Clone, Queryable)]
pub struct UtxoProcessQueue {
    pub id: i32,
    pub _account_id: i32,
    pub resource_address: String,
    pub utxo_tag: i32,
    pub public_nonce: String,
    pub _created_at: time::PrimitiveDateTime,
}
