//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;
use diesel::{Identifiable, Queryable};

use crate::schema::config;

#[derive(Debug, Clone, Identifiable, Queryable)]
#[table_name = "config"]
pub struct Config {
    pub id: i32,
    pub key: String,
    pub value: String,
    pub is_encrypted: bool,
    pub updated_at: NaiveDateTime,
    pub created_at: NaiveDateTime,
}
