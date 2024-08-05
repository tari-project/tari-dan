//  Copyright 2022. The Tari Project
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

use std::{
    collections::BTreeMap,
    fs::create_dir_all,
    ops::{Deref, DerefMut},
    path::PathBuf,
    str::FromStr,
    sync::{Arc, Mutex},
};

use diesel::{
    dsl::count,
    prelude::*,
    sql_query,
    sql_types::{Integer, Nullable, Text},
    SqliteConnection,
};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use log::*;
use tari_crypto::tari_utilities::hex::to_hex;
use tari_dan_common_types::{substate_type::SubstateType, Epoch, ShardGroup};
use tari_dan_storage::{consensus_models::BlockId, StorageError};
use tari_dan_storage_sqlite::{error::SqliteStorageError, SqliteTransaction};
use tari_engine_types::substate::SubstateId;
use tari_indexer_client::types::ListSubstateItem;
use tari_template_lib::models::TemplateAddress;
use tari_transaction::TransactionId;
use thiserror::Error;

use super::models::{
    events::{EventData, NewEvent, NewScannedBlockId},
    non_fungible_index::{IndexedNftSubstate, NewNonFungibleIndex},
};
use crate::substate_storage_sqlite::models::{
    events::{Event, NewEventPayloadField, ScannedBlockId},
    substate::{NewSubstate, Substate},
};

const LOG_TARGET: &str = "tari::indexer::substate_storage_sqlite";

#[derive(Clone)]
pub struct SqliteSubstateStore {
    connection: Arc<Mutex<SqliteConnection>>,
}

impl SqliteSubstateStore {
    pub fn try_create(path: PathBuf) -> Result<Self, StorageError> {
        create_dir_all(path.parent().unwrap()).map_err(|_| StorageError::FileSystemPathDoesNotExist)?;

        let database_url = path.to_str().expect("database_url utf-8 error").to_string();
        let mut connection = SqliteConnection::establish(&database_url).map_err(SqliteStorageError::from)?;

        pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./src/substate_storage_sqlite/migrations");
        if let Err(err) = connection.run_pending_migrations(MIGRATIONS) {
            log::error!(target: LOG_TARGET, "Error running migrations: {}", err);
        }
        sql_query("PRAGMA foreign_keys = ON;")
            .execute(&mut connection)
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "set pragma".to_string(),
            })?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn find_by_address(address: String, conn: &mut SqliteConnection) -> Result<Option<Substate>, StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        let substate_option = substates::table
            .filter(substates::address.eq(address))
            .first(&mut *conn)
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("find_by_address: {}", e),
            })?;

        Ok(substate_option)
    }
}
pub trait SubstateStore {
    type ReadTransaction<'a>: SubstateStoreReadTransaction
    where Self: 'a;
    type WriteTransaction<'a>: SubstateStoreWriteTransaction + Deref<Target = Self::ReadTransaction<'a>>
    where Self: 'a;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError>;
    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError>;

    fn with_write_tx<F: FnOnce(&mut Self::WriteTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let mut tx = self.create_write_tx()?;
        match f(&mut tx) {
            Ok(r) => {
                tx.commit()?;
                Ok(r)
            },
            Err(e) => {
                if let Err(err) = tx.rollback() {
                    log::error!(target: LOG_TARGET, "Failed to rollback transaction: {}", err);
                }
                Err(e)
            },
        }
    }

    fn with_read_tx<F: FnOnce(&mut Self::ReadTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where E: From<StorageError> {
        let mut tx = self.create_read_tx()?;
        let ret = f(&mut tx)?;
        Ok(ret)
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("Storage error: {details}")]
    StorageError { details: String },
}

impl From<StorageError> for StoreError {
    fn from(err: StorageError) -> Self {
        Self::StorageError {
            details: err.to_string(),
        }
    }
}

impl SubstateStore for SqliteSubstateStore {
    type ReadTransaction<'a> = SqliteSubstateStoreReadTransaction<'a>;
    type WriteTransaction<'a> = SqliteSubstateStoreWriteTransaction<'a>;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteSubstateStoreReadTransaction::new(tx))
    }

    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(SqliteSubstateStoreWriteTransaction::new(tx))
    }
}

