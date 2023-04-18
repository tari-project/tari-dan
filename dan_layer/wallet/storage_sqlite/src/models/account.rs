//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use chrono::NaiveDateTime;
use diesel::{Identifiable, Queryable};
use tari_engine_types::substate::InvalidSubstateAddressFormat;
use tari_template_lib::models::Amount;

use crate::schema::accounts;

#[derive(Debug, Clone, Queryable, Identifiable)]
#[diesel(table_name = accounts)]
pub struct Account {
    pub id: i32,
    pub name: String,
    pub address: String,
    pub owner_key_index: i64,
    pub balance: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub is_default: bool,
}

impl TryFrom<Account> for tari_dan_wallet_sdk::models::Account {
    type Error = InvalidSubstateAddressFormat;

    fn try_from(account: Account) -> Result<Self, Self::Error> {
        Ok(Self {
            name: account.name,
            address: account.address.parse()?,
            balance: Amount(account.balance),
            key_index: account.owner_key_index as u64,
            is_default: account.is_default,
        })
    }
}
