//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that
// the  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the
// following  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED
// WARRANTIES,  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
// PARTICULAR PURPOSE ARE  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL,  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
// PROCUREMENT OF SUBSTITUTE GOODS OR  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
// CAUSED AND ON ANY THEORY OF LIABILITY,  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
// OTHERWISE) ARISING IN ANY WAY OUT OF THE  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH
// DAMAGE.
use std::{
    collections::{HashMap, HashSet},
    convert::{TryFrom, TryInto},
    fmt::{Debug, Formatter},
    marker::PhantomData,
    sync::{Arc, Mutex, MutexGuard},
};

use diesel::{
    BoolExpressionMethods,
    ExpressionMethods,
    JoinOnDsl,
    OptionalExtension,
    QueryDsl,
    RunQueryDsl,
    SqliteConnection,
    sql_query,
    sql_types::{BigInt, Bigint},
};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use log::debug;
use serde::{Serialize, de::DeserializeOwned};
use tari_common_types::types::FixedHash;
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    ShardGroup,
    SubstateAddress,
    ValidatorSetHash,
    VotePower,
    committee::{Committee, CommitteeMember},
};
use tari_ootle_storage::{
    AtomicDb,
    global::{BlockHeaderModel, EpochData, GlobalDbAdapter, models::ValidatorNode},
};
use tari_template_lib::types::{Hash32, crypto::RistrettoPublicKeyBytes};
use tari_utilities::ByteArray;

use super::{models, models::DbValidatorNode};
use crate::{
    SqliteTransaction,
    error::SqliteStorageError,
    global::{
        models::{DbCommittee, MetadataModel},
        serialization::serialize_json,
    },
};

const LOG_TARGET: &str = "tari::ootle::storage_sqlite::global::backend_adapter";

define_sql_function! {
    #[sql_name = "COALESCE"]
    fn coalesce_bigint(x: diesel::sql_types::Nullable<Bigint>, y: BigInt) -> BigInt;
}
define_sql_function! {
    #[sql_name = "random"]
    fn sql_random() -> Integer;
}

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

    fn exists(
        &self,
        tx: &mut SqliteTransaction<MutexGuard<'_, SqliteConnection>>,
        key: &[u8],
    ) -> Result<bool, SqliteStorageError> {
        use crate::global::schema::metadata;
        let result = metadata::table
            .filter(metadata::key_name.eq(key))
            .count()
            .limit(1)
            .get_result::<i64>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "exists::metadata",
            })?;
        Ok(result > 0)
    }

    pub fn migrate(&self) -> Result<(), SqliteStorageError> {
        const MIGRATIONS: EmbeddedMigrations = embed_migrations!("./migrations");
        self.connection
            .lock()
            .unwrap()
            .run_pending_migrations(MIGRATIONS)
            .map_err(|source| SqliteStorageError::MigrationError { source })?;
        Ok(())
    }
}

impl<TAddr> AtomicDb for SqliteGlobalDbAdapter<TAddr> {
    type DbTransaction<'a> = SqliteTransaction<MutexGuard<'a, SqliteConnection>>;
    type Error = SqliteStorageError;

    fn create_transaction(&self) -> Result<Self::DbTransaction<'_>, Self::Error> {
        let tx = SqliteTransaction::begin(self.connection.lock().unwrap())?;
        Ok(tx)
    }

    fn commit(&self, transaction: Self::DbTransaction<'_>) -> Result<(), Self::Error> {
        transaction.commit()
    }

    fn rollback(&self, transaction: Self::DbTransaction<'_>) -> Result<(), Self::Error> {
        transaction.rollback()
    }
}

impl<TAddr: NodeAddressable> GlobalDbAdapter for SqliteGlobalDbAdapter<TAddr> {
    type Addr = TAddr;