pub struct SqliteSubstateStoreReadTransaction<'a> {
    transaction: SqliteTransaction<'a>,
}

impl<'a> SqliteSubstateStoreReadTransaction<'a> {
    fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self { transaction }
    }

    fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.connection()
    }
}

// TODO: remove the allow dead_code attributes as these become used.
pub trait SubstateStoreReadTransaction {
    fn list_substates(
        &mut self,
        filter_by_type: Option<SubstateType>,
        filter_by_template: Option<TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<ListSubstateItem>, StorageError>;
    fn get_substate(&mut self, address: &SubstateId) -> Result<Option<Substate>, StorageError>;
    #[allow(dead_code)]
    fn get_latest_version_for_substate(&mut self, address: &SubstateId) -> Result<Option<i64>, StorageError>;
    #[allow(dead_code)]
    fn get_all_addresses(&mut self) -> Result<Vec<(String, i64)>, StorageError>;
    #[allow(dead_code)]
    fn get_all_substates(&mut self) -> Result<Vec<Substate>, StorageError>;
    fn get_non_fungible_collections(&mut self) -> Result<Vec<(String, i64)>, StorageError>;
    fn get_non_fungible_count(&mut self, resource_address: String) -> Result<i64, StorageError>;
    #[allow(dead_code)]
    fn get_non_fungible_latest_index(&mut self, resource_address: String) -> Result<Option<i32>, StorageError>;
    #[allow(dead_code)]
    fn get_non_fungibles(
        &mut self,
        resource_address: String,
        start_idx: i32,
        end_idx: i32,
    ) -> Result<Vec<IndexedNftSubstate>, StorageError>;
    fn get_events_for_transaction(&mut self, tx_id: TransactionId) -> Result<Vec<EventData>, StorageError>;
    fn get_stored_versions_of_events(
        &mut self,
        substate_id: &SubstateId,
        start_version: u32,
    ) -> Result<Vec<u32>, StorageError>;
    #[allow(dead_code)]
    fn get_events_by_version(&mut self, substate_id: &SubstateId, version: u32)
        -> Result<Vec<EventData>, StorageError>;
    fn get_all_events(&mut self, substate_id: &SubstateId) -> Result<Vec<EventData>, StorageError>;
    fn get_events_by_payload(
        &mut self,
        payload_key: String,
        payload_value: String,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<EventData>, StorageError>;
    fn get_events(
        &mut self,
        substate_id_filter: Option<SubstateId>,
        topic_filter: Option<String>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Event>, StorageError>;
    fn event_exists(&mut self, event: NewEvent) -> Result<bool, StorageError>;
    fn get_last_scanned_block_id(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Option<BlockId>, StorageError>;
}

impl SubstateStoreReadTransaction for SqliteSubstateStoreReadTransaction<'_> {
    fn list_substates(
        &mut self,
        by_type: Option<SubstateType>,
        by_template_address: Option<TemplateAddress>,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> Result<Vec<ListSubstateItem>, StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        let mut query = substates::table.into_boxed();

        if let Some(template_address) = by_template_address {
            query = query.filter(substates::template_address.eq(template_address.to_string()));
        }

        if let Some(substate_type) = by_type {
            let address_like = match substate_type {
                SubstateType::NonFungible => format!("resource_% {}_%", substate_type.as_prefix_str()),
                _ => format!("{}_%", substate_type.as_prefix_str()),
            };
            query = query.filter(substates::address.like(address_like));
        }

        if let Some(limit) = limit {
            query = query.limit(limit as i64);
        }
        if let Some(offset) = offset {
            query = query.offset(offset as i64);
        }

        let substates: Vec<Substate> = query
            .get_results(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("list_substates: {}", e),
            })?;

        let items = substates
            .into_iter()
            .map(|s| {
                let substate_id = SubstateId::from_str(&s.address)?;
                let version = u32::try_from(s.version)?;
                let template_address = s.template_address.map(|h| TemplateAddress::from_hex(&h)).transpose()?;
                let timestamp = u64::try_from(s.timestamp)?;
                Ok(ListSubstateItem {
                    substate_id,
                    module_name: s.module_name,
                    version,
                    template_address,
                    timestamp,
                })
            })
            .collect::<Result<Vec<ListSubstateItem>, anyhow::Error>>()
            .map_err(|e| StorageError::QueryError {
                reason: format!("list_substates: invalid substate items: {}", e),
            })?;

        Ok(items)
    }

    fn get_substate(&mut self, address: &SubstateId) -> Result<Option<Substate>, StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        let substate = substates::table
            .filter(substates::address.eq(address.to_string()))
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_substate: {}", e),
            })?;

