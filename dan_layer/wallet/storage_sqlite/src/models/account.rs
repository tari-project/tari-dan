//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;
use diesel::{Identifiable, Queryable};

use crate::schema::accounts;

#[derive(Debug, Clone, Queryable, Identifiable)]
#[table_name = "accounts"]
pub struct Account {
    pub id: i32,
    pub name: String,
    pub address: String,
    pub owner_key_index: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