    fn get_metadata<T: DeserializeOwned>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &[u8],
    ) -> Result<Option<T>, Self::Error> {
        use crate::global::schema::metadata;

        let row: Option<MetadataModel> =
            metadata::table
                .find(key)
                .first(tx.connection())
                .optional()
                .map_err(|source| SqliteStorageError::DieselError {
                    source,
                    operation: "get::metadata_key",
                })?;

        let v = row.map(|r| serde_json::from_slice(&r.value)).transpose()?;
        Ok(v)
    }

    fn set_metadata<T: Serialize>(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        key: &[u8],
        value: &T,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::metadata;
        let value = serde_json::to_vec(value)?;
        match self.exists(tx, key) {
            Ok(true) => diesel::update(metadata::table)
                .filter(metadata::key_name.eq(key))
                .set(metadata::value.eq(value))
                .execute(tx.connection())
                .map_err(|source| SqliteStorageError::DieselError {
                    source,
                    operation: "update::metadata",
                })?,
            Ok(false) => diesel::insert_into(metadata::table)
                .values((metadata::key_name.eq(key), metadata::value.eq(value)))
                .execute(tx.connection())
                .map_err(|source| SqliteStorageError::DieselError {
                    source,
                    operation: "insert::metadata",
                })?,
            Err(e) => return Err(e),
        };

        Ok(())
    }

    fn insert_validator_node(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        address: Self::Addr,
        public_key: RistrettoPublicKeyBytes,
        shard_key: SubstateAddress,
        start_epoch: Epoch,
        fee_claim_public_key: RistrettoPublicKeyBytes,
        power: VotePower,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::validator_nodes;
        let addr = serialize_json(&address)?;

        diesel::insert_into(validator_nodes::table)
            .values((
                validator_nodes::address.eq(&addr),
                validator_nodes::public_key.eq(public_key.as_bytes()),
                validator_nodes::shard_key.eq(shard_key.as_bytes()),
                validator_nodes::start_epoch.eq(start_epoch.as_u64() as i64),
                validator_nodes::fee_claim_public_key.eq(fee_claim_public_key.as_bytes()),
                validator_nodes::power.eq(power.value() as i64),
            ))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert::validator_nodes",
            })?;

        Ok(())
    }

    fn deactivate_validator_node(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        public_key: RistrettoPublicKeyBytes,
        deactivation_epoch: Epoch,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::validator_nodes;

        diesel::update(validator_nodes::table)
            .set(validator_nodes::end_epoch.eq(deactivation_epoch.as_u64() as i64))
            .filter(validator_nodes::public_key.eq(public_key.as_bytes()))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "remove::validator_nodes",
            })?;

        Ok(())
    }

    fn get_validator_nodes_within_start_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<Self::Addr>>, Self::Error> {
        use crate::global::schema::validator_nodes;

        let sqlite_vns = validator_nodes::table
            .filter(validator_nodes::start_epoch.le(epoch.as_u64() as i64))
            .filter(
                validator_nodes::end_epoch
                    .is_null()
                    .or(validator_nodes::end_epoch.gt(epoch.as_u64() as i64)),
            )
            .get_results::<DbValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::get_validator_nodes_within_epochs",
            })?;

        distinct_validators_sorted(sqlite_vns)
    }

    fn get_validator_nodes_within_committee_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<Vec<ValidatorNode<Self::Addr>>, Self::Error> {
        use crate::global::schema::{committees, validator_nodes};

        let sqlite_vns = validator_nodes::table
            .select(validator_nodes::all_columns)
            .inner_join(committees::table.on(validator_nodes::id.eq(committees::validator_node_id)))
            .filter(committees::epoch.eq(epoch.as_u64() as i64))
            .order_by(validator_nodes::shard_key.asc())
            .get_results::<DbValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::get_validator_nodes_within_epochs",
            })?;

        sqlite_vns.into_iter().map(TryInto::try_into).collect()
    }

    fn get_validator_node_by_address(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        address: &Self::Addr,
    ) -> Result<ValidatorNode<Self::Addr>, Self::Error> {
        use crate::global::schema::{committees, validator_nodes};

        let vn = validator_nodes::table
            .select(validator_nodes::all_columns)
            .inner_join(committees::table.on(validator_nodes::id.eq(committees::validator_node_id)))
            .filter(committees::epoch.eq(epoch.as_u64() as i64))
            .filter(validator_nodes::address.eq(serialize_json(address)?))
            .order_by(validator_nodes::id.desc())
            .first::<DbValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::validator_node",
            })?;

        let vn = vn.try_into()?;
        Ok(vn)
    }

    fn get_validator_node_by_public_key(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        public_key: &RistrettoPublicKeyBytes,
    ) -> Result<ValidatorNode<Self::Addr>, Self::Error> {
        use crate::global::schema::{committees, validator_nodes};

        let vn = validator_nodes::table
            .select(validator_nodes::all_columns)
            .inner_join(committees::table.on(validator_nodes::id.eq(committees::validator_node_id)))
            .filter(committees::epoch.eq(epoch.as_u64() as i64))
            .filter(validator_nodes::public_key.eq(public_key.as_bytes()))
            .order_by(validator_nodes::shard_key.desc())
            .first::<DbValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::validator_node",
            })?;

        let vn = vn.try_into()?;
        Ok(vn)
    }

    fn validator_nodes_count(&self, tx: &mut Self::DbTransaction<'_>, epoch: Epoch) -> Result<u64, Self::Error> {
        let count = sql_query(
            "SELECT COUNT(distinct public_key) as cnt FROM validator_nodes WHERE start_epoch <= ? AND (end_epoch IS \
             NULL OR end_epoch > ?)",
        )
        .bind::<BigInt, _>(epoch.as_u64() as i64)
        .bind::<BigInt, _>(epoch.as_u64() as i64)
        .get_result::<Count>(tx.connection())
        .map_err(|source| SqliteStorageError::DieselError {
            source,
            operation: "count_validator_nodes",
        })?;

        Ok(count.cnt as u64)
    }

    fn validator_nodes_count_for_shard_group(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<u64, Self::Error> {
        use crate::global::schema::committees;

        let count = committees::table
            .filter(committees::epoch.eq(epoch.as_u64() as i64))
            .filter(committees::shard_start.eq(shard_group.start().as_u32() as i32))
            .filter(committees::shard_end.eq(shard_group.end().as_u32() as i32))
            .count()
            .get_result::<i64>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "count_validator_nodes",
            })?;

        Ok(count as u64)
    }

    fn validator_nodes_clear_committees(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::committees;

        diesel::delete(committees::table.filter(committees::epoch.eq(epoch.as_u64() as i64)))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "delete::committees",
            })?;
        Ok(())
    }

    fn validator_nodes_set_committee_shard(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        shard_key: SubstateAddress,
        shard_group: ShardGroup,
        epoch: Epoch,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::{committees, validator_nodes};
        // This is probably not the most robust way of doing this. Ideally you would pass the validator ID to the
        // function and use that to insert into the committees table.
        let validator_id = validator_nodes::table
            .select(validator_nodes::id)
            .filter(validator_nodes::shard_key.eq(shard_key.as_bytes()))
            .filter(validator_nodes::start_epoch.le(epoch.as_u64() as i64))
            .order_by(validator_nodes::id.desc())
            .first::<i32>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "validator_nodes_set_committee_bucket",
            })?;

        diesel::insert_into(committees::table)
            .values((
                committees::validator_node_id.eq(validator_id),
                committees::epoch.eq(epoch.as_u64() as i64),
                committees::shard_start.eq(shard_group.start().as_u32() as i32),
                committees::shard_end.eq(shard_group.end().as_u32() as i32),
            ))
            .on_conflict((committees::validator_node_id, committees::epoch))
            .do_update()
            .set((
                committees::shard_start.eq(shard_group.start().as_u32() as i32),
                committees::shard_end.eq(shard_group.end().as_u32() as i32),
            ))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "upsert::committee_bucket",
            })?;
        Ok(())
    }

    fn validator_nodes_get_for_shard_group(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        shard_group: ShardGroup,
        limit: usize,
    ) -> Result<Committee<Self::Addr>, Self::Error> {
        use crate::global::schema::{committees, validator_nodes};

        let validators = validator_nodes::table
            .inner_join(committees::table.on(committees::validator_node_id.eq(validator_nodes::id)))
            .select(validator_nodes::all_columns)
            .filter(committees::epoch.eq(epoch.as_u64() as i64))
            .filter(committees::shard_start.eq(shard_group.start().as_u32() as i32))
            .filter(committees::shard_end.eq(shard_group.end().as_u32() as i32))
            .order_by(validator_nodes::public_key.asc())
            .limit(i64::try_from(limit).unwrap_or(i64::MAX))
            .get_results::<DbValidatorNode>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "validator_nodes_get_for_shard_group",
            })?;

        debug!(target: LOG_TARGET, "Found {} validators", validators.len());

        validators
            .into_iter()
            .map(|vn| {
                let address = DbValidatorNode::try_parse_address(&vn.address)?;
                let public_key = RistrettoPublicKeyBytes::from_bytes(&vn.public_key).map_err(|_| {
                    SqliteStorageError::MalformedDbData(format!(
                        "validator_nodes_get_for_shard_group: Invalid public key in validator node record id={}",
                        vn.id
                    ))
                })?;
                Ok(CommitteeMember {
                    address,
                    public_key,
                    vote_power: VotePower::of(vn.power as u64),
                })
            })
            .collect()
    }

    fn validator_nodes_get_overlapping_shard_group(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        shard_group: ShardGroup,
    ) -> Result<HashMap<ShardGroup, Committee<Self::Addr>>, Self::Error> {
        use crate::global::schema::{committees, validator_nodes};

        let validators = validator_nodes::table
            .inner_join(committees::table.on(committees::validator_node_id.eq(validator_nodes::id)))
            .select((validator_nodes::all_columns, committees::all_columns))
            .filter(committees::epoch.eq(epoch.as_u64() as i64))
            // Overlapping c.shard_start <= :end and c.shard_end >= :start;
            .filter(committees::shard_start.le(shard_group.end().as_u32() as i32))
            .filter(committees::shard_end.ge(shard_group.start().as_u32() as i32))
            .get_results::<(DbValidatorNode, DbCommittee)>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "validator_nodes_get_overlapping_shard_group",
            })?;

        debug!(target: LOG_TARGET, "Found {} validators", validators.len());

        let mut members = HashMap::<_, Vec<_>>::with_capacity(shard_group.len());
        for (vn, committee) in validators {
            members
                .entry(committee.as_shard_group())
                .or_default()
                .push(CommitteeMember {
                    address: DbValidatorNode::try_parse_address(&vn.address)?,
                    public_key: RistrettoPublicKeyBytes::from_bytes(&vn.public_key).map_err(|_| {
                        SqliteStorageError::MalformedDbData(format!(
                            "validator_nodes_get_overlapping_shard_group: Invalid public key in validator node record \
                             id={}",
                            vn.id
                        ))
                    })?,
                    vote_power: VotePower::of(vn.power as u64),
                });
        }

        Ok(members
            .into_iter()
            .map(|(shard_group, members)| (shard_group, Committee::new(members)))
            .collect())
    }

    fn validator_nodes_get_random_committee_member_from_shard_group(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        shard_group: Option<ShardGroup>,
        excluding: HashSet<Self::Addr>,
    ) -> Result<ValidatorNode<Self::Addr>, Self::Error> {
        use crate::global::schema::{committees, validator_nodes};

        let mut query = validator_nodes::table
            .inner_join(committees::table.on(validator_nodes::id.eq(committees::validator_node_id)))
            .select(validator_nodes::all_columns)
            .filter(committees::epoch.eq(epoch.as_u64() as i64))
            .filter(
                validator_nodes::address.ne_all(
                    excluding
                        .iter()
                        .map(|a| serialize_json(a).expect("serialize address to json")),
                ),
            )
            .order_by(sql_random())
            .into_boxed();

        if let Some(shard_group) = shard_group {
            query = query
                .filter(committees::shard_start.eq(shard_group.start().as_u32() as i32))
                .filter(committees::shard_end.eq(shard_group.end().as_u32() as i32));
        }

        let vn = query
            .first::<DbValidatorNode>(tx.connection())
            .optional()
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::validator_node",
            })?
            .ok_or(SqliteStorageError::NotFound {
                item: "validator_node",
                key: "random selection in validator_nodes_get_random_committee_member_from_shard_group".to_string(),
            })?;

        let vn = vn.try_into()?;
        Ok(vn)
    }

    fn validator_nodes_get_committees_for_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<HashMap<ShardGroup, Committee<Self::Addr>>, Self::Error> {
        use crate::global::schema::{committees, validator_nodes};

        let results = committees::table
            .inner_join(validator_nodes::table.on(committees::validator_node_id.eq(validator_nodes::id)))
            .select((
                committees::shard_start,
                committees::shard_end,
                validator_nodes::address,
                validator_nodes::public_key,
                validator_nodes::power,
            ))
            .filter(committees::epoch.eq(epoch.as_u64() as i64))
            .filter(
                validator_nodes::end_epoch
                    .is_null()
                    .or(validator_nodes::end_epoch.gt(epoch.as_u64() as i64)),
            )
            .load::<(i32, i32, String, Vec<u8>, i64)>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "validator_nodes_get_committees",
            })?;

        let mut members = HashMap::<_, Vec<_>>::new();
        for (shard_start, shard_end, address, public_key, power) in results {
            let addr = DbValidatorNode::try_parse_address(&address)?;
            let pk = RistrettoPublicKeyBytes::from_bytes(&public_key)
                .map_err(|_| SqliteStorageError::MalformedDbData("Invalid public key".to_string()))?;
            members
                .entry(ShardGroup::new(shard_start as u32, shard_end as u32))
                .or_default()
                .push(CommitteeMember {
                    address: addr,
                    public_key: pk,
                    vote_power: VotePower::of(power as u64),
                });
        }

        Ok(members
            .into_iter()
            .map(|(shard_group, members)| (shard_group, Committee::new(members)))
            .collect())
    }

    fn insert_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        epoch_hash: FixedHash,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::epochs;

        // Upsert: the base-layer scanner re-emits EpochChanged for an epoch it has already persisted
        // when a reorg near the epoch boundary surfaces a different boundary-block hash. The epoch
        // manager only forwards such a correction while the epoch is still unlocked (see
        // EpochManagerService::activate_epoch), so overwriting the stored hash here is the intended
        // self-heal. A plain INSERT would hit the UNIQUE(epoch) constraint and abort the correction.
        diesel::insert_into(epochs::table)
            .values((
                epochs::epoch.eq(epoch.as_u64() as i64),
                epochs::epoch_hash.eq(epoch_hash.as_slice()),
            ))
            .on_conflict(epochs::epoch)
            .do_update()
            .set(epochs::epoch_hash.eq(epoch_hash.as_slice()))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert::epoch",
            })?;

        Ok(())
    }

    fn get_epoch(&self, tx: &mut Self::DbTransaction<'_>, epoch: Epoch) -> Result<Option<EpochData>, Self::Error> {
        use crate::global::schema::epochs::dsl;

        let query_res: Option<models::DbEpochData> = dsl::epochs
            .find(epoch.as_u64() as i64)
            .first(tx.connection())
            .optional()
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::epoch",
            })?;

        query_res.map(EpochData::try_from).transpose()
    }

    fn set_validator_set(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
        validator_set_hash: &ValidatorSetHash,
        leaves: &[(ShardGroup, Hash32)],
    ) -> Result<(), Self::Error> {
        use crate::global::schema::{epochs, validator_set_leaves};

        let epoch = epoch.as_u64() as i64;
        let num_updated = diesel::update(epochs::table.find(epoch))
            .set(epochs::validator_set_hash.eq(validator_set_hash.as_bytes()))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "set_validator_set::epoch",
            })?;
        if num_updated == 0 {
            return Err(SqliteStorageError::NotFound {
                item: "epoch",
                key: format!("{epoch} (to set its validator set)"),
            });
        }

        diesel::delete(validator_set_leaves::table.filter(validator_set_leaves::epoch.eq(epoch)))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "set_validator_set::clear_leaves",
            })?;
        let rows = leaves
            .iter()
            .map(|(shard_group, leaf_hash)| {
                (
                    validator_set_leaves::epoch.eq(epoch),
                    validator_set_leaves::shard_start.eq(shard_group.start().as_u32() as i32),
                    validator_set_leaves::shard_end.eq(shard_group.end().as_u32() as i32),
                    validator_set_leaves::leaf_hash.eq(leaf_hash.as_slice()),
                )
            })
            .collect::<Vec<_>>();
        if !rows.is_empty() {
            diesel::insert_into(validator_set_leaves::table)
                .values(&rows)
                .execute(tx.connection())
                .map_err(|source| SqliteStorageError::DieselError {
                    source,
                    operation: "set_validator_set::insert_leaves",
                })?;
        }
        Ok(())
    }

    fn get_validator_set_leaves(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<Vec<(ShardGroup, Hash32)>, Self::Error> {
        use crate::global::schema::validator_set_leaves;

        let rows = validator_set_leaves::table
            .select((
                validator_set_leaves::shard_start,
                validator_set_leaves::shard_end,
                validator_set_leaves::leaf_hash,
            ))
            .filter(validator_set_leaves::epoch.eq(epoch.as_u64() as i64))
            .order_by((
                validator_set_leaves::shard_start.asc(),
                validator_set_leaves::shard_end.asc(),
            ))
            .load::<(i32, i32, Vec<u8>)>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get_validator_set_leaves",
            })?;

        rows.into_iter()
            .map(|(shard_start, shard_end, leaf_hash)| {
                let shard_group = ShardGroup::new_checked(shard_start as u32, shard_end as u32).ok_or_else(|| {
                    SqliteStorageError::MalformedDbData(format!(
                        "get_validator_set_leaves: invalid shard group {shard_start}-{shard_end} for epoch {epoch}"
                    ))
                })?;
                let leaf_hash = Hash32::try_from_slice(&leaf_hash).map_err(|_| {
                    SqliteStorageError::MalformedDbData(format!(
                        "get_validator_set_leaves: leaf hash of shard group {shard_group} in epoch {epoch} has {} \
                         bytes, expected 32",
                        leaf_hash.len()
                    ))
                })?;
                Ok((shard_group, leaf_hash))
            })
            .collect()
    }

    fn insert_block_header(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        header: BlockHeaderModel,
    ) -> Result<(), Self::Error> {
        use crate::global::schema::block_headers;

        // Idempotent insert: the base-layer scanner may re-scan previously seen heights after a
        // base-layer reorg (see base_layer/oracle.rs::handle_reorg, which rewinds the scan position
        // to the fork point), in which case this insert would otherwise fail the UNIQUE(block_hash,
        // epoch) constraint and abort the scan. Swallowing duplicates is safe because (block_hash,
        // epoch) identifies the row.
        diesel::insert_into(block_headers::table)
            .values((
                block_headers::epoch.eq(header.epoch.as_u64() as i64),
                block_headers::height.eq(header.height as i64),
                block_headers::block_hash.eq(header.block_hash.as_bytes()),
                block_headers::block_output_merkle_root.eq(header.block_output_merkle_root.as_bytes()),
                block_headers::validator_node_merkle_root.eq(header.validator_node_merkle_root.as_bytes()),
            ))
            .on_conflict((block_headers::block_hash, block_headers::epoch))
            .do_nothing()
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "insert::block_header",
            })?;

        Ok(())
    }

    fn get_block_header_by_hash(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        max_epoch: Epoch,
        block_hash: &Hash32,
    ) -> Result<BlockHeaderModel, Self::Error> {
        use crate::global::schema::block_headers;

        let header = block_headers::table
            .filter(block_headers::block_hash.eq(block_hash.as_ref()))
            .filter(block_headers::epoch.le(max_epoch.as_u64() as i64))
            .first::<models::BlockHeaderModel>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::block_header_by_hash",
            })?;

        header.try_into()
    }

    fn get_first_block_header_by_epoch(
        &self,
        tx: &mut Self::DbTransaction<'_>,
        epoch: Epoch,
    ) -> Result<BlockHeaderModel, Self::Error> {
        use crate::global::schema::block_headers;

        let header = block_headers::table
            .filter(block_headers::epoch.eq(epoch.as_u64() as i64))
            .order_by(block_headers::height.asc())
            .first::<models::BlockHeaderModel>(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "get::first_block_header_by_epoch",
            })?;

        header.try_into()
    }

    fn delete_block_headers_above(&self, tx: &mut Self::DbTransaction<'_>, height: u64) -> Result<usize, Self::Error> {
        use crate::global::schema::block_headers;

        // Convert with try_from rather than `as`: a height above i64::MAX would wrap negative and the
        // `height > N` filter would then match every row, deleting all stored headers. No stored height
        // can exceed i64::MAX, so a height that large means there is nothing above it to delete.
        let Ok(height) = i64::try_from(height) else {
            return Ok(0);
        };

        let num_deleted = diesel::delete(block_headers::table.filter(block_headers::height.gt(height)))
            .execute(tx.connection())
            .map_err(|source| SqliteStorageError::DieselError {
                source,
                operation: "delete::block_headers_above",
            })?;

        Ok(num_deleted)
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
    mut sqlite_vns: Vec<DbValidatorNode>,
) -> Result<Vec<ValidatorNode<TAddr>>, SqliteStorageError> {
    // first, sort by registration block height so that we get newer registrations first
    let mut db_vns = Vec::with_capacity(sqlite_vns.len());
    sqlite_vns.sort_by(|a, b| a.start_epoch.cmp(&b.start_epoch).reverse());
    let mut dedup_map = HashSet::<Vec<u8>>::with_capacity(sqlite_vns.len());
    for vn in sqlite_vns {
        if !dedup_map.contains(&vn.public_key) {
            dedup_map.insert(vn.public_key.clone());
            db_vns.push(ValidatorNode::try_from(vn)?);
        }
    }

    Ok(db_vns)
}

fn distinct_validators_sorted<TAddr: NodeAddressable>(
    sqlite_vns: Vec<DbValidatorNode>,
) -> Result<Vec<ValidatorNode<TAddr>>, SqliteStorageError> {
    let mut db_vns = distinct_validators(sqlite_vns)?;
    db_vns.sort_by_key(|a| a.shard_key);
    Ok(db_vns)
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    cnt: i64,
}
