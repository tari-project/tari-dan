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

use std::{convert::TryFrom, str::FromStr};

use diesel::sql_types::{Integer, Nullable, Text};
use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_template_lib::Hash;

use crate::substate_storage_sqlite::schema::*;

#[derive(Debug, Identifiable, Queryable)]
#[diesel(table_name = events)]
pub struct Event {
    pub id: i32,
    pub template_address: String,
    pub tx_hash: String,
    pub topic: String,
    pub payload: String,
    pub version: i32,
    pub substate_id: Option<String>,
}

#[derive(Debug, Clone, Insertable, AsChangeset)]
#[diesel(table_name = events)]
#[diesel(treat_none_as_null = true)]
pub struct NewEvent {
    pub template_address: String,
    pub tx_hash: String,
    pub topic: String,
    pub payload: String,
    pub version: i32,
    pub substate_id: Option<String>,
}

#[derive(Debug, Clone, Insertable, AsChangeset)]
#[diesel(table_name = event_payloads)]
#[diesel(treat_none_as_null = true)]
pub struct NewEventPayloadField {
    pub payload_key: String,
    pub payload_value: String,
    pub event_id: i32,
}

#[derive(Clone, Debug, QueryableByName, Deserialize, Serialize)]
pub struct EventData {
    #[diesel(sql_type = Text)]
    pub template_address: String,
    #[diesel(sql_type = Text)]
    pub tx_hash: String,
    #[diesel(sql_type = Text)]
    pub topic: String,
    #[diesel(sql_type = Text)]
    pub payload: String,
    #[diesel(sql_type = Integer)]
    pub version: i32,
    #[diesel(sql_type = Nullable<Text>)]
    pub substate_id: Option<String>,
}

impl TryFrom<EventData> for crate::graphql::model::events::Event {
    type Error = anyhow::Error;

    fn try_from(event_data: EventData) -> Result<Self, Self::Error> {
        let substate_id = event_data.substate_id;

        let template_address = Hash::from_hex(&event_data.template_address)?.into_array();

        let tx_hash = Hash::from_hex(&event_data.tx_hash)?.into_array();

        let payload = serde_json::from_str(event_data.payload.as_str())?;

        Ok(Self {
            substate_id,
            template_address,
            tx_hash,
            payload,
            topic: event_data.topic,
        })
    }
}

impl TryFrom<EventData> for tari_engine_types::events::Event {
    type Error = anyhow::Error;

    fn try_from(event_data: EventData) -> Result<Self, Self::Error> {
        let substate_id = event_data
            .substate_id
            .clone()
            .map(|sub_id| SubstateId::from_str(&sub_id))
            .transpose()?;
        let template_address = Hash::from_hex(&event_data.template_address)?;
        let tx_hash = Hash::from_hex(&event_data.tx_hash)?;
        let payload = serde_json::from_str(event_data.payload.as_str())?;

        Ok(Self::new(
            substate_id,
            template_address,
            tx_hash,
            event_data.topic,
            payload,
        ))
    }
}

// To keep track of the latest blocks that we scanned for events

#[derive(Debug, Identifiable, Queryable)]
#[diesel(table_name = scanned_block_ids)]
pub struct ScannedBlockId {
    pub id: i32,
    pub epoch: i64,
    pub shard: i64,
    pub last_block_id: Vec<u8>,
}

#[derive(Debug, Clone, Insertable, AsChangeset)]
#[diesel(table_name = scanned_block_ids)]
pub struct NewScannedBlockId {
    pub epoch: i64,
    pub shard: i64,
    pub last_block_id: Vec<u8>,
}
