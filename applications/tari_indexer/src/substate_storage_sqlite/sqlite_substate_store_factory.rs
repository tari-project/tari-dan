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
    fs::create_dir_all,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use diesel::{
    dsl::count,
    prelude::*,
    sql_query,
    sql_types::{Integer, Text},
    SqliteConnection,
};
use diesel_migrations::EmbeddedMigrations;
use log::warn;
use tari_dan_common_types::PayloadId;
use tari_dan_core::storage::StorageError;
use tari_dan_storage_sqlite::{error::SqliteStorageError, SqliteTransaction};
use tari_engine_types::{substate::SubstateAddress, TemplateAddress};
use thiserror::Error;

use super::models::{
    events::{EventData, NewEvent},
    non_fungible_index::{IndexedNftSubstate, NewNonFungibleIndex},
};
use crate::{
    diesel_migrations::MigrationHarness,
    substate_storage_sqlite::models::substate::{NewSubstate, Substate},
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
    where
        Self: 'a;
    type WriteTransaction<'a>: SubstateStoreWriteTransaction + Deref<Target = Self::ReadTransaction<'a>>
    where
        Self: 'a;

    fn create_read_tx(&self) -> Result<Self::ReadTransaction<'_>, StorageError>;
    fn create_write_tx(&self) -> Result<Self::WriteTransaction<'_>, StorageError>;

    fn with_write_tx<F: FnOnce(&mut Self::WriteTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where
        E: From<StorageError>,
    {
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

    fn with_read_tx<F: FnOnce(&Self::ReadTransaction<'_>) -> Result<R, E>, R, E>(&self, f: F) -> Result<R, E>
    where
        E: From<StorageError>,
    {
        let tx = self.create_read_tx()?;
        let ret = f(&tx)?;
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

pub trait SubstateStoreReadTransaction {
    fn get_substate(&mut self, address: &SubstateAddress) -> Result<Option<Substate>, StorageError>;
    fn get_all_addresses(&mut self) -> Result<Vec<(String, i64)>, StorageError>;
    fn get_all_substates(&mut self) -> Result<Vec<Substate>, StorageError>;
    fn get_non_fungible_collections(&mut self) -> Result<Vec<(String, i64)>, StorageError>;
    fn get_non_fungible_count(&mut self, resource_address: String) -> Result<i64, StorageError>;
    fn get_non_fungibles(
        &mut self,
        resource_address: String,
        start_idx: i32,
        end_idx: i32,
    ) -> Result<Vec<IndexedNftSubstate>, StorageError>;
    fn get_events(
        &mut self,
        template_address: TemplateAddress,
        topic: PayloadId,
    ) -> Result<Vec<EventData>, StorageError>;
}

impl SubstateStoreReadTransaction for SqliteSubstateStoreReadTransaction<'_> {
    fn get_substate(&mut self, address: &SubstateAddress) -> Result<Option<Substate>, StorageError> {
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

    fn get_events(
        &mut self,
        template_address: TemplateAddress,
        tx_hash: PayloadId,
    ) -> Result<Vec<EventData>, StorageError> {
        let res = sql_query(
            "SELECT template_address, tx_hash, topic, payload FROM events WHERE template_address = ? AND tx_hash = ?",
        )
        .bind::<Text, _>(template_address.to_string())
        .bind::<Text, _>(tx_hash.to_string())
        .get_results::<EventData>(self.connection())
        .map_err(|e| StorageError::QueryError {
            reason: format!("get_events: {}", e),
        })?;

        Ok(res)
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

pub trait SubstateStoreWriteTransaction {
    fn commit(self) -> Result<(), StorageError>;
    fn rollback(self) -> Result<(), StorageError>;
    fn set_substate(&mut self, new_substate: NewSubstate) -> Result<(), StorageError>;
    fn delete_substate(&mut self, address: String) -> Result<(), StorageError>;
    fn clear_substates(&mut self) -> Result<(), StorageError>;
    fn add_non_fungible_index(&mut self, new_nft_index: NewNonFungibleIndex) -> Result<(), StorageError>;
    fn save_events(&mut self, new_event: NewEvent) -> Result<(), StorageError>;
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
                log::info!(
                    target: LOG_TARGET,
                    "Updated substate {} version to {}",
                    address,
                    new_substate.version
                );
            },
            None => {
                diesel::insert_into(substates::table)
                    .values(&new_substate)
                    .execute(&mut *conn)
                    .map_err(|e| StorageError::QueryError {
                        reason: format!("Update substate error: {}", e),
                    })?;
                log::info!(
                    target: LOG_TARGET,
                    "Added new substate {} with version {}",
                    address,
                    new_substate.version
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

        log::info!(
            target: LOG_TARGET,
            "Added new NFT index for resource {} with index {}",
            new_nft_index.resource_address,
            new_nft_index.idx
        );

        Ok(())
    }

    fn save_events(&mut self, new_event: NewEvent) -> Result<(), StorageError> {
        use crate::substate_storage_sqlite::schema::events;

        diesel::insert_into(events::table)
            .values(&new_event)
            .execute(&mut *self.connection())
            .map_err(|e| StorageError::QueryError {
                reason: format!("events: {}", e),
            })?;

        log::info!(
            target: LOG_TARGET,
            "Added a new event with template_address = {} and tx_hash = {}",
            new_event.template_address,
            new_event.tx_hash
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
