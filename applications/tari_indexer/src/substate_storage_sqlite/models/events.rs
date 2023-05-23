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
use tari_template_lib::{prelude::ComponentAddress, Hash};

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
    pub component_address: Option<String>,
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
    pub component_address: Option<String>,
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
    pub component_address: Option<String>,
}

impl TryFrom<EventData> for crate::graphql::model::events::Event {
    type Error = anyhow::Error;

    fn try_from(event_data: EventData) -> Result<Self, Self::Error> {
        let component_address = event_data
            .component_address
            .map(|comp_addr| ComponentAddress::from_str(comp_addr.as_str()))
            .transpose()?
            .map(|comp_addr| comp_addr.into_array());

        let template_address = Hash::from_hex(&event_data.template_address)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .into_array();

        let tx_hash = Hash::from_hex(&event_data.tx_hash)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .into_array();

        let payload = serde_json::from_str(event_data.payload.as_str()).map_err(|e| anyhow::anyhow!(e.to_string()))?;

        Ok(Self {
            component_address,
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
        let component_address = event_data
            .component_address
            .clone()
            .map(|comp_addr| ComponentAddress::from_str(comp_addr.as_str()))
            .transpose()?;
        let template_address =
            Hash::from_hex(&event_data.template_address).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let tx_hash = Hash::from_hex(&event_data.tx_hash).map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let payload = serde_json::from_str(event_data.payload.as_str()).map_err(|e| anyhow::anyhow!(e.to_string()))?;

        Ok(Self::new_with_payload(
            component_address,
            template_address,
            tx_hash,
            event_data.topic,
            payload,
        ))
    }
}