        Ok(substate)
    }

    fn get_latest_version_for_substate(&mut self, address: &SubstateId) -> Result<Option<i64>, StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        let version = substates::table
            .filter(substates::address.eq(address.to_string()))
            .select(diesel::dsl::max(substates::version))
            .get_result(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_latest_version_for_substate: {}", e),
            })?;

        Ok(version)
    }

    fn get_all_addresses(&mut self) -> Result<Vec<(String, i64)>, StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        let addresses = substates::table
            .select((substates::address, substates::version))
            .get_results(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_all_addresses: {}", e),
            })?;

        match addresses {
            Some(address_vec) => Ok(address_vec),
            None => Ok(vec![]),
        }
    }

    fn get_all_substates(&mut self) -> Result<Vec<Substate>, StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        let substates = substates::table
            .get_results(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_all_substates: {}", e),
            })?;

        match substates {
            Some(substates_vec) => Ok(substates_vec),
            None => Ok(vec![]),
        }
    }

    fn get_non_fungible_collections(&mut self) -> Result<Vec<(String, i64)>, StorageError> {
        use crate::substate_storage_sqlite::schema::non_fungible_indexes as nfts;

        let collections = nfts::table
            .group_by(nfts::resource_address)
            .select((nfts::resource_address, count(nfts::id)))
            .get_results(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_all_addresses: {}", e),
            })?;

        match collections {
            Some(collections_vec) => Ok(collections_vec),
            None => Ok(vec![]),
        }
    }

    fn get_non_fungible_count(&mut self, resource_address: String) -> Result<i64, StorageError> {
        use crate::substate_storage_sqlite::schema::non_fungible_indexes;

        let count = non_fungible_indexes::table
            .filter(non_fungible_indexes::resource_address.eq(resource_address))
            .count()
            .get_result::<i64>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_non_fungible_count: {}", e),
            })?;

        Ok(count)
    }

    fn get_non_fungible_latest_index(&mut self, resource_address: String) -> Result<Option<i32>, StorageError> {
        use crate::substate_storage_sqlite::schema::non_fungible_indexes;

        let latest_index = non_fungible_indexes::table
            .filter(non_fungible_indexes::resource_address.eq(resource_address))
            .select(diesel::dsl::max(non_fungible_indexes::idx))
            .get_result(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_non_fungible_latest_index: {}", e),
            })?;

        Ok(latest_index)
    }

    fn get_non_fungibles(
        &mut self,
        resource_address: String,
        start_idx: i32,
        end_idx: i32,
    ) -> Result<Vec<IndexedNftSubstate>, StorageError> {
        let res = sql_query(
            "SELECT s.address, s.version, s.data, n.idx FROM substates s INNER JOIN non_fungible_indexes n ON \
             s.address = n.non_fungible_address WHERE n.resource_address = ? AND n.idx BETWEEN ? AND ? ORDER BY n.idx \
             ASC",
        )
        .bind::<Text, _>(resource_address)
        .bind::<Integer, _>(start_idx)
        .bind::<Integer, _>(end_idx)
        .get_results::<IndexedNftSubstate>(self.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("get_non_fungibles: {}", e),
        })?;

        Ok(res)
    }

    fn get_events_for_transaction(&mut self, tx_id: TransactionId) -> Result<Vec<EventData>, StorageError> {
        info!(
            target: LOG_TARGET,
            "Querying substate scanner database: get_events_for_transaction with tx_hash = {}", tx_id
        );
        let res = sql_query(
            "SELECT substate_id, template_address, tx_hash, topic, payload, version FROM events WHERE tx_hash = ?",
        )
        .bind::<Text, _>(tx_id.to_string())
        .get_results::<EventData>(self.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("get_events_for_transaction: {}", e),
        })?;

        Ok(res)
    }

    fn get_stored_versions_of_events(
        &mut self,
        substate_id: &SubstateId,
        start_version: u32,
    ) -> Result<Vec<u32>, StorageError> {
        info!(
            target: LOG_TARGET,
            "Querying substate scanner database: get_stored_versions_of_events with substate_id = {} and \
             start_version = {}",
             substate_id,
            start_version
        );
        use crate::substate_storage_sqlite::schema::events;
        let res: Vec<i32> = events::table
            .filter(
                events::substate_id
                    .eq(&substate_id.to_string())
                    .and(events::version.gt(start_version as i32)),
            )
            .select(events::version)
            .get_results(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_last_version_of_events: {}", e),
            })?;

        // for our purposes, a non-existing version in the db, means we have
        // to scan the network from res = 0
        Ok(res.into_iter().map(|v| v as u32).collect::<Vec<_>>())
    }

    fn get_events_by_version(
        &mut self,
        substate_id: &SubstateId,
        version: u32,
    ) -> Result<Vec<EventData>, StorageError> {
        info!(
            target: LOG_TARGET,
            "Querying substate scanner database: get_events_by_version with substate_id = {} and version = {}",
            substate_id,
            version
        );
        let res = sql_query(
            "SELECT substate_id, template_address, tx_hash, topic, payload FROM events WHERE substate_id = ? AND \
             version = ?",
        )
        .bind::<Nullable<Text>, _>(Some(substate_id.to_string()))
        .bind::<Integer, _>(version as i32)
        .get_results::<EventData>(self.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("get_events_by_version: {}", e),
        })?;

        Ok(res)
    }

    fn get_events_by_payload(
        &mut self,
        payload_key: String,
        payload_value: String,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<EventData>, StorageError> {
        info!(
            target: LOG_TARGET,
            "Querying substate scanner database: get_events_by_payload with payload_key = {} and payload_value = {}",
            payload_key,
            payload_value
        );
        let res = sql_query(
            "SELECT substate_id, template_address, tx_hash, topic, payload, version FROM events e INNER JOIN \
             event_payloads p ON p.event_id = e.id WHERE p.payload_key = ? AND p.payload_value = ? LIMIT ?,?",
        )
        .bind::<Text, _>(payload_key)
        .bind::<Text, _>(payload_value)
        .bind::<Integer, _>(offset as i32)
        .bind::<Integer, _>(limit as i32)
        .get_results::<EventData>(self.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("get_events_by_payload: {}", e),
        })?;

        Ok(res)
    }

    fn get_all_events(&mut self, substate_id: &SubstateId) -> Result<Vec<EventData>, StorageError> {
        let res = sql_query("SELECT substate_id, tx_hash, topic, payload FROM events WHERE substate_id = ?")
            .bind::<Text, _>(substate_id.to_string())
            .get_results::<EventData>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_events_by_version: {}", e),
            })?;
        Ok(res)
    }

    fn get_events(
        &mut self,
        substate_id_filter: Option<SubstateId>,
        topic_filter: Option<String>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Event>, StorageError> {
        // TODO: allow to query by payload as well, unifying all event methods into one
        info!(
            target: LOG_TARGET,
            "Querying substate scanner database: get_events with substate_id_filter = {:?} and \
            topic_filter = {:?}",
            substate_id_filter,
            topic_filter
        );
        use crate::substate_storage_sqlite::schema::events;

        let mut query = events::table.into_boxed();

        if let Some(substate_id) = substate_id_filter {
            query = query.filter(events::substate_id.eq(substate_id.to_string()));
        }

        if let Some(topic) = topic_filter {
            query = query.filter(events::topic.eq(topic));
        }

        query = query.offset(offset.into());
        if limit > 0 {
            query = query.limit(limit.into());
        }

        let events = query
            .get_results::<Event>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_events: {}", e),
            })?;

        Ok(events)
    }

    fn event_exists(&mut self, value: NewEvent) -> Result<bool, StorageError> {
        use crate::substate_storage_sqlite::schema::events;

        let count = events::table
            .filter(
                events::substate_id
                    .eq(value.substate_id)
                    .and(events::template_address.eq(value.template_address))
                    .and(events::topic.eq(value.topic))
                    .and(events::version.eq(value.version))
                    .and(events::payload.eq(value.payload))
                    .and(events::tx_hash.eq(value.tx_hash)),
            )
            .count()
            .get_result::<i64>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("event_exists: {}", e),
            })?;
        let exists = count > 0;

        Ok(exists)
    }

    fn get_last_scanned_block_id(
        &mut self,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<Option<BlockId>, StorageError> {
        use crate::substate_storage_sqlite::schema::scanned_block_ids;

        let row: Option<ScannedBlockId> = scanned_block_ids::table
            .filter(
                scanned_block_ids::epoch
                    .eq(epoch.0 as i64)
                    .and(scanned_block_ids::shard_group.eq(shard_group.encode_as_u32() as i32)),
            )
            .first(self.connection())
            .optional()
            .map_err(|e| StorageError::QueryError {
                reason: format!("get_last_scanned_block_id: {}", e),
            })?;

        let block_id_option = row.map(|r| BlockId::try_from(r.last_block_id)).transpose()?;

        Ok(block_id_option)
    }
}

