//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;
use diesel::{Identifiable, Queryable};
use tari_engine_types::substate::InvalidSubstateIdFormat;

use crate::schema::accounts;

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = accounts)]
pub struct Account {
    pub id: i32,
    pub name: Option<String>,
    pub address: String,
    pub owner_key_index: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub is_default: bool,
}

impl TryFrom<Account> for tari_dan_wallet_sdk::models::Account {
    type Error = InvalidSubstateIdFormat;

    fn try_from(account: Account) -> Result<Self, Self::Error> {
        Ok(Self {
            name: account.name,
            address: account.address.parse()?,
            key_index: account.owner_key_index as u64,
            is_default: account.is_default,
        })
    }
}
