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
    collections::{HashMap, HashSet},
    convert::{TryFrom, TryInto},
    fmt::{Debug, Formatter},
    marker::PhantomData,
    ops::RangeInclusive,
    sync::{Arc, Mutex},
};

use diesel::{
    prelude::*,
    sql_query,
    sql_types::{BigInt, Integer},
    RunQueryDsl,
    SqliteConnection,
};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use serde::{de::DeserializeOwned, Serialize};
use tari_common_types::types::PublicKey;
use tari_dan_common_types::{
    committee::Committee,
    hashing::ValidatorNodeBalancedMerkleTree,
    shard::Shard,
    Epoch,
    NodeAddressable,
    SubstateAddress,
};
use tari_dan_storage::{
    global::{
        models::ValidatorNode,
        DbEpoch,
        DbTemplate,
        DbTemplateUpdate,
        GlobalDbAdapter,
        MetadataKey,
        TemplateStatus,
    },
    AtomicDb,
};
use tari_utilities::ByteArray;

use super::{models, models::DbValidatorNode};
use crate::{
    error::SqliteStorageError,
    global::{
        models::{MetadataModel, NewEpoch, NewTemplateModel, TemplateModel, TemplateUpdateModel},
        schema::templates,
        serialization::serialize_json,
    },
    SqliteTransaction,
};

pub struct SqliteGlobalDbAdapter<TAddr> {
    connection: Arc<Mutex<SqliteConnection>>,
    _addr: PhantomData<TAddr>,
}

impl<TAddr> SqliteGlobalDbAdapter<TAddr> {
    pub fn new(connection: SqliteConnection) -> Self {
        Self {
            connection: Arc::new(Mutex::new(connection)),
            _addr: PhantomData,
        }
    }

    fn exists(&self, tx: &mut SqliteTransaction<'_>, key: MetadataKey) -> Result<bool, SqliteStorageError> {
        use crate::global::schema::metadata;
        let result = metadata::table
            .filter(metadata::key_name.eq(key.as_key_bytes()))
            .count()
            .limit(1)
            .get_result::<i64>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "exists::metadata".to_string(),
            })?;
        Ok(result > 0)
    }

    pub fn migrate(&self) -> Result<(), SqliteStorageError> {
        const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./global_db_migrations");
        self.connection
            .lock()
            .unwrap()
            .run_pending_migrations(MIGRATIONS)
            .map_err(|source| SqliteStorageError::MigrationError { source })?;
        Ok(())
    }
}

impl<TAddr> AtomicDb for SqliteGlobalDbAdapter<TAddr> {
    type DbTransaction<'a> = SqliteTransaction<'a>;
    type Error = SqliteStorageError;

    fn create_transaction(&self) -> Result<Self::DbTransaction<'_>, Self::Error> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(tx)
    }

    fn commit(&self, transaction: Self::DbTransaction<'_>) -> Result<(), Self::Error> {
        transaction.commit()
    }
}

impl<TAddr: NodeAddressable> GlobalDbAdapter for SqliteGlobalDbAdapter<TAddr> {
    type Addr = TAddr;