pub struct SqliteSubstateStoreWriteTransaction<'a> {
    /// None indicates if the transaction has been explicitly committed/rolled back
    transaction: Option<SqliteSubstateStoreReadTransaction<'a>>,
}

impl<'a> SqliteSubstateStoreWriteTransaction<'a> {
    pub fn new(transaction: SqliteTransaction<'a>) -> Self {
        Self {
            transaction: Some(SqliteSubstateStoreReadTransaction::new(transaction)),
        }
    }

    pub fn connection(&mut self) -> &mut SqliteConnection {
        self.transaction.as_mut().unwrap().connection()
    }
}

// TODO: remove the allow dead_code attributes as these become used.
pub trait SubstateStoreWriteTransaction {
    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;
    fn set_substate(&mut self, new_substate: NewSubstate) -> Result<(), StorageError>;
    #[allow(dead_code)]
    fn delete_substate(&mut self, address: String) -> Result<(), StorageError>;
    #[allow(dead_code)]
    fn clear_substates(&mut self) -> Result<(), StorageError>;
    #[allow(dead_code)]
    fn add_non_fungible_index(&mut self, new_nft_index: NewNonFungibleIndex) -> Result<(), StorageError>;
    fn save_event(&mut self, new_event: NewEvent) -> Result<(), StorageError>;
    fn save_scanned_block_id(&mut self, new_scanned_block_id: NewScannedBlockId) -> Result<(), StorageError>;
}

