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

use async_graphql::{Context, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryInto;
use tari_crypto::tari_utilities::message_format::MessageFormat;

use crate::substate_manager::SubstateManager;
use crate::substate_storage_sqlite::models::events::NewEvent;

#[derive(SimpleObject, Deserialize, Serialize)]
pub struct Event {
    pub(crate) template_address: [u8; 32],
    pub(crate) tx_hash: [u8; 32],
    pub(crate) topic: String,
    pub(crate) payload: HashMap<String, String>,
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
        panic!("FLAG: WE ARE HEREEE");
        let substate_manager = ctx.data_unchecked::<SubstateManager>();
        let events = substate_manager.get_event_from_db(template_address, tx_hash).await?;
        events
            .into_iter()
            .map(|e| e.try_into())
            .collect::<Result<Vec<Event>, anyhow::Error>>()
    }

    pub async fn save_event(
        &self,
        ctx: &Context<'_>,
        template_address: String,
        tx_hash: String,
        topic: String,
        payload: HashMap<String, String>,
    ) -> Result<String, anyhow::Error> {
        let substate_manager = ctx.data_unchecked::<SubstateManager>();
        // TODO: a more direct way to convert to string ?
        let payload_string = payload.to_json()?.to_string();
        let new_event = NewEvent {
            template_address,
            tx_hash,
            topic,
            payload: payload_string,
        };
        if let Err(e) = substate_manager.save_event_to_db(new_event.clone()).await {
            return Ok(format!("error: {}", e.to_string()));
        }

        Ok("success".to_string())
    }
}
