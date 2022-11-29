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
    convert::{TryFrom, TryInto},
    sync::{Arc, Mutex},
};

use diesel::{prelude::*, RunQueryDsl, SqliteConnection};
use tari_dan_storage::{
    global::{DbEpoch, DbTemplate, DbTemplateUpdate, DbValidatorNode, GlobalDbAdapter, MetadataKey, TemplateStatus},
    AtomicDb,
};

use super::models::{
    epoch::Epoch,
    validator_node::{NewValidatorNode, ValidatorNode},
};
use crate::{
    error::SqliteStorageError,
    global::{
        models::{epoch::NewEpoch, MetadataModel, NewTemplateModel, TemplateModel, TemplateUpdateModel},
        schema::templates,
    },
    SqliteTransaction,
};

#[derive(Clone)]
pub struct SqliteGlobalDbAdapter {
    connection: Arc<Mutex<SqliteConnection>>,
}

impl SqliteGlobalDbAdapter {
    pub fn new(connection: SqliteConnection) -> Self {
        Self {
            connection: Arc::new(Mutex::new(connection)),
        }
    }
}

impl AtomicDb for SqliteGlobalDbAdapter {
    type DbTransaction<'a> = SqliteTransaction<'a>;
    type Error = SqliteStorageError;

    fn create_transaction<'a>(&'a self) -> Result<Self::DbTransaction<'a>, Self::Error> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(tx)
    }

    fn commit(&self, transaction: Self::DbTransaction<'_>) -> Result<(), Self::Error> {
        transaction.commit()
    }
}