    fn set_metadata<T: Serialize>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: MetadataKey,
        value: &T,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::metadata;
        let value = serde_json::to_vec(value)?;
        match self.exists(tx, key) {
            Ok(true) => diesel::update(metadata::table)
                .filter(metadata::key_name.eq(key.as_key_bytes()))
                .set(metadata::value.eq(value))
                .execute(tx.connection())
                .map_err(|source| SqliteStorageError::DieselError {
                    source,
                    operation: "update::metadata".to_string(),
                })?,
            Ok(false) => diesel::insert_into(metadata::table)
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

    fn get_metadata<T: DeserializeOwned>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &MetadataKey,
    ) -> Result<Option<T>, Self::Error> {
        use crate::global::schema::metadata::dsl;

        let row: Option<MetadataModel> = dsl::metadata
            .find(key.as_key_bytes())
            .first(tx.connection())
            .optional()
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::metadata_key".to_string(),
            })?;

        let v = row.map(|r| serde_json::from_slice(&r.value)).transpose()?;
        Ok(v)
    }

    fn get_template(&self, tx: &mut Self::DbTransaction<'_>, key: &[u8]) -> Result<Option<DbTemplate>, Self::Error> {
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

                expected_hash: t.expected_hash.try_into()?,
                template_address: t.template_address.try_into()?,
                url: t.url,
                height: t.height as u64,
                template_type: t.template_type.parse().expect("DB template type corrupted"),
                compiled_code: t.compiled_code,
                flow_json: t.flow_json,
                manifest: t.manifest,
                status: t.status.parse().expect("DB status corrupted"),
                added_at: t.added_at,
            })),
            None => Ok(None),
        }
    }

    fn get_templates(&self, tx: &mut Self::DbTransaction<'_>, limit: usize) -> Result<Vec<DbTemplate>, Self::Error> {
        use crate::global::schema::templates::dsl;
        let mut templates = dsl::templates
            .filter(templates::status.eq(TemplateStatus::Active.as_str()))
            .into_boxed();

        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        if limit > 0 {
            templates = templates.limit(limit);
        }
        let templates = templates
            .get_results::<TemplateModel>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get_templates".to_string(),
            })?;

        templates
            .into_iter()
            .map(|t| {
                Ok(DbTemplate {
                    template_name: t.template_name,
                    expected_hash: t.expected_hash.try_into()?,
                    template_address: t.template_address.try_into()?,
                    url: t.url,
                    height: t.height as u64,
                    template_type: t.template_type.parse().expect("DB template type corrupted"),
                    compiled_code: t.compiled_code,
                    flow_json: t.flow_json,
                    manifest: t.manifest,
                    status: t.status.parse().expect("DB status corrupted"),
                    added_at: t.added_at,
                })
            })
            .collect()
    }

    fn get_pending_templates(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        limit: usize,
    ) -> Result<Vec<DbTemplate>, Self::Error> {
        use crate::global::schema::templates::dsl;
        let templates = dsl::templates
            .filter(templates::status.eq(TemplateStatus::Pending.as_str()))
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .get_results::<TemplateModel>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get_pending_template".to_string(),
            })?;

        templates
            .into_iter()
            .map(|t| {
                Ok(DbTemplate {
                    template_name: t.template_name,
                    expected_hash: t.expected_hash.try_into()?,
                    template_address: t.template_address.try_into()?,
                    url: t.url,
                    height: t.height as u64,
                    template_type: t.template_type.parse().expect("DB template type corrupted"),
                    compiled_code: t.compiled_code,
                    flow_json: t.flow_json,
                    manifest: t.manifest,
                    status: t.status.parse().expect("DB status corrupted"),
                    added_at: t.added_at,
                })
            })
            .collect()
    }

    fn insert_template(&self, tx: &mut Self::DbTransaction<'_>, item: DbTemplate) -> Result<(), Self::Error> {
        let new_template = NewTemplateModel {
            template_name: item.template_name,
            expected_hash: item.expected_hash.to_vec(),
            template_address: item.template_address.to_vec(),
            url: item.url.to_string(),
            height: item.height as i64,
            template_type: item.template_type.as_str().to_string(),
            compiled_code: item.compiled_code,
            flow_json: item.flow_json,
            status: item.status.as_str().to_string(),
            wasm_path: None,
            manifest: None,
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
        tx: &mut Self::DbTransaction<'_>,
        key: &[u8],
        template: DbTemplateUpdate,
    ) -> Result<(), Self::Error> {
        let model = TemplateUpdateModel {
            compiled_code: template.compiled_code,
            flow_json: template.flow_json,
            manifest: template.manifest,
            status: template.status.map(|s| s.as_str().to_string()),
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

    fn template_exists(&self, tx: &mut Self::DbTransaction<'_>, key: &[u8]) -> Result<bool, Self::Error> {
        use crate::global::schema::templates::dsl;
        let result = dsl::templates
            .filter(templates::template_address.eq(key))
            .count()
            .limit(1)
            .get_result::<i64>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "exists::metadata".to_string(),
            })?;
        Ok(result > 0)
    }

    fn insert_validator_node(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        address: Self::Addr,
        public_key: PublicKey,
        shard_key: SubstateAddress,
        epoch: Epoch,
        fee_claim_public_key: PublicKey,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::validator_nodes;

        // TODO: We update records for this validator node (incl all previous records) with the latest fee claim public
        //       key. This is hacky and this behaviour is not clear to the caller nor trait implementor.
        diesel::update(validator_nodes::table)
            .filter(validator_nodes::public_key.eq(ByteArray::as_bytes(&public_key)))
            .set(validator_nodes::fee_claim_public_key.eq(ByteArray::as_bytes(&fee_claim_public_key)))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert::validator_nodes".to_string(),
            })?;

        diesel::insert_into(validator_nodes::table)
            .values((
                validator_nodes::address.eq(serialize_json(&address)?),
                validator_nodes::public_key.eq(ByteArray::as_bytes(&public_key)),
                validator_nodes::shard_key.eq(shard_key.as_bytes()),
                validator_nodes::epoch.eq(epoch.as_u64() as i64),
                validator_nodes::fee_claim_public_key.eq(ByteArray::as_bytes(&fee_claim_public_key)),
            ))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert::validator_nodes".to_string(),
            })?;

        Ok(())
    }

    fn get_validator_node_by_public_key(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        public_key: &PublicKey,
    ) -> Result<ValidatorNode<Self::Addr>, Self::Error> {
        use crate::global::schema::{validator_nodes, validator_nodes::dsl};

        let vn = dsl::validator_nodes
            .filter(validator_nodes::epoch.ge(start_epoch.as_u64() as i64))
            .filter(validator_nodes::epoch.le(end_epoch.as_u64() as i64))
            .filter(validator_nodes::public_key.eq(ByteArray::as_bytes(public_key)))
            // Last one inserted
            .order_by(validator_nodes::id.desc())
            .first::<DbValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::validator_node".to_string(),
            })?;

        let vn = vn.try_into()?;
        Ok(vn)
    }

    fn validator_nodes_count(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
    ) -> Result<u64, Self::Error> {
        let count =
            sql_query("SELECT COUNT(distinct public_key) as cnt FROM validator_nodes WHERE epoch >= ? AND epoch <= ?")
                .bind::<BigInt, _>(start_epoch.as_u64() as i64)
                .bind::<BigInt, _>(end_epoch.as_u64() as i64)
                .get_result::<Count>(tx.connection())
                .map_err(|source| SqliteStorageError::DieselError {
                    source,
                    operation: "count_validator_nodes".to_string(),
                })?;

        Ok(count.cnt as u64)
    }

    fn validator_nodes_count_for_bucket(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        bucket: Shard,
    ) -> Result<u64, Self::Error> {
        let count = sql_query(
            "SELECT COUNT(distinct public_key) as cnt FROM validator_nodes WHERE epoch >= ? AND epoch <= ? AND \
             committee_bucket = ?",
        )
        .bind::<BigInt, _>(start_epoch.as_u64() as i64)
        .bind::<BigInt, _>(end_epoch.as_u64() as i64)
        .bind::<Integer, _>(bucket.as_u32() as i32)
        .get_result::<Count>(tx.connection())
        .map_err(|source| SqliteStorageError::DieselError {
            source,
            operation: "count_validator_nodes".to_string(),
        })?;

        Ok(count.cnt as u64)
    }

    fn validator_nodes_set_committee_bucket(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        shard_key: SubstateAddress,
        bucket: Shard,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::validator_nodes;

        diesel::update(validator_nodes::table)
            .filter(validator_nodes::shard_key.eq(shard_key.as_bytes()))
            .set(validator_nodes::committee_bucket.eq(i64::from(bucket.as_u32())))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert::committee_bucket".to_string(),
            })?;

        Ok(())
    }

    fn validator_nodes_get_by_shard_range(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        shard_range: RangeInclusive<SubstateAddress>,
    ) -> Result<Vec<ValidatorNode<Self::Addr>>, Self::Error> {
        use crate::global::schema::validator_nodes;

        let validators: Vec<DbValidatorNode> = validator_nodes::table
            .filter(validator_nodes::epoch.le(end_epoch.as_u64() as i64))
            .filter(validator_nodes::epoch.ge(start_epoch.as_u64() as i64))
            // SQLite compares BLOB types using memcmp which, IIRC, compares bytes "left to right"/big-endian which is 
            // the same way convert shard IDs to 256-bit integers when allocating committee shards.
            .filter(validator_nodes::shard_key.ge(shard_range.start().as_bytes()))
            .filter(validator_nodes::shard_key.le(shard_range.end().as_bytes()))
            .order_by(validator_nodes::id.asc())
            .get_results(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "validator_nodes_get_by_shard_range".to_string(),
            })?;

        distinct_validators_sorted(validators)
    }

    fn validator_nodes_get_by_buckets(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        buckets: HashSet<Shard>,
    ) -> Result<HashMap<Shard, Committee<Self::Addr>>, Self::Error> {
        use crate::global::schema::validator_nodes;

        let validators: Vec<DbValidatorNode> = validator_nodes::table
            .filter(validator_nodes::epoch.le(end_epoch.as_u64() as i64))
            .filter(validator_nodes::epoch.ge(start_epoch.as_u64() as i64))
            .filter(validator_nodes::committee_bucket.eq_any(buckets.iter().map(|b| i64::from(b.as_u32()))))
            .order_by(validator_nodes::id.asc())
            .get_results(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "validator_nodes_get_by_buckets".to_string(),
            })?;

        let mut committees = HashMap::with_capacity(buckets.len());
        for bucket in buckets {
            committees.insert(bucket, Committee::empty());
        }

        for validator in distinct_validators_sorted(validators)? {
            let Some(bucket) = validator.committee_shard else {
                continue;
            };
            committees
                .get_mut(&bucket)
                .unwrap()
                .members
                .push((validator.address, validator.public_key));
        }

        Ok(committees)
    }

    fn get_validator_nodes_within_epochs(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<Self::Addr>>, Self::Error> {
        use crate::global::schema::{validator_nodes, validator_nodes::dsl};

        let sqlite_vns = dsl::validator_nodes
            .filter(validator_nodes::epoch.ge(start_epoch.as_u64() as i64))
            .filter(validator_nodes::epoch.le(end_epoch.as_u64() as i64))
            .order_by(validator_nodes::id.asc())
            .load::<DbValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: format!("get::get_validator_nodes_within_epochs({}, {})", start_epoch, end_epoch),
            })?;

        // TODO: Perhaps we should overwrite duplicate validator node entries for the epoch
        distinct_validators_sorted(sqlite_vns)
    }

    fn get_validator_node_by_address(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        start_epoch: Epoch,
        end_epoch: Epoch,
        address: &Self::Addr,
    ) -> Result<ValidatorNode<Self::Addr>, Self::Error> {
        use crate::global::schema::{validator_nodes, validator_nodes::dsl};

        let vn = dsl::validator_nodes
            .filter(validator_nodes::epoch.ge(start_epoch.as_u64() as i64))
            .filter(validator_nodes::epoch.le(end_epoch.as_u64() as i64))
            .filter(validator_nodes::address.eq(serialize_json(address)?))
            // Last one inserted
            .order_by(validator_nodes::id.desc())
            .first::<DbValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::validator_node".to_string(),
            })?;

        let vn = vn.try_into()?;
        Ok(vn)
    }

    fn insert_epoch(&self, tx: &mut Self::DbTransaction<'_>, epoch: DbEpoch) -> Result<(), Self::Error> {
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

    fn get_epoch(&self, tx: &mut Self::DbTransaction<'_>, epoch: u64) -> Result<Option<DbEpoch>, Self::Error> {
        use crate::global::schema::epochs::dsl;

        let query_res: Option<models::Epoch> = dsl::epochs
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

    fn insert_bmt(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: u64,
        bmt: ValidatorNodeBalancedMerkleTree,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::bmt_cache;

        diesel::insert_into(bmt_cache::table)
            .values((
                bmt_cache::epoch.eq(epoch as i64),
                bmt_cache::bmt.eq(serde_json::to_vec(&bmt)?),
            ))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert::bmt".to_string(),
            })?;

        Ok(())
    }

    fn get_bmt(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<Option<ValidatorNodeBalancedMerkleTree>, Self::Error> {
        use crate::global::schema::bmt_cache::dsl;

        let query_res: Option<models::Bmt> = dsl::bmt_cache
            .find(epoch.as_u64() as i64)
            .first(tx.connection())
            .optional()
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::bmt".to_string(),
            })?;
        match query_res {
            Some(bmt) => Ok(Some(serde_json::from_slice(&bmt.bmt)?)),
            None => Ok(None),
        }
    }
}

