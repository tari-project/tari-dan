//  Copyright 2024 The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{collections::BTreeMap, str::FromStr, sync::Arc};

use log::*;
use tari_crypto::tari_utilities::message_format::MessageFormat;
use tari_dan_app_utilities::substate_file_cache::SubstateFileCache;
use tari_dan_common_types::PeerAddress;
use tari_engine_types::{events::Event, substate::SubstateId};
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_indexer_lib::substate_scanner::SubstateScanner;
use tari_template_lib::{
    models::{Metadata, TemplateAddress},
    Hash,
};
use tari_transaction::TransactionId;
use tari_validator_node_rpc::client::TariValidatorNodeRpcClientFactory;

use crate::substate_storage_sqlite::{
    models::events::NewEvent,
    sqlite_substate_store_factory::{
        SqliteSubstateStore,
        SubstateStore,
        SubstateStoreReadTransaction,
        SubstateStoreWriteTransaction,
    },
};

const LOG_TARGET: &str = "tari::indexer::event_manager";

pub struct EventManager {
    substate_store: SqliteSubstateStore,
    substate_scanner:
        Arc<SubstateScanner<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, SubstateFileCache>>,
}

impl EventManager {
    pub fn new(
        substate_store: SqliteSubstateStore,
        substate_scanner: Arc<
            SubstateScanner<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, SubstateFileCache>,
        >,
    ) -> Self {
        Self {
            substate_store,
            substate_scanner,
        }
    }

    pub fn save_event_to_db(
        &self,
        substate_id: &SubstateId,
        template_address: TemplateAddress,
        tx_hash: TransactionId,
        topic: String,
        payload: &Metadata,
        version: u64,
        timestamp: u64,
    ) -> Result<(), anyhow::Error> {
        let mut tx = self.substate_store.create_write_tx()?;
        let new_event = NewEvent {
            substate_id: Some(substate_id.to_string()),
            template_address: template_address.to_string(),
            tx_hash: tx_hash.to_string(),
            topic,
            payload: payload.to_json().expect("Failed to convert to JSON"),
            version: version as i32,
            timestamp: timestamp as i64,
        };
        tx.save_event(new_event)?;
        tx.commit()?;
        Ok(())
    }

    pub async fn scan_events_for_transaction(&self, tx_id: TransactionId) -> Result<Vec<Event>, anyhow::Error> {
        let events = {
            let mut tx = self.substate_store.create_read_tx()?;
            tx.get_events_for_transaction(tx_id)?
        };

        let mut events = events
            .iter()
            .map(|e| Event::try_from(e.clone()))
            .collect::<Result<Vec<Event>, anyhow::Error>>()?;

        // If we have no events locally, fetch from the network if possible.
        if events.is_empty() {
            let network_events = self.substate_scanner.get_events_for_transaction(tx_id).await?;
            events.extend(network_events);
        }

        Ok(events)
    }

    pub async fn scan_events_for_substate_from_network(
        &self,
        substate_id: SubstateId,
        version: Option<u32>,
    ) -> Result<Vec<Event>, anyhow::Error> {
        let mut events = vec![];
        let version = version.unwrap_or_default();

        // check if database contains the events for this transaction, by querying
        // what is the latest version for the given component_address
        let stored_versions_in_db;
        {
            let mut tx = self.substate_store.create_read_tx()?;
            stored_versions_in_db = tx.get_stored_versions_of_events(&substate_id, version)?;

            let stored_events = match tx.get_all_events(&substate_id) {
                Ok(events) => events,
                Err(e) => {
                    info!(
                        target: LOG_TARGET,
                        "Failed to get all events for substate_id = {}, version = {} with error = {}",
                        substate_id,
                        version,
                        e
                    );
                    return Err(e.into());
                },
            };

            let stored_events = stored_events
                .iter()
                .map(|e| e.clone().try_into())
                .collect::<Result<Vec<_>, _>>()?;
            events.extend(stored_events);
        }

        for v in 0..version {
            if stored_versions_in_db.contains(&v) {
                continue;
            }
            let network_version_events = self
                .substate_scanner
                .get_events_for_substate_and_version(&substate_id, v)
                .await?;
            events.extend(network_version_events);
        }

        let latest_version_in_db = stored_versions_in_db.into_iter().max().unwrap_or_default();
        let version = version.max(latest_version_in_db);

        // check if there are newest events for this component address in the network
        let network_events = self
            .substate_scanner
            .get_events_for_substate(&substate_id, Some(version))
            .await?;
        // because the same substate_id with different version
        // can be processed in the same transaction, we need to avoid
        // duplicates
        for (_, event) in network_events {
            events.push(event);
        }

        Ok(events)
    }

    pub async fn scan_events_by_payload(
        &self,
        payload_key: String,
        payload_value: String,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Event>, anyhow::Error> {
        let events = {
            let mut tx = self.substate_store.create_read_tx()?;
            tx.get_events_by_payload(payload_key, payload_value, offset, limit)?
        };

        let events = events
            .iter()
            .map(|e| Event::try_from(e.clone()))
            .collect::<Result<Vec<Event>, anyhow::Error>>()?;

        Ok(events)
    }

    pub async fn get_events_from_db(
        &self,
        topic: Option<String>,
        substate_id: Option<SubstateId>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Event>, anyhow::Error> {
        let rows = self
            .substate_store
            .with_read_tx(|tx| tx.get_events(substate_id, topic, offset, limit))?;

        let mut events = vec![];
        for row in rows {
            let substate_id = row.substate_id.map(|str| SubstateId::from_str(&str)).transpose()?;
            let template_address = Hash::from_hex(&row.template_address)?;
            let tx_hash = Hash::from_hex(&row.tx_hash)?;
            let topic = row.topic;
            let payload = Metadata::from(serde_json::from_str::<BTreeMap<String, String>>(row.payload.as_str())?);
            events.push(Event::new(substate_id, template_address, tx_hash, topic, payload));
        }

        Ok(events)
    }
}
