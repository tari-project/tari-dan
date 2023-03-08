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

use diesel::sql_types::{Integer, Text};

use crate::substate_storage_sqlite::schema::*;
#[derive(Debug, Identifiable, Queryable)]
#[diesel(table_name = non_fungible_indexes)]
pub struct NonFungibleIndex {
    pub id: i32,
    pub resource_address: String,
    pub idx: i32,
    pub non_fungible_address: String,
}

#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = non_fungible_indexes)]
pub struct NewNonFungibleIndex {
    pub resource_address: String,
    pub idx: i32,
    pub non_fungible_address: String,
}

#[derive(Debug, QueryableByName)]
pub struct IndexedNftSubstate {
    #[diesel(sql_type = Text)]
    pub address: String,
    #[diesel(sql_type = Integer)]
    pub version: i32,
    #[diesel(sql_type = Text)]
    pub data: String,
    #[diesel(sql_type = Integer)]
    pub idx: i32,
}
