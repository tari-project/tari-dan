//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    ops::{Deref, DerefMut},
    sync::MutexGuard,
};

use diesel::{
    sql_query,
    sql_types::{BigInt, Bool, Integer, Nullable, Text},
    OptionalExtension,
    QueryDsl,
    RunQueryDsl,
    SqliteConnection,
};
use log::*;
use serde::Serialize;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::QuorumCertificate;
use tari_dan_wallet_sdk::{
    models::{SubstateRecord, TransactionStatus, VersionedSubstateAddress},
    storage::{WalletStorageError, WalletStoreReader, WalletStoreWriter},
};
use tari_engine_types::{commit_result::FinalizeResult, substate::SubstateAddress, TemplateAddress};
use tari_transaction::Transaction;
use tari_utilities::hex::Hex;

use crate::{diesel::ExpressionMethods, reader::ReadTransaction, serialization::serialize_json};

const LOG_TARGET: &str = "auth::tari::dan::wallet_sdk::storage_sqlite::writer";

pub struct WriteTransaction<'a> {
    /// In SQLite any transaction is writable. We keep a ReadTransaction to satisfy the Deref requirement of the
    /// WalletStore.
    transaction: ReadTransaction<'a>,
}

impl<'a> WriteTransaction<'a> {
    pub fn new(connection: MutexGuard<'a, SqliteConnection>) -> Self {
        Self {
            transaction: ReadTransaction::new(connection),
        }
    }
}

