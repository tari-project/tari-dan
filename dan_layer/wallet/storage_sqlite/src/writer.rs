//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    ops::{Deref, DerefMut},
    sync::MutexGuard,
};

use chrono::NaiveDateTime;
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
use tari_common_types::types::{Commitment, FixedHash, PublicKey};
use tari_dan_common_types::QuorumCertificate;
use tari_dan_wallet_sdk::{
    models::{
        ConfidentialOutput,
        ConfidentialProofId,
        OutputStatus,
        SubstateRecord,
        TransactionStatus,
        VersionedSubstateAddress,
    },
    storage::{WalletStorageError, WalletStoreReader, WalletStoreWriter},
};
use tari_engine_types::{commit_result::FinalizeResult, substate::SubstateAddress, TemplateAddress};
use tari_transaction::Transaction;
use tari_utilities::hex::Hex;

use crate::{diesel::ExpressionMethods, models, reader::ReadTransaction, serialization::serialize_json};

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

    fn outputs_lock_smallest_amount(
        &mut self,
        account_name: &str,
        locked_by_proof: ConfidentialProofId,
    ) -> Result<ConfidentialOutput, WalletStorageError> {
        use crate::schema::{accounts, outputs};

        let account_id = accounts::table
            .select(accounts::id)
            .filter(accounts::name.eq(account_name))
            .first::<i32>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_lock_smallest_amount", e))?;

        let locked_output = outputs::table
            .filter(outputs::account_id.eq(account_id))
            .filter(outputs::status.eq(OutputStatus::Unspent.as_key_str()))
            .order_by(outputs::value.asc())
            .first::<models::ConfidentialOutputModel>(self.connection())
            .optional()
            .map_err(|e| WalletStorageError::general("outputs_lock_smallest_amount", e))?;

        let locked_output = locked_output.ok_or_else(|| WalletStorageError::NotFound {
            operation: "outputs_lock_smallest_amount",
            entity: "output".to_string(),
            key: account_name.to_string(),
        })?;

        sql_query("UPDATE outputs SET `status` = ?, locked_by_proof = ?, locked_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind::<Text, _>(OutputStatus::Locked.as_key_str())
            .bind::<Integer, _>(locked_by_proof as i32)
            .bind::<Integer, _>(locked_output.id)
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_lock_smallest_amount", e))?;

        Ok(ConfidentialOutput {
            account_name: account_name.to_string(),
            commitment: Commitment::from_hex(&locked_output.commitment).map_err(|_| {
                WalletStorageError::DecodingError {
                    operation: "outputs_lock_smallest_amount",
                    item: "output commitment",
                    details: "Corrupt db: invalid hex representation".to_string(),
                }
            })?,
            value: locked_output.value as u64,
            sender_public_nonce: locked_output
                .sender_public_nonce
                .map(|nonce| PublicKey::from_hex(&nonce).unwrap()),
            secret_key_index: locked_output.secret_key_index as u64,
            public_asset_tag: None,
            status: OutputStatus::Locked,
            locked_by_proof: Some(locked_by_proof),
        })
    }

    fn outputs_insert(&mut self, output: ConfidentialOutput) -> Result<(), WalletStorageError> {
        use crate::schema::{accounts, outputs};

        let account_id = accounts::table
            .select(accounts::id)
            .filter(accounts::name.eq(&output.account_name))
            .first::<i32>(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_insert", e))?;

        diesel::insert_into(outputs::table)
            .values((
                outputs::account_id.eq(account_id),
                outputs::commitment.eq(output.commitment.to_hex()),
                outputs::value.eq(output.value as i64),
                outputs::sender_public_nonce.eq(output.sender_public_nonce.map(|pk| pk.to_hex())),
                outputs::secret_key_index.eq(output.secret_key_index as i64),
                outputs::status.eq(output.status.as_key_str()),
                outputs::locked_by_proof.eq(output.locked_by_proof.map(|v| v as i32)),
            ))
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_insert", e))?;

        Ok(())
    }

    fn outputs_finalize_by_proof_id(&mut self, proof_id: ConfidentialProofId) -> Result<(), WalletStorageError> {
        use crate::schema::outputs;

        // Unlock locked unconfirmed outputs
        diesel::update(outputs::table)
            .filter(outputs::locked_by_proof.eq(proof_id as i32))
            .filter(outputs::status.eq(OutputStatus::LockedUnconfirmed.as_key_str()))
            .set((
                outputs::status.eq(OutputStatus::Unspent.as_key_str()),
                outputs::locked_by_proof.eq::<Option<i32>>(None),
                outputs::locked_at.eq::<Option<NaiveDateTime>>(None),
            ))
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_finalize_by_proof_id", e))?;

        // Mark locked outputs as spent
        diesel::update(outputs::table)
            .filter(outputs::locked_by_proof.eq(proof_id as i32))
            .filter(outputs::status.eq(OutputStatus::Locked.as_key_str()))
            .set((
                outputs::status.eq(OutputStatus::Spent.as_key_str()),
                outputs::locked_by_proof.eq::<Option<i32>>(None),
                outputs::locked_at.eq::<Option<NaiveDateTime>>(None),
            ))
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_finalize_by_proof_id", e))?;

        Ok(())
    }

    fn outputs_release_by_proof_id(&mut self, proof_id: ConfidentialProofId) -> Result<(), WalletStorageError> {
        use crate::schema::outputs;

        // Unlock locked unspent outputs
        diesel::update(outputs::table)
            .filter(outputs::locked_by_proof.eq(proof_id as i32))
            .filter(outputs::status.eq(OutputStatus::Locked.as_key_str()))
            .set((
                outputs::status.eq(OutputStatus::Unspent.as_key_str()),
                outputs::locked_by_proof.eq::<Option<i32>>(None),
                outputs::locked_at.eq::<Option<NaiveDateTime>>(None),
            ))
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_unlock_by_proof_id", e))?;

        // Remove outputs that were created by this proof
        diesel::delete(outputs::table)
            .filter(outputs::status.eq(OutputStatus::LockedUnconfirmed.as_key_str()))
            .filter(outputs::locked_by_proof.eq(proof_id as i32))
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("outputs_unlock_by_proof_id", e))?;

        Ok(())
    }

    // Proofs
    fn proofs_insert(&mut self, account_name: String) -> Result<ConfidentialProofId, WalletStorageError> {
        use crate::schema::{accounts, proofs};

        let account_id = accounts::table
            .select(accounts::id)
            .filter(accounts::name.eq(&account_name))
            .first::<i32>(self.connection())
            .map_err(|e| WalletStorageError::general("proof_insert", e))?;

        diesel::insert_into(proofs::table)
            .values(proofs::account_id.eq(account_id))
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("proof_insert", e))?;

        // RETURNING only available from SQLite 3.35 https://www.sqlite.org/lang_returning.html
        // TODO: See if we can upgrade SQLite
        let proof_id = proofs::table
            .select(proofs::id)
            .order_by(proofs::id.desc())
            .first::<i32>(self.connection())
            .map_err(|e| WalletStorageError::general("proof_insert", e))?;

        Ok(proof_id as ConfidentialProofId)
    }

    fn proofs_delete(&mut self, proof_id: ConfidentialProofId) -> Result<(), WalletStorageError> {
        use crate::schema::proofs;

        diesel::delete(proofs::table.filter(proofs::id.eq(proof_id as i32)))
            .execute(self.connection())
            .map_err(|e| WalletStorageError::general("proof_delete", e))?;

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
