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

use std::{convert::TryInto, sync::Arc};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_dan_app_utilities::substate_file_cache::SubstateFileCache;
use tari_dan_common_types::PeerAddress;
use tari_engine_types::substate::{Substate, SubstateId};
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_indexer_client::types::{ListSubstateItem, SubstateType};
use tari_indexer_lib::{substate_scanner::SubstateScanner, NonFungibleSubstate};
use tari_template_lib::models::TemplateAddress;
use tari_transaction::TransactionId;
use tari_validator_node_rpc::client::{SubstateResult, TariValidatorNodeRpcClientFactory};

use crate::substate_storage_sqlite::sqlite_substate_store_factory::{
    SqliteSubstateStore,
    SubstateStore,
    SubstateStoreReadTransaction,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstateResponse {
    pub address: SubstateId,
    pub version: u32,
    pub substate: Substate,
    pub created_by_transaction: TransactionId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NonFungibleResponse {
    pub index: u64,
    pub address: SubstateId,
    pub substate: Substate,
}

impl From<NonFungibleSubstate> for NonFungibleResponse {
    fn from(nf: NonFungibleSubstate) -> Self {
        Self {
            index: nf.index,
            address: nf.address,
            substate: nf.substate,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EventResponse {
    pub address: SubstateId,
    pub created_by_transaction: FixedHash,
}

pub struct SubstateManager {
    substate_scanner:
        Arc<SubstateScanner<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, SubstateFileCache>>,
    substate_store: SqliteSubstateStore,
}

impl SubstateManager {
    pub fn new(
        dan_layer_scanner: Arc<
            SubstateScanner<EpochManagerHandle<PeerAddress>, TariValidatorNodeRpcClientFactory, SubstateFileCache>,
        >,
        substate_store: SqliteSubstateStore,
    ) -> Self {
        Self {
            substate_scanner: dan_layer_scanner,
            substate_store,
        }
    }

    pub async fn list_substates(
        &self,
        filter_by_type: Option<SubstateType>,
        filter_by_template: Option<TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<ListSubstateItem>, anyhow::Error>  {
        let mut tx = self.substate_store.create_read_tx()?;
        let substates = tx.list_substates(filter_by_type, filter_by_template, limit, offset)?;
        Ok(substates)
    }

    pub async fn get_substate(
        &self,
        substate_address: &SubstateId,
        version: Option<u32>,
    ) -> Result<Option<SubstateResponse>, anyhow::Error> {
        // we store the latest version of the substates related to the events
        // so we will return the substate directly from database if it's there
        if let Some(substate) = self.get_substate_from_db(substate_address, version).await? {
            return Ok(Some(substate));
        }

        // the substate is not in db (or is not the requested version) so we fetch it from the dan layer committee
        let substate_result = self.substate_scanner.get_substate(substate_address, version).await?;
        match substate_result {
            SubstateResult::Up {
                id,
                substate,
                created_by_tx,
            } => Ok(Some(SubstateResponse {
                address: id,
                version: substate.version(),
                substate,
                created_by_transaction: created_by_tx,
            })),
            _ => Ok(None),
        }
    }

    async fn get_substate_from_db(
        &self,
        substate_address: &SubstateId,
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
        substate_address: &SubstateId,
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

    pub async fn get_non_fungible_count(&self, substate_address: &SubstateId) -> Result<u64, anyhow::Error> {
        let address_str = substate_address.to_address_string();
        let mut tx = self.substate_store.create_read_tx()?;
        let count = tx.get_non_fungible_count(address_str)?;
        Ok(count as u64)
    }

    pub async fn get_non_fungibles(
        &self,
        substate_address: &SubstateId,
        start_index: u64,
        end_index: u64,
    ) -> Result<Vec<NonFungibleResponse>, anyhow::Error> {
        let non_fungibles = if let SubstateId::Resource(addr) = substate_address {
            self.substate_scanner
                .get_non_fungibles(addr, start_index, Some(end_index))
                .await?
                .into_iter()
                .map(Into::into)
                .collect()
        } else {
            vec![]
        };

        Ok(non_fungibles)
    }
}
