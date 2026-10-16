//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::{Identifiable, Queryable};
use time::PrimitiveDateTime;

use crate::schema::trusted_validator_sets;

#[derive(Debug, Clone, Identifiable, Queryable)]
#[diesel(table_name = trusted_validator_sets, primary_key(epoch))]
pub struct TrustedValidatorSet {
    pub epoch: i64,
    pub validator_set: String,
    pub anchor: Option<String>,
    pub created_at: PrimitiveDateTime,
}
