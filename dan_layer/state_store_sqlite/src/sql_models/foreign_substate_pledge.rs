//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use time::PrimitiveDateTime;

#[derive(Debug, Clone, Queryable)]
pub struct ForeignSubstatePledge {
    #[allow(dead_code)]
    pub id: i32,
    #[allow(dead_code)]
    pub transaction_id: String,
    #[allow(dead_code)]
    pub address: String,
    pub substate_id: String,
    pub version: i32,
    pub substate_value: Option<String>,
    #[allow(dead_code)]
    pub shard_group: i32,
    pub lock_type: String,
    #[allow(dead_code)]
    pub created_at: PrimitiveDateTime,
}
