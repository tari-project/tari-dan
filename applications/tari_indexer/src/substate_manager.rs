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

use std::{str::FromStr, sync::Arc};

use anyhow::anyhow;
use log::info;
use tari_engine_types::substate::{Substate, SubstateAddress, SubstateValue};

use crate::{
    dan_layer_scanner::DanLayerScanner,
    substate_storage_sqlite::{
        models::substate::{NewSubstate, Substate as SubstateRow},
        sqlite_substate_store_factory::{
            SqliteSubstateStore,
            SubstateStore,
            SubstateStoreReadTransaction,
            SubstateStoreWriteTransaction,
        },
    },
};

const LOG_TARGET: &str = "tari::indexer::substate_manager";

pub struct SubstateManager {
    dan_layer_scanner: Arc<DanLayerScanner>,
    substate_store: SqliteSubstateStore,
}

impl SubstateManager {
    pub fn new(dan_layer_scanner: Arc<DanLayerScanner>, substate_store: SqliteSubstateStore) -> Self {
        Self {
            dan_layer_scanner,
            substate_store,
        }
    }

    pub async fn fetch_and_add_substate_to_db(&self, substate_address: &SubstateAddress) -> Result<(), anyhow::Error> {
        // get the last version of the substate from the dan layer
        let substate_scan_result = self.dan_layer_scanner.get_substate(substate_address, None).await;

        // store the substate into the database
        match substate_scan_result {
            Some(substate) => {
                // if the substate is already stored it will be updated with the new version and data,
                // otherwise it will be inserted as a new row
                let substate_row = map_substate_to_db_row(substate_address, &substate)?;
                let mut tx = self.substate_store.create_write_tx().unwrap();
                tx.set_substate(substate_row)?;
                tx.commit()?;

                info!(
                    target: LOG_TARGET,
                    "Added substate {} with version {} to the database",
                    substate_address.to_address_string(),
                    substate.version()
                );

                Ok(())
            },
            None => Err(anyhow!("Substate not found in the network")),
        }
    }

    pub async fn delete_substate_from_db(&self, substate_address: &SubstateAddress) -> Result<(), anyhow::Error> {
        let mut tx = self.substate_store.create_write_tx().unwrap();
        tx.delete_substate(substate_address.to_address_string())?;
        tx.commit()?;

        Ok(())
    }

    pub async fn delete_all_substates_from_db(&self) -> Result<(), anyhow::Error> {
        let mut tx = self.substate_store.create_write_tx().unwrap();
        tx.clear_substates()?;
        tx.commit()?;

        Ok(())
    }

    pub async fn get_all_addresses_from_db(&self) -> Result<Vec<String>, anyhow::Error> {
        let tx = self.substate_store.create_read_tx().unwrap();
        let addresses = tx.get_all_addresses()?;

        Ok(addresses)
    }

    pub async fn get_substate(
        &self,
        substate_address: &SubstateAddress,
        version: Option<u32>,
    ) -> Result<Option<Substate>, anyhow::Error> {
        // we store the latest version of the substates in the watchlist,
        // so we will return the substate directly from database if it's there
        if let Some(substate) = self.get_substate_from_db(substate_address, version).await? {
            return Ok(Some(substate));
        }

        // the substate is not in db (or is not the requested version) so we fetch it from the dan layer commitee
        let substate = self.get_substate_from_dan_layer(substate_address, version).await;
        Ok(substate)
    }

    async fn get_substate_from_db(
        &self,
        substate_address: &SubstateAddress,
        version: Option<u32>,
    ) -> Result<Option<Substate>, anyhow::Error> {
        let address_str = substate_address.to_address_string();

        let tx = self.substate_store.create_read_tx().unwrap();
        if let Some(row) = tx.get_substate(address_str)? {
            // if a version is requested, we must check that it matches the one in db
            if let Some(version) = version {
                if i64::from(version) != row.version {
                    return Ok(None);
                }
            }

            // the substate is present in db and the version matches the requested version
            let substate = map_db_row_to_substate(&row)?;
            return Ok(Some(substate));
        };

        // the substate is not present in db
        Ok(None)
    }

    async fn get_substate_from_dan_layer(
        &self,
        substate_address: &SubstateAddress,
        version: Option<u32>,
    ) -> Option<Substate> {
        self.dan_layer_scanner.get_substate(substate_address, version).await
    }

    #[allow(dead_code)]
    pub async fn scan_and_update_substates(&self) -> Result<(), anyhow::Error> {
        // fetch all substates from db
        let mut tx = self.substate_store.create_write_tx().unwrap();
        let db_rows: Vec<SubstateRow> = tx.get_all_substates()?;

        // try to get the newest version of each substate in the dan layer, and update the row in the db
        for row in db_rows {
            let address = SubstateAddress::from_str(&row.address)?;
            let res = self.dan_layer_scanner.get_substate(&address, None).await;
            if let Some(substate) = res {
                let updated_row = map_substate_to_db_row(&address, &substate)?;
                tx.set_substate(updated_row)?;
            }
        }

        tx.commit()?;

        Ok(())
    }
}

fn map_db_row_to_substate(row: &SubstateRow) -> Result<Substate, anyhow::Error> {
    let data: SubstateValue = serde_json::from_str(&row.data).unwrap();
    let version = row.version as u32;
    let substate = Substate::new(version, data);
    Ok(substate)
}

fn map_substate_to_db_row(address: &SubstateAddress, substate: &Substate) -> Result<NewSubstate, anyhow::Error> {
    let pretty_data = serde_json::to_string_pretty(&substate)?;
    let row = NewSubstate {
        address: address.to_address_string(),
        version: i64::from(substate.version()),
        data: pretty_data,
    };
    Ok(row)
}
