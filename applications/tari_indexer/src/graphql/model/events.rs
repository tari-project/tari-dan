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

use std::{collections::BTreeMap, str::FromStr, sync::Arc};

use async_graphql::{Context, EmptyMutation, EmptySubscription, Object, Schema, SimpleObject};
use log::*;
use serde::{Deserialize, Serialize};
use tari_template_lib::{models::ObjectKey, prelude::ComponentAddress, Hash};
use tari_transaction::TransactionId;

use crate::substate_manager::SubstateManager;

const LOG_TARGET: &str = "tari::indexer::graphql::events";

#[derive(SimpleObject, Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub component_address: Option<[u8; ObjectKey::LENGTH]>,
    pub template_address: [u8; 32],
    pub tx_hash: [u8; 32],
    pub topic: String,
    pub payload: BTreeMap<String, String>,
}

impl Event {
    fn from_engine_event(event: tari_engine_types::events::Event) -> Result<Self, anyhow::Error> {
        Ok(Self {
            component_address: event
                .component_address()
                .map(|comp_addr| comp_addr.as_object_key().into_array()),
            template_address: event.template_address().into_array(),
            tx_hash: event.tx_hash().into_array(),
            topic: event.topic(),
            payload: event.into_payload().into_iter().collect(),
        })
    }
}

pub(crate) type EventSchema = Schema<EventQuery, EmptyMutation, EmptySubscription>;

pub struct EventQuery;

#[Object]
impl EventQuery {
    pub async fn get_events_for_transaction(
        &self,
        ctx: &Context<'_>,
        tx_hash: String,
    ) -> Result<Vec<Event>, anyhow::Error> {
        info!(target: LOG_TARGET, "Querying events for transaction hash = {}", tx_hash);
        let substate_manager = ctx.data_unchecked::<Arc<SubstateManager>>();
        let tx_id = TransactionId::from_hex(&tx_hash)?;
        let events = match substate_manager.scan_events_for_transaction(tx_id).await {
            Ok(events) => events,
            Err(e) => {
                info!(
                    target: LOG_TARGET,
                    "Failed to scan events for transaction {} with error {}", tx_hash, e
                );
                return Err(e);
            },
        };

        let events = events
            .iter()
            .map(|e| Event::from_engine_event(e.clone()))
            .collect::<Result<Vec<Event>, _>>()?;

        Ok(events)
    }

    pub async fn get_events_for_component(
        &self,
        ctx: &Context<'_>,
        component_address: String,
        version: Option<u32>,
    ) -> Result<Vec<Event>, anyhow::Error> {
        let version = version.unwrap_or_default();
        info!(
            target: LOG_TARGET,
            "Querying events for component_address = {}, starting from version = {}", component_address, version
        );
        let substate_manager = ctx.data_unchecked::<Arc<SubstateManager>>();
        let events = substate_manager
            .scan_events_for_substate_from_network(ComponentAddress::from_str(&component_address)?, Some(version))
            .await?
            .iter()
            .map(|e| Event::from_engine_event(e.clone()))
            .collect::<Result<Vec<Event>, anyhow::Error>>()?;

        Ok(events)
    }

    pub async fn get_events(
        &self,
        ctx: &Context<'_>,
        offset: u64,
        limit: u64,
        payload_filter: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<Event>, anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "Querying events, offset: {}, limit: {}, payload_filter: {:?}", offset, limit, payload_filter
        );
        let _substate_manager = ctx.data_unchecked::<Arc<SubstateManager>>();
        let events = vec![
            Event { component_address: None, template_address: [0; 32], tx_hash: [0; 32], topic: "Hello World!".to_owned(), payload: BTreeMap::new() },
        ];

        Ok(events)
    }

    pub async fn save_event(
        &self,
        ctx: &Context<'_>,
        component_address: String,
        template_address: String,
        tx_hash: String,
        topic: String,
        payload: String,
        version: u64,
    ) -> Result<Event, anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "Saving event for component_address = {}, tx_hash = {} and topic = {}", component_address, tx_hash, topic
        );

        let component_address = ComponentAddress::from_hex(&component_address)?;
        let template_address = Hash::from_str(&template_address)?;
        let tx_hash = TransactionId::from_hex(&tx_hash)?;

        let payload = serde_json::from_str(&payload)?;
        let substate_manager = ctx.data_unchecked::<Arc<SubstateManager>>();
        substate_manager.save_event_to_db(
            component_address,
            template_address,
            tx_hash,
            topic.clone(),
            &payload,
            version,
        )?;

        Ok(Event {
            component_address: Some(component_address.as_object_key().into_array()),
            template_address: template_address.into_array(),
            tx_hash: tx_hash.into_array(),
            topic,
            payload: payload.into_iter().collect(),
        })
    }
}
