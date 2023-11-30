//  Copyright 2023, The Tari Project
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

use std::{collections::HashMap, convert::TryInto, str::FromStr, sync::Arc};

use anyhow::anyhow;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_crypto::tari_utilities::message_format::MessageFormat;
use tari_dan_app_utilities::substate_file_cache::SubstateFileCache;
use tari_engine_types::{
    events::Event,
    substate::{Substate, SubstateAddress},
};
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_indexer_lib::{
    substate_decoder::find_related_substates,
    substate_scanner::SubstateScanner,
    NonFungibleSubstate,
};
use tari_template_lib::{
    models::TemplateAddress,
    prelude::{ComponentAddress, Metadata},
};
use tari_transaction::TransactionId;
use tari_validator_node_rpc::client::{SubstateResult, TariCommsValidatorNodeClientFactory};

use crate::substate_storage_sqlite::{
    models::{events::NewEvent, non_fungible_index::NewNonFungibleIndex, substate::NewSubstate},
    sqlite_substate_store_factory::{
        SqliteSubstateStore,
        SqliteSubstateStoreWriteTransaction,
        SubstateStore,
        SubstateStoreReadTransaction,
        SubstateStoreWriteTransaction,
    },
};

const LOG_TARGET: &str = "tari::indexer::substate_manager";

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstateResponse {
    pub address: SubstateAddress,
    pub version: u32,
    pub substate: Substate,
    pub created_by_transaction: TransactionId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NonFungibleResponse {
    pub index: u64,
    pub address: SubstateAddress,
    pub substate: Substate,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EventResponse {
    pub address: SubstateAddress,
    pub created_by_transaction: FixedHash,
}

pub struct SubstateManager {
    substate_scanner: Arc<SubstateScanner<EpochManagerHandle, TariCommsValidatorNodeClientFactory, SubstateFileCache>>,
    substate_store: SqliteSubstateStore,
}

impl SubstateManager {
    pub fn new(
        dan_layer_scanner: Arc<
            SubstateScanner<EpochManagerHandle, TariCommsValidatorNodeClientFactory, SubstateFileCache>,
        >,
        substate_store: SqliteSubstateStore,
    ) -> Self {
        Self {
            substate_scanner: dan_layer_scanner,
            substate_store,
        }
    }

    pub async fn fetch_and_add_substate_to_db(&self, substate_address: &SubstateAddress) -> Result<(), anyhow::Error> {
        // get the last version of the substate from the dan layer
        let latest_stored_substate_version = {
            let mut tx = self.substate_store.create_read_tx()?;
            tx.get_latest_version_for_substate(substate_address)?
                .map(|i| u32::try_from(i).expect("Failed to parse latest substate version"))
        };

        let substate = match self
            .substate_scanner
            .get_substate(substate_address, latest_stored_substate_version)
            .await
        {
            Ok(SubstateResult::Up { substate, .. }) => substate,
            Ok(_) => return Err(anyhow!("Substate not found in the network")),
            Err(err) => return Err(anyhow!("Error scanning for substate: {}", err)),
        };

        // fetch all related substates
        let related_addresses = find_related_substates(&substate)?;
        let mut related_substates = HashMap::new();
        for address in related_addresses {
            if let SubstateResult::Up {
                substate: related_substate,
                ..
            // TODO: substate fetching could be done in parallel (tokio)
            } = self.substate_scanner.get_substate(&address, None).await?
            {
                related_substates.insert(address, related_substate);
            }
        }

        // if it's a resource, we need also to retrieve all the individual nfts
        let non_fungibles = if let SubstateAddress::Resource(addr) = substate_address {
            // fetch the last index from database to avoid scaning always from the beginning
            let latest_non_fungible_index = {
                let mut tx = self.substate_store.create_read_tx()?;
                tx.get_non_fungible_latest_index(addr.to_string())?
                    .map(|i| i as u64)
                    .unwrap_or_default()
            };
            self.substate_scanner
                .get_non_fungibles(addr, latest_non_fungible_index, None)
                .await?
        } else {
            vec![]
        };

        // store the substate in the database
        let mut tx = self.substate_store.create_write_tx()?;
        store_substate_in_db(&mut tx, substate_address, &substate)?;
        info!(
            target: LOG_TARGET,
            "Added substate {} with version {} to the database",
            substate_address.to_address_string(),
            substate.version()
        );

        // store related substates in the database
        for (address, substate) in related_substates {
            store_substate_in_db(&mut tx, &address, &substate)?;
            info!(
                target: LOG_TARGET,
                "Added related substate {} of {} to the database",
                address.to_address_string(),
                substate_address.to_address_string()
            );
        }

        // store the associated non fungibles in the database
        for nft in non_fungibles {
            // store the substate of the nft in the databas
            store_substate_in_db(&mut tx, &nft.address, &nft.substate)?;

            // store the index of the nft
            let nft_index_db_row = map_nft_index_to_db_row(substate_address, &nft)?;
            tx.add_non_fungible_index(nft_index_db_row)?;
            info!(
                target: LOG_TARGET,
                "Added non fungible {} at index {} to the database",
                nft.address.to_address_string(),
                nft.index,
            );
        }
        tx.commit()?;

        Ok(())
    }

    pub async fn delete_substate_from_db(&self, substate_address: &SubstateAddress) -> Result<(), anyhow::Error> {
        let mut tx = self.substate_store.create_write_tx()?;
        tx.delete_substate(substate_address.to_address_string())?;
        tx.commit()?;

        Ok(())
    }

    pub async fn delete_all_substates_from_db(&self) -> Result<(), anyhow::Error> {
        let mut tx = self.substate_store.create_write_tx()?;
        tx.clear_substates()?;
        tx.commit()?;

        Ok(())
    }

    pub async fn get_all_addresses_from_db(&self) -> Result<Vec<(String, i64)>, anyhow::Error> {
        let mut tx = self.substate_store.create_read_tx()?;
        let addresses = tx.get_all_addresses()?;

        Ok(addresses)
    }

    pub async fn get_substate(
        &self,
        substate_address: &SubstateAddress,
        version: Option<u32>,
    ) -> Result<Option<SubstateResponse>, anyhow::Error> {
        // we store the latest version of the substates in the watchlist,
        // so we will return the substate directly from database if it's there
        if let Some(substate) = self.get_substate_from_db(substate_address, version).await? {
            return Ok(Some(substate));
        }

        // the substate is not in db (or is not the requested version) so we fetch it from the dan layer committee
        let substate_result = self.substate_scanner.get_substate(substate_address, version).await?;
        match substate_result {
            SubstateResult::Up {
                address,
                substate,
                created_by_tx,
            } => Ok(Some(SubstateResponse {
                address,
                version: substate.version(),
                substate,
                created_by_transaction: created_by_tx,
            })),
            _ => Ok(None),
        }
    }

    async fn get_substate_from_db(
        &self,
        substate_address: &SubstateAddress,
        version: Option<u32>,
    ) -> Result<Option<SubstateResponse>, anyhow::Error> {
        let mut tx = self.substate_store.create_read_tx()?;
        if let Some(row) = tx.get_substate(substate_address)? {
            // if a version is requested, we must check that it matches the one in db
            if let Some(version) = version {
                if i64::from(version) != row.version {
                    return Ok(None);
                }
            }

            // the substate is present in db and the version matches the requested version
            let substate_resp = row.try_into()?;
            return Ok(Some(substate_resp));
        };

        // the substate is not present in db
        Ok(None)
    }

    pub async fn get_specific_substate(
        &self,
        substate_address: &SubstateAddress,
        version: u32,
    ) -> Result<SubstateResult, anyhow::Error> {
        let substate_result = self
            .substate_scanner
            .get_specific_substate_from_committee(substate_address, version)
            .await?;
        Ok(substate_result)
    }

    pub async fn get_non_fungible_collections(&self) -> Result<Vec<(String, i64)>, anyhow::Error> {
        let mut tx = self.substate_store.create_read_tx()?;
        tx.get_non_fungible_collections().map_err(|e| e.into())
    }

    pub async fn get_non_fungible_count(&self, substate_address: &SubstateAddress) -> Result<u64, anyhow::Error> {
        let address_str = substate_address.to_address_string();
        let mut tx = self.substate_store.create_read_tx()?;
        let count = tx.get_non_fungible_count(address_str)?;
        Ok(count as u64)
    }

    pub async fn get_non_fungibles(
        &self,
        substate_address: &SubstateAddress,
        start_index: u64,
        end_index: u64,
    ) -> Result<Vec<NonFungibleResponse>, anyhow::Error> {
        let address_str = substate_address.to_address_string();
        let mut tx = self.substate_store.create_read_tx()?;
        let db_rows = tx.get_non_fungibles(address_str, start_index as i32, end_index as i32)?;

        let mut nfts = Vec::with_capacity(db_rows.len());
        for row in db_rows {
            nfts.push(row.try_into()?);
        }
        Ok(nfts)
    }

    pub fn save_event_to_db(
        &self,
        component_address: ComponentAddress,
        template_address: TemplateAddress,
        tx_hash: TransactionId,
        topic: String,
        payload: &Metadata,
        version: u64,
    ) -> Result<(), anyhow::Error> {
        let mut tx = self.substate_store.create_write_tx()?;
        let new_event = NewEvent {
            component_address: Some(component_address.to_string()),
            template_address: template_address.to_string(),
            tx_hash: tx_hash.to_string(),
            topic,
            payload: payload.to_json().expect("Failed to convert to JSON"),
            version: version as i32,
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
        component_address: ComponentAddress,
        version: Option<u32>,
    ) -> Result<Vec<Event>, anyhow::Error> {
        let mut events = vec![];
        let version = version.unwrap_or_default();

        // check if database contains the events for this transaction, by querying
        // what is the latest version for the given component_address
        let stored_versions_in_db;
        {
            let mut tx = self.substate_store.create_read_tx()?;
            stored_versions_in_db = tx.get_stored_versions_of_events(&component_address, version)?;

            let stored_events = match tx.get_all_events(&component_address) {
                Ok(events) => events,
                Err(e) => {
                    warn!(
                        target: LOG_TARGET,
                        "Failed to get all events for component_address = {}, version = {} with error = {}",
                        component_address,
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
                .get_events_for_component_and_version(component_address, v)
                .await?;
            events.extend(network_version_events);
        }

        let latest_version_in_db = stored_versions_in_db.into_iter().max().unwrap_or_default();
        let version = version.max(latest_version_in_db);

        // check if there are newest events for this component address in the network
        let network_events = self
            .substate_scanner
            .get_events_for_component(component_address, Some(version))
            .await?;

        // stores the newest network events to the db
        // because the same component address with different version
        // can be processed in the same transaction, we need to avoid
        // duplicates
        for (version, event) in network_events.into_iter().filter(|(v, e)| v > &latest_version_in_db) {
            let template_address = event.template_address();
            let tx_hash = TransactionId::new(event.tx_hash().into_array());
            let topic = event.topic();
            let payload = event.payload();
            self.save_event_to_db(
                component_address,
                template_address,
                tx_hash,
                topic,
                payload,
                u64::from(version),
            )?;
            events.push(event);
        }

        Ok(events)
    }

    pub async fn scan_and_update_substates(&self) -> Result<usize, anyhow::Error> {
        let addresses = self.get_all_addresses_from_db().await?;

        let num_scanned = addresses.len();
        for (address, _) in addresses {
            let address = SubstateAddress::from_str(&address)?;
            self.fetch_and_add_substate_to_db(&address).await?;
        }

        Ok(num_scanned)
    }
}

fn store_substate_in_db(
    tx: &mut SqliteSubstateStoreWriteTransaction,
    address: &SubstateAddress,
    substate: &Substate,
) -> Result<(), anyhow::Error> {
    let substate_row = NewSubstate {
        address: address.to_address_string(),
        version: i64::from(substate.version()),
        data: encode_substate(substate)?,
    };
    tx.set_substate(substate_row)?;

    Ok(())
}

fn map_nft_index_to_db_row(
    resource_address: &SubstateAddress,
    nft: &NonFungibleSubstate,
) -> Result<NewNonFungibleIndex, anyhow::Error> {
    Ok(NewNonFungibleIndex {
        resource_address: resource_address.to_address_string(),
        idx: nft.index as i32,
        non_fungible_address: nft.address.to_address_string(),
    })
}

fn encode_substate(substate: &Substate) -> Result<String, anyhow::Error> {
    // let decoded_data = encode_substate_into_json(substate)?;
    // let value = IndexedValue::from_raw(&tari_bor::encode(substate.substate_value())?)?;
    let pretty_json = serde_json::to_string_pretty(&substate)?;
    Ok(pretty_json)
}
