//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use time::PrimitiveDateTime;

#[derive(Debug, Clone, Queryable)]
pub struct ForeignSubstatePledge {
    pub id: i32,
    pub transaction_id: String,
    pub substate_id: String,
    pub version: i32,
    pub substate_value: Option<String>,
    pub shard_group: i32,
    pub lock_type: String,
    pub created_at: PrimitiveDateTime,
}