impl GlobalDbAdapter for SqliteGlobalDbAdapter {
    fn set_metadata(&self, tx: &Self::DbTransaction<'_>, key: MetadataKey, value: &[u8]) -> Result<(), Self::Error> {
        use crate::global::schema::metadata;
        match self.get_metadata(tx, &key) {
            Ok(Some(r)) => diesel::update(&MetadataModel {
                key_name: key.as_key_bytes().to_vec(),
                value: r,
            })
            .set(metadata::value.eq(value))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "update::metadata".to_string(),
            })?,
            Ok(None) => diesel::insert_into(metadata::table)
                .values((metadata::key_name.eq(key.as_key_bytes()), metadata::value.eq(value)))
                .execute(tx.connection())
                .map_err(|source| SqliteStorageError::DieselError {
                    source,
                    operation: "insert::metadata".to_string(),
                })?,
            Err(e) => return Err(e),
        };

        Ok(())
    }

    fn get_metadata(&self, tx: &Self::DbTransaction<'_>, key: &MetadataKey) -> Result<Option<Vec<u8>>, Self::Error> {
        use crate::global::schema::metadata::dsl;

        let row: Option<MetadataModel> = dsl::metadata
            .find(key.as_key_bytes())
            .first(tx.connection())
            .optional()
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::metadata_key".to_string(),
            })?;

        Ok(row.map(|r| r.value))
    }

    fn get_template(&self, tx: &Self::DbTransaction<'_>, key: &[u8]) -> Result<Option<DbTemplate>, Self::Error> {
        use crate::global::schema::templates::dsl;
        let template: Option<TemplateModel> = dsl::templates
            .filter(templates::template_address.eq(key))
            .first(tx.connection())
            .optional()
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get_template".to_string(),
            })?;

        match template {
            Some(t) => Ok(Some(DbTemplate {
                template_name: t.template_name,
                template_address: t.template_address.try_into()?,
                url: t.url,
                height: t.height as u64,
                compiled_code: t.compiled_code,
                status: t.status.parse().expect("DB status corrupted"),
                added_at: time::OffsetDateTime::from_unix_timestamp(t.added_at).expect("added_at timestamp corrupted"),
            })),
            None => Ok(None),
        }
    }

    fn get_templates(&self, tx: &Self::DbTransaction<'_>, limit: usize) -> Result<Vec<DbTemplate>, Self::Error> {
        use crate::global::schema::templates::dsl;
        let templates = dsl::templates
            .filter(templates::status.eq(TemplateStatus::Active.as_str()))
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .get_results::<TemplateModel>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get_template".to_string(),
            })?;

        templates
            .into_iter()
            .map(|t| {
                Ok(DbTemplate {
                    template_name: t.template_name,
                    template_address: t.template_address.try_into()?,
                    url: t.url,
                    height: t.height as u64,
                    compiled_code: t.compiled_code,
                    status: t.status.parse().expect("DB status corrupted"),
                    added_at: time::OffsetDateTime::from_unix_timestamp(t.added_at)
                        .expect("added_at timestamp corrupted"),
                })
            })
            .collect()
    }

    fn insert_template(&self, tx: &Self::DbTransaction<'_>, item: DbTemplate) -> Result<(), Self::Error> {
        let new_template = NewTemplateModel {
            template_name: item.template_name,
            template_address: item.template_address.to_vec(),
            url: item.url.to_string(),
            height: item.height as i32,
            compiled_code: item.compiled_code.clone(),
            status: item.status.as_str().to_string(),
            // TODO
            wasm_path: None,
            added_at: item.added_at.unix_timestamp(),
        };
        diesel::insert_into(templates::table)
            .values(new_template)
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert_template".to_string(),
            })?;

        Ok(())
    }

    fn update_template(
        &self,
        tx: &Self::DbTransaction<'_>,
        key: &[u8],
        template: DbTemplateUpdate,
    ) -> Result<(), Self::Error> {
        let model = TemplateUpdateModel {
            compiled_code: template.compiled_code,
            status: template.status.map(|s| s.as_str().to_string()),
            wasm_path: None,
        };
        diesel::update(templates::table)
            .filter(templates::template_address.eq(key))
            .set(model)
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "update_template".to_string(),
            })?;

        Ok(())
    }

    fn insert_validator_nodes(
        &self,
        tx: &Self::DbTransaction<'_>,
        validator_nodes: Vec<DbValidatorNode>,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::validator_nodes;

        let sqlite_vns: Vec<NewValidatorNode> = validator_nodes.into_iter().map(Into::into).collect();

        // Sqlite does not support batch transactions, so we need to insert each VN in a separated query
        for vn in sqlite_vns {
            diesel::insert_into(validator_nodes::table)
                .values(&vn)
                .execute(tx.connection())
                .map_err(|source| SqliteStorageError::DieselError {
                    source,
                    operation: "insert::validator_nodes".to_string(),
                })?;
        }

        Ok(())
    }

    fn get_validator_node(
        &self,
        tx: &Self::DbTransaction<'_>,
        epoch: u64,
        public_key: &[u8],
    ) -> Result<DbValidatorNode, Self::Error> {
        use crate::global::schema::{validator_nodes, validator_nodes::dsl};

        let vn = dsl::validator_nodes
            .filter(validator_nodes::epoch.eq(epoch as i64))
            .filter(validator_nodes::public_key.eq(public_key))
            .first::<ValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::validator_nodes_per_epoch".to_string(),
            })?;

        Ok(vn.into())
    }

    fn get_validator_nodes_per_epoch(
        &self,
        tx: &Self::DbTransaction<'_>,
        epoch: u64,
    ) -> Result<Vec<DbValidatorNode>, Self::Error> {
        use crate::global::schema::{validator_nodes, validator_nodes::dsl};

        let sqlite_vns = dsl::validator_nodes
            .filter(validator_nodes::epoch.eq(epoch as i64))
            .load::<ValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::validator_nodes_per_epoch".to_string(),
            })?;

        let db_vns: Vec<DbValidatorNode> = sqlite_vns.into_iter().map(Into::into).collect();

        Ok(db_vns)
    }

    fn insert_epoch(
        &self,
        tx: &Self::DbTransaction<'_>,
        epoch: tari_dan_storage::global::DbEpoch,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::epochs;

        let sqlite_epoch: NewEpoch = epoch.into();

        diesel::insert_into(epochs::table)
            .values(&sqlite_epoch)
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert::epoch".to_string(),
            })?;

        Ok(())
    }

    fn get_epoch(&self, tx: &Self::DbTransaction<'_>, epoch: u64) -> Result<Option<DbEpoch>, Self::Error> {
        use crate::global::schema::epochs::dsl;

        let query_res: Option<Epoch> = dsl::epochs
            .find(epoch as i64)
            .first(tx.connection())
            .optional()
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::epoch".to_string(),
            })?;

        match query_res {
            Some(e) => Ok(Some(e.into())),
            None => Ok(None),
        }
    }
}
