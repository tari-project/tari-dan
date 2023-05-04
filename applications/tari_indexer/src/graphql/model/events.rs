//   Copyright 2023. The Tari Project
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

use std::{collections::HashMap, convert::TryInto, sync::Arc};

use async_graphql::{Context, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject};
use log::*;
use serde::{Deserialize, Serialize};
use tari_crypto::tari_utilities::{hex::Hex, message_format::MessageFormat};

use crate::{substate_manager::SubstateManager, substate_storage_sqlite::models::events::NewEvent};

const LOG_TARGET: &str = "tari::indexer::graphql::events";

#[derive(SimpleObject, Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub template_address: [u8; 32],
    pub tx_hash: [u8; 32],
    pub topic: String,
    pub payload: HashMap<String, String>,
}

pub(crate) type EventSchema = Schema<EventQuery, EmptyMutation, EmptySubscription>;

pub struct EventQuery;

#[Object]
impl EventQuery {
    pub async fn get_event(
        &self,
        ctx: &Context<'_>,
        template_address: String,
        tx_hash: String,
    ) -> Result<Vec<Event>, anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "Querying events with template_address = {} and tx_hash = {}", template_address, tx_hash
        );
        let substate_manager = ctx.data_unchecked::<Arc<SubstateManager>>();
        let events = substate_manager.get_event_from_db(template_address, tx_hash).await?;
        let events = events
            .into_iter()
            .map(|e| e.try_into())
            .collect::<Result<Vec<Event>, anyhow::Error>>()?;
        Ok(events)
    }

    pub async fn save_event(
        &self,
        ctx: &Context<'_>,
        template_address: String,
        tx_hash: String,
        topic: String,
        payload: String,
    ) -> Result<Event, anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "Saving event for template_address = {}, tx_hash = {} and topic = {}", template_address, tx_hash, topic
        );
        let mut template_address_bytes = [0u8; 32];
        let mut tx_hash_bytes = [0u8; 32];

        template_address_bytes.copy_from_slice(&Vec::<u8>::from_hex(&template_address).unwrap());
        tx_hash_bytes.copy_from_slice(&Vec::<u8>::from_hex(&tx_hash).unwrap());

        let payload_hashmap = HashMap::<String, String>::from_json(&payload).unwrap();
        let substate_manager = ctx.data_unchecked::<Arc<SubstateManager>>();
        let new_event = NewEvent {
            template_address: template_address.clone(),
            tx_hash: tx_hash.clone(),
            topic,
            payload,
        };
        substate_manager.save_event_to_db(new_event.clone()).await?;

        info!(
            target: LOG_TARGET,
            "Event with template_address = {}, tx_hash = {} and topic = {} has been saved",
            template_address,
            tx_hash,
            new_event.topic
        );

        Ok(Event {
            template_address: template_address_bytes,
            tx_hash: tx_hash_bytes,
            topic: new_event.topic,
            payload: payload_hashmap,
        })
    }
}
