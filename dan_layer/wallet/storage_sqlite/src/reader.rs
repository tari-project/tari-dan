//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::MutexGuard;

use bigdecimal::{BigDecimal, ToPrimitive};
use diesel::{dsl::sum, sql_query, OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection};
use log::error;
use serde::de::DeserializeOwned;
use tari_common_types::types::FixedHash;
use tari_dan_wallet_sdk::{
    models::{
        Account,
        ConfidentialOutput,
        ConfidentialProofId,
        Config,
        OutputStatus,
        SubstateRecord,
        TransactionStatus,
        WalletTransaction,
    },
    storage::{WalletStorageError, WalletStoreReader},
};
use tari_engine_types::substate::{InvalidSubstateAddressFormat, SubstateAddress};

use crate::{diesel::ExpressionMethods, models, serialization::deserialize_json};

const LOG_TARGET: &str = "tari::dan::wallet_sdk::storage_sqlite::reader";

pub struct ReadTransaction<'a> {
    connection: MutexGuard<'a, SqliteConnection>,
    is_done: bool,
}

impl<'a> ReadTransaction<'a> {
    pub fn new(connection: MutexGuard<'a, SqliteConnection>) -> Self {
        Self {
            connection,
            is_done: false,
        }
    }

    pub(super) fn is_done(&self) -> bool {
        self.is_done
    }

    pub(super) fn connection(&mut self) -> &mut SqliteConnection {
        &mut self.connection
    }

    /// Internal commit
    pub(super) fn commit(&mut self) -> Result<(), WalletStorageError> {
        sql_query("COMMIT")
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("commit", e))?;
        self.is_done = true;
        Ok(())
    }

    /// Internal rollback
    pub(super) fn rollback(&mut self) -> Result<(), WalletStorageError> {
        sql_query("ROLLBACK")
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("rollback", e))?;
        self.is_done = true;
        Ok(())
    }
}