impl<TAddr> Debug for SqliteGlobalDbAdapter<TAddr> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteGlobalDbAdapter")
            .field("db", &"Arc<Mutex<SqliteConnection>>")
            .finish()
    }
}

impl<TAddr> Clone for SqliteGlobalDbAdapter<TAddr> {
    fn clone(&self) -> Self {
        Self {
            connection: self.connection.clone(),
            _addr: PhantomData,
        }
    }
}

fn distinct_validators<TAddr: NodeAddressable>(
    sqlite_vns: Vec<DbValidatorNode>,
) -> Result<Vec<ValidatorNode<TAddr>>, SqliteStorageError> {
    let mut db_vns = Vec::with_capacity(sqlite_vns.len());
    let mut dedup_map = HashMap::with_capacity(sqlite_vns.len());
    for (i, vn) in sqlite_vns.into_iter().enumerate() {
        if let Some(idx) = dedup_map.insert(vn.public_key.clone(), i) {
            *db_vns.get_mut(idx).unwrap() = None;
        }
        db_vns.push(Some(ValidatorNode::try_from(vn)?));
    }

    let db_vns = db_vns.into_iter().flatten().collect::<Vec<_>>();
    Ok(db_vns)
}

fn distinct_validators_sorted<TAddr: NodeAddressable>(
    sqlite_vns: Vec<DbValidatorNode>,
) -> Result<Vec<ValidatorNode<TAddr>>, SqliteStorageError> {
    let mut db_vns = distinct_validators(sqlite_vns)?;
    db_vns.sort_by(|a, b| a.shard_key.cmp(&b.shard_key));
    Ok(db_vns)
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    cnt: i64,
}