impl WalletStoreWriter for WriteTransaction<'_> {
    fn commit(mut self) -> Result<(), WalletStorageError> {
        self.transaction.commit()?;
        Ok(())
    }

    fn rollback(mut self) -> Result<(), WalletStorageError> {
        self.transaction.rollback()?;
        Ok(())
    }

    fn key_manager_set_active_index(&mut self, branch: &str, index: u64) -> Result<(), WalletStorageError> {
        use crate::schema::key_manager_states;
        let index = i64::try_from(index)
            .map_err(|_| WalletStorageError::general("key_manager_set_index", "index is negative"))?;

        let maybe_active_id = key_manager_states::table
            .select(key_manager_states::id)
            .filter(key_manager_states::branch_seed.eq(branch))
            .filter(key_manager_states::index.eq(index))
            .limit(1)
            .first::<i32>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("key_manager_set_index", e))?;

        sql_query(
            "UPDATE key_manager_states SET `is_active` = 0, updated_at = CURRENT_TIMESTAMP WHERE branch_seed = ?",
        )
        .bind::<Text, _>(branch)
        .execute(self.connection())
        .map_err(|e| WalletStorageError::general("key_manager_set_index", e))?;

        if let Some(active_id) = maybe_active_id {
            sql_query("UPDATE key_manager_states SET `is_active` = 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
                .bind::<Integer, _>(active_id)
                .execute(self.connection())
                .map_err(|e| WalletStorageError::general("key_manager_set_index", e))?;
        } else {
            sql_query("INSERT INTO key_manager_states (branch_seed, `index`, is_active) VALUES (?, ?, 1)")
                .bind::<Text, _>(branch)
                .bind::<BigInt, _>(index)
                .execute(self.connection())
                .map_err(|e| WalletStorageError::general("key_manager_set_index", e))?;
        }

        Ok(())
    }

    fn config_set<T: Serialize>(&mut self, key: &str, value: &T, is_encrypted: bool) -> Result<(), WalletStorageError> {
        use crate::schema::config;

        let exists = config::table
            .filter(config::key.eq(key))
            .limit(1)
            .count()
            .get_result(self.connection())
            .map(|count: i64| count > 0)
            .map_err(|e| WalletStorageError::general("key_manager_set_index", e))?;

        if exists {
            sql_query("UPDATE config SET value = ?, is_encrypted = ?, updated_at = CURRENT_TIMESTAMP WHERE key = ?")
                .bind::<Text, _>(serialize_json(value)?)
                .bind::<Text, _>(key)
                .execute(self.connection())
                .map_err(|e| WalletStorageError::general("key_manager_set_index", e))?;
        } else {
            sql_query("INSERT INTO config (key, value, is_encrypted) VALUES (?, ?, ?)")
                .bind::<Text, _>(key)
                .bind::<Text, _>(serialize_json(value)?)
                .bind::<Bool, _>(is_encrypted)
                .execute(self.connection())
                .map_err(|e| WalletStorageError::general("key_manager_set_index", e))?;
        }

        Ok(())
    }

    fn transactions_insert(&mut self, transaction: &Transaction, is_dry_run: bool) -> Result<(), WalletStorageError> {
        let status = if is_dry_run {
            TransactionStatus::DryRun
        } else {
            TransactionStatus::New
        };

        sql_query(
            "INSERT INTO transactions (hash, instructions, sender_address, fee, signature, meta, status, dry_run) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind::<Text, _>(transaction.hash().to_string())
        .bind::<Text, _>(serialize_json(transaction.instructions())?)
        .bind::<Text, _>(transaction.sender_public_key().to_hex())
        .bind::<BigInt, _>(transaction.fee() as i64)
        .bind::<Text, _>(serialize_json(transaction.signature())?)
        .bind::<Text, _>(serialize_json(transaction.meta())?)
        .bind::<Text, _>(status.as_key_str())
        .bind::<Bool, _>(is_dry_run)
        .execute(self.connection())
        .map_err(|e| WalletStorageError::general("transactions_insert", e))?;

        Ok(())
    }

    fn transactions_set_result_and_status(
        &mut self,
        hash: FixedHash,
        result: Option<&FinalizeResult>,
        qcs: Option<&[QuorumCertificate]>,
        new_status: TransactionStatus,
    ) -> Result<(), WalletStorageError> {
        let num_rows = sql_query(
            "UPDATE transactions SET result = ?, status = ?, qcs = ?, updated_at = CURRENT_TIMESTAMP WHERE hash = ?",
        )
        .bind::<Nullable<Text>, _>(result.map(serialize_json).transpose()?)
        .bind::<Text, _>(new_status.as_key_str())
        .bind::<Nullable<Text>, _>(qcs.map(serialize_json).transpose()?)
        .bind::<Text, _>(hash.to_string())
        .execute(self.connection())
        .map_err(|e| WalletStorageError::general("transactions_set_result_and_status", e))?;

        if num_rows == 0 {
            return Err(WalletStorageError::NotFound {
                operation: "transactions_set_result_and_status",
                entity: "transaction".to_string(),
                key: hash.to_string(),
            });
        }

        Ok(())
    }

    fn substates_insert_parent(
        &mut self,
        tx_hash: FixedHash,
        substate: VersionedSubstateAddress,
        module_name: String,
        template_addr: TemplateAddress,
    ) -> Result<(), WalletStorageError> {
        sql_query(
            "INSERT INTO substates (module_name, address, parent_address, transaction_hash, template_address, \
             version) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind::<Nullable<Text>, _>(Some(module_name))
        .bind::<Text, _>(substate.address.to_string())
        .bind::<Nullable<Text>, _>(None::<String>)
        .bind::<Text, _>(tx_hash.to_string())
        .bind::<Nullable<Text>, _>(Some(template_addr.to_string()))
        .bind::<Integer, _>(substate.version as i32)
        .execute(self.connection())
        .map_err(|e| WalletStorageError::general("substates_insert", e))?;

        Ok(())
    }

    fn substates_insert_child(
        &mut self,
        tx_hash: FixedHash,
        parent: SubstateAddress,
        child: VersionedSubstateAddress,
    ) -> Result<(), WalletStorageError> {
        sql_query("INSERT INTO substates (transaction_hash, address, parent_address, version) VALUES (?, ?, ?, ?)")
            .bind::<Text, _>(tx_hash.to_string())
            .bind::<Text, _>(child.address.to_string())
            .bind::<Nullable<Text>, _>(Some(parent.to_string()))
            .bind::<Integer, _>(child.version as i32)
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("substates_insert", e))?;
        Ok(())
    }

    fn substates_remove(
        &mut self,
        substate_addr: &VersionedSubstateAddress,
    ) -> Result<SubstateRecord, WalletStorageError> {
        let substate = self.transaction.substates_get(&substate_addr.address)?;
        let num_rows = sql_query("DELETE FROM substates WHERE address = ? AND version = ?")
            .bind::<Text, _>(substate_addr.address.to_string())
            .bind::<Integer, _>(substate_addr.version as i32)
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("substates_remove", e))?;

        if num_rows == 0 {
            return Err(WalletStorageError::NotFound {
                operation: "substates_remove",
                entity: "substate".to_string(),
                key: substate.address.to_string(),
            });
        }

        Ok(substate)
    }

    fn accounts_insert(
        &mut self,
        account_name: &str,
        address: &SubstateAddress,
        owner_key_index: u64,
    ) -> Result<(), WalletStorageError> {
        sql_query("INSERT INTO accounts (name, address, owner_key_index) VALUES (?, ?, ?)")
            .bind::<Text, _>(account_name)
            .bind::<Text, _>(address.to_string())
            .bind::<BigInt, _>(owner_key_index as i64)
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("accounts_insert", e))?;

        Ok(())
    }
}

impl Drop for WriteTransaction<'_> {
    fn drop(&mut self) {
        if !self.transaction.is_done() {
            warn!(target: LOG_TARGET, "WriteTransaction was not committed or rolled back");
            if let Err(err) = self.transaction.rollback() {
                warn!(target: LOG_TARGET, "Failed to rollback WriteTransaction: {}", err);
            }
        }
    }
}

impl<'a> Deref for WriteTransaction<'a> {
    type Target = ReadTransaction<'a>;

    fn deref(&self) -> &Self::Target {
        &self.transaction
    }
}

impl<'a> DerefMut for WriteTransaction<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.transaction
    }
}
