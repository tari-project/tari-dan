//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//

use std::convert::{TryFrom, TryInto};

use tari_transaction::TransactionId;

use crate::{
    substate_manager::SubstateResponse,
    substate_storage_sqlite::{models::substate::Substate as SubstateRow, schema::*},
};

#[derive(Debug, Identifiable, Queryable)]
pub struct Substate {
    pub id: i32,
    pub address: String,
    pub version: i64,
    pub data: String,
    pub tx_hash: String,
    pub template_address: Option<String>,
    pub module_name: Option<String>,
    pub timestamp: i64,
}

impl TryFrom<Substate> for SubstateResponse {
    type Error = anyhow::Error;

    fn try_from(row: SubstateRow) -> Result<Self, Self::Error> {
        let created_by_transaction = TransactionId::from_hex(&row.tx_hash)?;
        Ok(SubstateResponse {
            address: row.address.parse()?,
            version: row.version.try_into()?,
            substate: serde_json::from_str(&row.data)?,
            created_by_transaction,
        })
    }
}

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = substates)]
pub struct NewSubstate {
    pub address: String,
    pub version: i64,
    pub data: String,
    pub tx_hash: String,
    pub template_address: Option<String>,
    pub module_name: Option<String>,
    pub timestamp: i64,
}
