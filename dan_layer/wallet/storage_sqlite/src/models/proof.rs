//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;
use diesel::{Identifiable, Queryable};

use crate::schema::proofs;

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = proofs)]
pub struct Proof {
    pub id: i32,
    pub account_id: i32,
    pub vault_id: i32,
    pub transaction_hash: Option<String>,
    pub locked_revealed_amount: i64,
    pub created_at: NaiveDateTime,
}