impl SubstateStoreWriteTransaction for SqliteSubstateStoreWriteTransaction<'_> {
    fn commit(mut self) -> Result<(), StorageError> {
        self.transaction.take().unwrap().transaction.commit()?;
        Ok(())
    }

    fn rollback(mut self) -> Result<(), StorageError> {
        self.transaction.take().unwrap().transaction.rollback()?;
        Ok(())
    }

    fn set_substate(&mut self, new_substate: NewSubstate) -> Result<(), StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        let address = &new_substate.address;
        let conn = self.connection();
        let current_substate = SqliteSubstateStore::find_by_address(address.clone(), conn)?;

        match current_substate {
            Some(_) => {
                diesel::update(substates::table)
                    .set(&new_substate)
                    .filter(substates::address.eq(address))
                    .execute(&mut *conn)
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("Update leaf node: {}", e),
                    })?;
                info!(
                    target: LOG_TARGET,
                    "Updated substate {} version to {}", address, new_substate.version
                );
            },
            None => {
                diesel::insert_into(substates::table)
                    .values(&new_substate)
                    .execute(&mut *conn)
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("Update substate error: {}", e),
                    })?;
                info!(
                    target: LOG_TARGET,
                    "Added new substate {} with version {}", address, new_substate.version
                );
            },
        };

        Ok(())
    }

    fn delete_substate(&mut self, address: String) -> Result<(), StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        diesel::delete(substates::table)
            .filter(substates::address.eq(address))
            .execute(&mut *self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("delete substate error: {}", e),
            })?;

        Ok(())
    }

    fn clear_substates(&mut self) -> Result<(), StorageError> {
        use crate::substate_storage_sqlite::schema::substates;

        diesel::delete(substates::table)
            .execute(&mut *self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("clear_substates error: {}", e),
            })?;

        Ok(())
    }

    fn add_non_fungible_index(&mut self, new_nft_index: NewNonFungibleIndex) -> Result<(), StorageError> {
        use crate::substate_storage_sqlite::schema::non_fungible_indexes;

        diesel::insert_or_ignore_into(non_fungible_indexes::table)
            .values(&new_nft_index)
            .execute(&mut *self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("add_non_fungible_index error: {}", e),
            })?;

        info!(
            target: LOG_TARGET,
            "Added new NFT index for resource {} with index {}", new_nft_index.resource_address, new_nft_index.idx
        );

        Ok(())
    }

    fn save_event(&mut self, new_event: NewEvent) -> Result<(), StorageError> {
        use crate::substate_storage_sqlite::schema::{event_payloads, events};

        // Save the event into the database
        let event_row: Event = diesel::insert_into(events::table)
            .values(&new_event)
            .get_result::<Event>(self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("save_event: {}", e),
            })?;

        // Save all the key-value pairs of the payload to be able to query them later
        let payload: BTreeMap<String, String> =
            serde_json::from_str(new_event.payload.as_str()).map_err(|e| StorageError::QueryError {
                reason: format!("save_event: {}", e),
            })?;
        let new_payload_fields = payload
            .into_iter()
            .map(|(key, value)| NewEventPayloadField {
                payload_key: key,
                payload_value: value,
                event_id: event_row.id,
            })
            .collect::<Vec<_>>();
        // diesel fails if we try to pass all the new rows in a single insert
        // so the workaround is to loop over them
        // TODO: make diesel work with a single insert instruction instead of looping
        for field in new_payload_fields {
            diesel::insert_into(event_payloads::table)
                .values(&field)
                .execute(self.connection())
                .map_err(|e| StorageError::QueryError {
                    reason: format!("save_event: {}", e),
                })?;
        }
        debug!(
            target: LOG_TARGET,
            "Added new event to the database with substate_id = {:?}, template_address = {} and for transaction \
             hash = {}, version = {}",
            new_event.substate_id,
            new_event.template_address,
            new_event.tx_hash,
            new_event.version,
        );

        Ok(())
    }

    fn save_scanned_block_id(&mut self, new: NewScannedBlockId) -> Result<(), StorageError> {
        use crate::substate_storage_sqlite::schema::scanned_block_ids;

        diesel::insert_into(scanned_block_ids::table)
            .values(&new)
            .on_conflict((scanned_block_ids::epoch, scanned_block_ids::shard_group))
            .do_update()
            .set(new.clone())
            .execute(&mut *self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("save_scanned_block_id error: {}", e),
            })?;

        debug!(
            target: LOG_TARGET,
            "Added new scanned block id {} for epoch {} and shard {:?}", to_hex(&new.last_block_id), new.epoch, new.shard_group
        );

        Ok(())
    }
}

impl<'a> Deref for SqliteSubstateStoreWriteTransaction<'a> {
    type Target = SqliteSubstateStoreReadTransaction<'a>;

    fn deref(&self) -> &Self::Target {
        self.transaction.as_ref().unwrap()
    }
}

impl<'a> DerefMut for SqliteSubstateStoreWriteTransaction<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transaction.as_mut().unwrap()
    }
}

impl Drop for SqliteSubstateStoreWriteTransaction<'_> {
    fn drop(&mut self) {
        if self.transaction.is_some() {
            warn!(
                target: LOG_TARGET,
                "Substate store write transaction was not committed/rolled back"
            );
        }
    }
}
