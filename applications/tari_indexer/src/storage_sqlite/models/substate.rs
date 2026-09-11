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

use std::convert::TryFrom;

use tari_ootle_storage::{StorageError, time::PrimitiveDateTime};

use crate::{
    storage_sqlite::{
        models::substate::SubstateRecord as SubstateRow,
        schema::substates,
        serialization::deserialize_json,
    },
    substate_manager::SubstateResponse,
};

#[derive(Debug, Identifiable, Queryable)]
#[diesel(table_name = substates)]
pub struct SubstateRecord {
    pub id: i32,
    pub address: String,
    pub version: i64,
    pub data: String,
    pub template_address: Option<String>,
    pub module_name: Option<String>,
    pub updated_at: PrimitiveDateTime,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<SubstateRecord> for SubstateResponse {
    type Error = StorageError;

    fn try_from(row: SubstateRow) -> Result<Self, Self::Error> {
        Ok(SubstateResponse {
            id: row.address.parse().map_err(|e| StorageError::DecodingError {
                operation: "TryFrom<SubstateRecord> for SubstateResponse",
                item: "Substate",
                details: format!("Invalid substate address {}: {}", row.address, e),
            })?,
            version: row.version as u64,
            substate: deserialize_json(&row.data)?,
        })
    }
}

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = substates)]
pub struct NewSubstate {
    pub address: String,
    pub version: i64,
    pub data: String,
    pub template_address: Option<String>,
    pub module_name: Option<String>,
}