impl WalletStoreReader for ReadTransaction<'_> {
    fn key_manager_get_all(&mut self, branch: &str) -> Result<Vec<(u64, bool)>, WalletStorageError> {
        use crate::schema::key_manager_states;

        let results = key_manager_states::table
            .select((key_manager_states::index, key_manager_states::is_active))
            .filter(key_manager_states::branch_seed.eq(branch))
            .get_results::<(i64, bool)>(self.connection())
            .map_err(|e| WalletStorageError::general("key_manager_get_index", e))?;

        Ok(results
            .into_iter()
            .map(|(index, is_active)| (index as u64, is_active))
            .collect())
    }

    fn key_manager_get_active_index(&mut self, branch: &str) -> Result<u64, WalletStorageError> {
        use crate::schema::key_manager_states;

        key_manager_states::table
            .select(key_manager_states::index)
            .filter(key_manager_states::branch_seed.eq(branch))
            .filter(key_manager_states::is_active.eq(true))
            .first(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("key_manager_get_index", e))?
            .map(|index: i64| index as u64)
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "key_manager_get_index",
                entity: "key_manager_state".to_string(),
                key: branch.to_string(),
            })
    }

    // Config
    fn config_get<T: DeserializeOwned>(&mut self, key: &str) -> Result<Config<T>, WalletStorageError> {
        use crate::schema::config;

        let config = config::table
            .filter(config::key.eq(key))
            .first::<models::Config>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("config_get", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "config_get",
                entity: "config".to_string(),
                key: key.to_string(),
            })?;

        Ok(Config {
            key: config.key,
            value: deserialize_json(&config.value)?,
            is_encrypted: config.is_encrypted,
            created_at: 0,
            updated_at: 0,
        })
    }

    // Transactions
    fn transaction_get(&mut self, hash: FixedHash) -> Result<WalletTransaction, WalletStorageError> {
        use crate::schema::transactions;
        let row = transactions::table
            .filter(transactions::hash.eq(hash.to_string()))
            .first::<models::Transaction>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("transaction_get", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "transaction_get",
                entity: "transaction".to_string(),
                key: hash.to_string(),
            })?;

        let transaction = row.try_into_wallet_transaction()?;
        Ok(transaction)
    }

    fn transactions_fetch_all_by_status(
        &mut self,
        status: TransactionStatus,
    ) -> Result<Vec<WalletTransaction>, WalletStorageError> {
        use crate::schema::transactions;

        let rows = transactions::table
            .filter(transactions::status.eq(status.as_key_str()))
            .filter(transactions::dry_run.eq(false))
            .load::<models::Transaction>(self.connection())
            .map_err(|e| WalletStorageError::general("transactions_fetch_all_by_status", e))?;

        rows.into_iter().map(|row| row.try_into_wallet_transaction()).collect()
    }

    fn substates_get(&mut self, address: &SubstateAddress) -> Result<SubstateRecord, WalletStorageError> {
        use crate::schema::substates;

        let rec = substates::table
            .filter(substates::address.eq(address.to_string()))
            .first::<models::Substate>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("transactions_fetch_all_by_status", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "substates_get_root",
                entity: "substate".to_string(),
                key: address.to_string(),
            })?;

        let rec = rec.try_to_record()?;
        Ok(rec)
    }

    fn substates_get_children(&mut self, parent: &SubstateAddress) -> Result<Vec<SubstateRecord>, WalletStorageError> {
        use crate::schema::substates;

        let rows = substates::table
            .filter(substates::parent_address.eq(parent.to_string()))
            .get_results::<models::Substate>(self.connection())
            .map_err(|e| WalletStorageError::general("transactions_fetch_all_by_status", e))?;

        rows.into_iter().map(|rec| rec.try_to_record()).collect()
    }

    fn accounts_get_many(&mut self, limit: u64) -> Result<Vec<Account>, WalletStorageError> {
        use crate::schema::accounts;

        let rows = accounts::table
            .limit(limit as i64)
            .load::<models::Account>(self.connection())
            .map_err(|e| WalletStorageError::general("accounts_get_many", e))?;

        Ok(rows
            .into_iter()
            .map(|row| Account {
                name: row.name,
                address: row.address.parse().unwrap(),
                key_index: row.owner_key_index as u64,
            })
            .collect())
    }

    fn accounts_count(&mut self) -> Result<u64, WalletStorageError> {
        use crate::schema::accounts;

        let count = accounts::table
            .count()
            .first::<i64>(self.connection())
            .map_err(|e| WalletStorageError::general("account_count", e))?;

        Ok(count as u64)
    }

    fn accounts_get_by_name(&mut self, name: &str) -> Result<Account, WalletStorageError> {
        use crate::schema::accounts;

        let row = accounts::table
            .filter(accounts::name.eq(name))
            .first::<models::Account>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("accounts_get_by_name", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "accounts_get_by_name",
                entity: "account".to_string(),
                key: name.to_string(),
            })?;

        let account = row
            .try_into()
            .map_err(|e: InvalidSubstateAddressFormat| WalletStorageError::DecodingError {
                operation: "accounts_get_by_name",
                item: "account",
                details: e.to_string(),
            })?;
        Ok(account)
    }

    // Outputs
    fn outputs_get_unspent_balance(&mut self, account_name: &str) -> Result<u64, WalletStorageError> {
        use crate::schema::{accounts, outputs};

        let account_id = accounts::table
            .filter(accounts::name.eq(account_name))
            .select(accounts::id)
            .first::<i32>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_unspent_balance", e))?;

        let balance = outputs::table
            .select(sum(outputs::value))
            .filter(outputs::account_id.eq(account_id))
            .filter(outputs::status.eq(OutputStatus::Unspent.as_key_str()))
            .first::<Option<BigDecimal>>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_unspent_balance", e))?;

        Ok(balance.map(|v| v.to_u64().unwrap()).unwrap_or(0))
    }

    fn outputs_get_locked_by_proof(
        &mut self,
        proof_id: ConfidentialProofId,
    ) -> Result<Vec<ConfidentialOutput>, WalletStorageError> {
        use crate::schema::{accounts, outputs};

        let rows = outputs::table
            .filter(outputs::locked_by_proof.eq(proof_id as i32))
            .load::<models::ConfidentialOutputModel>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_locked_by_proof", e))?;

        let account_name = rows
            .first()
            .map(|row| {
                accounts::table
                    .filter(accounts::id.eq(row.account_id))
                    .select(accounts::name)
                    .first::<String>(self.connection())
                    .map_err(|e| WalletStorageError::general("outputs_get_locked_by_proof", e))
            })
            .transpose()?;

        let outputs = rows
            .into_iter()
            // TODO: we should add account id to the public interface (an account name is optional)
            .map(|row| row.try_into_output(account_name.clone().unwrap()))
            .collect::<Result<_, _>>()?;
        Ok(outputs)
    }

    fn outputs_get_by_account_and_status(
        &mut self,
        account_name: &str,
        status: OutputStatus,
    ) -> Result<Vec<ConfidentialOutput>, WalletStorageError> {
        use crate::schema::{accounts, outputs};

        let account_id = accounts::table
            .filter(accounts::name.eq(account_name))
            .select(accounts::id)
            .first::<i32>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_by_account_and_status", e))?;

        let rows = outputs::table
            .filter(outputs::account_id.eq(account_id))
            .filter(outputs::status.eq(status.as_key_str()))
            .load::<models::ConfidentialOutputModel>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_by_account_and_status", e))?;

        let outputs = rows
            .into_iter()
            .map(|row| row.try_into_output("".to_string()))
            .collect::<Result<_, _>>()?;
        Ok(outputs)
    }

    fn proofs_get_by_transaction_hash(
        &mut self,
        transaction_hash: FixedHash,
    ) -> Result<ConfidentialProofId, WalletStorageError> {
        use crate::schema::proofs;

        let proof_id = proofs::table
            .filter(proofs::transaction_hash.eq(transaction_hash.to_string()))
            .select(proofs::id)
            .first::<i32>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("proofs_get_by_transaction_hash", e))?;
        let proof_id = proof_id.ok_or_else(|| WalletStorageError::NotFound {
            operation: "proofs_get_by_transaction_hash",
            entity: "proofs".to_string(),
            key: transaction_hash.to_string(),
        })?;

        Ok(proof_id as u64)
    }
}

impl Drop for ReadTransaction<'_> {
    fn drop(&mut self) {
        if !self.is_done {
            if let Err(err) = self.rollback() {
                error!(target: LOG_TARGET, "Failed to rollback transaction: {}", err);
            }
        }
    }
}
