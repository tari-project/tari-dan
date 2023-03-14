//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, str::FromStr, sync::MutexGuard};

use bigdecimal::{BigDecimal, ToPrimitive};
use diesel::{dsl::sum, sql_query, OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection};
use log::error;
use serde::de::DeserializeOwned;
use tari_common_types::types::{Commitment, FixedHash};
use tari_dan_wallet_sdk::{
    models::{
        Account,
        ConfidentialOutputModel,
        ConfidentialProofId,
        Config,
        OutputStatus,
        SubstateModel,
        TransactionStatus,
        VaultModel,
        WalletTransaction,
    },
    storage::{WalletStorageError, WalletStoreReader},
};
use tari_engine_types::substate::{InvalidSubstateAddressFormat, SubstateAddress};
use tari_template_lib::models::ResourceAddress;
use tari_utilities::hex::Hex;

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
    // -------------------------------- KeyManager -------------------------------- //

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

    // -------------------------------- Config -------------------------------- //
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

    // -------------------------------- Transactions -------------------------------- //
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

    // -------------------------------- Substates -------------------------------- //
    fn substates_get(&mut self, address: &SubstateAddress) -> Result<SubstateModel, WalletStorageError> {
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

    fn substates_get_children(&mut self, parent: &SubstateAddress) -> Result<Vec<SubstateModel>, WalletStorageError> {
        use crate::schema::substates;

        let rows = substates::table
            .filter(substates::parent_address.eq(parent.to_string()))
            .get_results::<models::Substate>(self.connection())
            .map_err(|e| WalletStorageError::general("transactions_fetch_all_by_status", e))?;

        rows.into_iter().map(|rec| rec.try_to_record()).collect()
    }

    // -------------------------------- Accounts -------------------------------- //
    fn accounts_get(&mut self, address: &SubstateAddress) -> Result<Account, WalletStorageError> {
        use crate::schema::accounts;

        let row = accounts::table
            .filter(accounts::address.eq(address.to_string()))
            .first::<models::Account>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("accounts_get", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "accounts_get",
                entity: "account".to_string(),
                key: address.to_string(),
            })?;

        let account = row.try_into().map_err(|e| WalletStorageError::DecodingError {
            operation: "accounts_get",
            item: "account",
            details: format!("Failed to convert SQL record to Account: {}", e),
        })?;
        Ok(account)
    }

    fn accounts_get_many(&mut self, offset: u64, limit: u64) -> Result<Vec<Account>, WalletStorageError> {
        use crate::schema::accounts;

        let rows = accounts::table
            .limit(limit as i64)
            .offset(offset as i64)
            .load::<models::Account>(self.connection())
            .map_err(|e| WalletStorageError::general("accounts_get_many", e))?;

        let accs = rows
            .into_iter()
            .map(|row| {
                row.try_into().map_err(|e| WalletStorageError::DecodingError {
                    operation: "accounts_get_many",
                    item: "account",
                    details: format!("Failed to convert SQL record to Account: {}", e),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(accs)
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

    fn accounts_get_by_vault(&mut self, vault_address: &SubstateAddress) -> Result<Account, WalletStorageError> {
        use crate::schema::{accounts, vaults};

        let account_id = vaults::table
            .select(vaults::account_id)
            .filter(vaults::address.eq(vault_address.to_string()))
            .first::<i32>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("accounts_get_by_vault", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "accounts_get_by_vault",
                entity: "vault".to_string(),
                key: vault_address.to_string(),
            })?;

        let row = accounts::table
            .filter(accounts::id.eq(account_id))
            .first::<models::Account>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("accounts_get_by_vault", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "accounts_get_by_vault",
                entity: "account".to_string(),
                key: vault_address.to_string(),
            })?;

        let account = row
            .try_into()
            .map_err(|e: InvalidSubstateAddressFormat| WalletStorageError::DecodingError {
                operation: "accounts_get_by_vault",
                item: "account",
                details: e.to_string(),
            })?;
        Ok(account)
    }

    // -------------------------------- Vaults -------------------------------- //
    fn vaults_get(&mut self, address: &SubstateAddress) -> Result<VaultModel, WalletStorageError> {
        use crate::schema::{accounts, vaults};

        let row = vaults::table
            .filter(vaults::address.eq(address.to_string()))
            .first::<models::Vault>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("vaults_get", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "vaults_get",
                entity: "vault".to_string(),
                key: address.to_string(),
            })?;

        let account_address = accounts::table
            .select(accounts::address)
            .filter(accounts::id.eq(row.account_id))
            .first::<String>(self.connection())
            .map_err(|e| WalletStorageError::general("vaults_get", e))?;

        let vault = row.try_into_vault(SubstateAddress::from_str(&account_address).map_err(|e| {
            WalletStorageError::DecodingError {
                operation: "vaults_get",
                item: "vault",
                details: e.to_string(),
            }
        })?)?;
        Ok(vault)
    }

    fn vaults_get_by_resource(
        &mut self,
        account_addr: &SubstateAddress,
        resource_address: &ResourceAddress,
    ) -> Result<VaultModel, WalletStorageError> {
        use crate::schema::{accounts, vaults};

        let account_id = accounts::table
            .filter(accounts::address.eq(account_addr.to_string()))
            .select(accounts::id)
            .first::<i32>(self.connection())
            .map_err(|e| WalletStorageError::general("vaults_get_by_resource", e))?;

        let row = vaults::table
            .filter(vaults::account_id.eq(account_id))
            .filter(vaults::resource_address.eq(resource_address.to_string()))
            .first::<models::Vault>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("vaults_get_by_resource", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "vaults_get_by_resource",
                entity: "vault".to_string(),
                key: resource_address.to_string(),
            })?;

        let vault = row
            .try_into_vault(account_addr.clone())
            .map_err(|e| WalletStorageError::DecodingError {
                operation: "vaults_get_by_resource",
                item: "vault",
                details: format!("Failed to convert record to Vault: {}", e),
            })?;
        Ok(vault)
    }

    fn vaults_get_by_account(&mut self, account_addr: &SubstateAddress) -> Result<Vec<VaultModel>, WalletStorageError> {
        use crate::schema::{accounts, vaults};

        let account_id = accounts::table
            .filter(accounts::address.eq(account_addr.to_string()))
            .select(accounts::id)
            .first::<i32>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("vaults_get_by_account", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "vaults_get_by_account",
                entity: "account".to_string(),
                key: account_addr.to_string(),
            })?;

        let rows = vaults::table
            .filter(vaults::account_id.eq(account_id))
            .load::<models::Vault>(self.connection())
            .map_err(|e| WalletStorageError::general("vaults_get_by_account", e))?;

        let vaults = rows
            .into_iter()
            .map(|row| row.try_into_vault(account_addr.clone()))
            .collect::<Result<_, _>>()?;

        Ok(vaults)
    }

    // -------------------------------- Outputs -------------------------------- //
    fn outputs_get_unspent_balance(&mut self, account_address: &SubstateAddress) -> Result<u64, WalletStorageError> {
        use crate::schema::{accounts, outputs};

        let account_id = accounts::table
            .filter(accounts::address.eq(account_address.to_string()))
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
    ) -> Result<Vec<ConfidentialOutputModel>, WalletStorageError> {
        use crate::schema::{accounts, outputs, vaults};

        let rows = outputs::table
            .filter(outputs::locked_by_proof.eq(proof_id as i32))
            .get_results::<models::ConfidentialOutput>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_locked_by_proof", e))?;

        if rows.is_empty() {
            return Ok(vec![]);
        }

        let vault_addresses = if rows.is_empty() {
            HashMap::new()
        } else {
            let vec = vaults::table
                .filter(vaults::id.eq_any(rows.iter().map(|v| v.vault_id)))
                .select((vaults::id, vaults::address))
                .get_results::<(i32, String)>(self.connection())
                .map_err(|e| WalletStorageError::general("outputs_get_locked_by_proof", e))?;
            vec.into_iter().collect()
        };

        // account_id should be the same in all rows
        let account_address = rows
            .first()
            .map(|row| {
                accounts::table
                    .filter(accounts::id.eq(row.account_id))
                    .select(accounts::address)
                    .first::<String>(self.connection())
                    .map_err(|e| WalletStorageError::general("outputs_get_locked_by_proof", e))
            })
            .transpose()?;

        let outputs = rows
            .into_iter()
            .map(|row| {
                let vault_id = row.vault_id;
                row.try_into_output(
                    account_address.as_ref().unwrap(),
                    vault_addresses.get(&vault_id).unwrap(),
                )
            })
            .collect::<Result<_, _>>()?;
        Ok(outputs)
    }

    fn outputs_get_by_commitment(
        &mut self,
        commitment: &Commitment,
    ) -> Result<ConfidentialOutputModel, WalletStorageError> {
        use crate::schema::{accounts, outputs, vaults};

        let row = outputs::table
            .filter(outputs::commitment.eq(commitment.to_hex()))
            .first::<models::ConfidentialOutput>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("outputs_get_by_commitment", e))?
            .ok_or_else(|| WalletStorageError::NotFound {
                operation: "outputs_get_by_commitment",
                entity: "output".to_string(),
                key: commitment.to_hex(),
            })?;

        let account_addr = accounts::table
            .filter(accounts::id.eq(row.account_id))
            .select(accounts::address)
            .first::<String>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_by_commitment", e))?;

        let vaults_addr = vaults::table
            .filter(vaults::id.eq(row.vault_id))
            .select(vaults::address)
            .first::<String>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_by_commitment", e))?;

        let output = row.try_into_output(&account_addr, &vaults_addr)?;
        Ok(output)
    }

    fn outputs_get_by_account_and_status(
        &mut self,
        account_addr: &SubstateAddress,
        status: OutputStatus,
    ) -> Result<Vec<ConfidentialOutputModel>, WalletStorageError> {
        use crate::schema::{accounts, outputs, vaults};

        let account_id = accounts::table
            .filter(accounts::address.eq(account_addr.to_string()))
            .select(accounts::id)
            .first::<i32>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_by_account_and_status", e))?;

        let rows = outputs::table
            .filter(outputs::account_id.eq(account_id))
            .filter(outputs::status.eq(status.as_key_str()))
            .load::<models::ConfidentialOutput>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_get_by_account_and_status", e))?;

        let vault_addresses = if rows.is_empty() {
            HashMap::new()
        } else {
            let vec = vaults::table
                .filter(vaults::id.eq_any(rows.iter().map(|v| v.vault_id)))
                .select((vaults::id, vaults::address))
                .get_results::<(i32, String)>(self.connection())
                .map_err(|e| WalletStorageError::general("outputs_get_locked_by_proof", e))?;
            vec.into_iter().collect()
        };

        let outputs = rows
            .into_iter()
            .map(|row| {
                let vault_id = row.vault_id;
                row.try_into_output(&account_addr.to_string(), vault_addresses.get(&vault_id).unwrap())
            })
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
