//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use log::*;
use tari_dan_common_types::{
    optional::{IsNotFoundError, Optional},
    SubstateRequirement,
};
use tari_engine_types::{
    indexed_value::{IndexedValueError, IndexedWellKnownTypes},
    substate::SubstateDiff,
};
use tari_template_lib::prelude::ComponentAddress;
use tari_transaction::{Transaction, TransactionId};

use crate::{
    models::{NewAccountInfo, TransactionStatus, VersionedSubstateId, WalletTransaction},
    network::{TransactionFinalizedResult, WalletNetworkInterface},
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

const LOG_TARGET: &str = "tari::dan::wallet_sdk::apis::transaction";

pub struct TransactionApi<'a, TStore, TNetworkInterface> {
    store: &'a TStore,
    network_interface: &'a TNetworkInterface,
}

impl<'a, TStore, TNetworkInterface> TransactionApi<'a, TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
    TNetworkInterface::Error: IsNotFoundError,
{
    pub fn new(store: &'a TStore, network_interface: &'a TNetworkInterface) -> Self {
        Self {
            store,
            network_interface,
        }
    }

    pub fn get(&self, tx_id: TransactionId) -> Result<WalletTransaction, TransactionApiError> {
        let mut tx = self.store.create_read_tx()?;
        let transaction = tx.transactions_get(tx_id)?;
        Ok(transaction)
    }

    pub async fn insert_new_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
        new_account_info: Option<NewAccountInfo>,
        is_dry_run: bool,
    ) -> Result<TransactionId, TransactionApiError> {
        let tx_id = *transaction.id();
        self.store.with_write_tx(|tx| {
            tx.transactions_insert(&transaction, &required_substates, new_account_info.as_ref(), is_dry_run)
        })?;

        Ok(tx_id)
    }

    pub async fn submit_transaction(&self, transaction_id: TransactionId) -> Result<(), TransactionApiError> {
        let transaction = self.store.with_read_tx(|tx| tx.transactions_get(transaction_id))?;

        if !matches!(transaction.status, TransactionStatus::New) {
            return Err(TransactionApiError::StoreError(WalletStorageError::OperationError {
                operation: "submit_transaction",
                details: format!("Transaction {} is not in New status", transaction_id),
            }));
        }

        self.network_interface
            .submit_transaction(transaction.transaction, transaction.required_substates)
            .await
            .map_err(|e| TransactionApiError::NetworkInterfaceError(e.to_string()))?;

        self.store.with_write_tx(|tx| {
            tx.transactions_set_result_and_status(
                transaction_id,
                None,
                None,
                None,
                TransactionStatus::Pending,
                None,
                None,
            )
        })?;

        Ok(())
    }

    pub async fn submit_dry_run_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<WalletTransaction, TransactionApiError> {
        self.store
            .with_write_tx(|tx| tx.transactions_insert(&transaction, &required_substates, None, true))?;

        let tx_id = *transaction.id();
        let query = self
            .network_interface
            .submit_dry_run_transaction(transaction, required_substates)
            .await
            .map_err(|e| TransactionApiError::NetworkInterfaceError(e.to_string()))?;

        match &query.result {
            TransactionFinalizedResult::Pending => {
                return Err(TransactionApiError::NetworkInterfaceError(
                    "Pending execution result returned from dry run".to_string(),
                ));
            },
            TransactionFinalizedResult::Finalized {
                execution_result,
                finalized_time,
                execution_time,
                ..
            } => {
                self.store.with_write_tx(|tx| {
                    tx.transactions_set_result_and_status(
                        query.transaction_id,
                        execution_result.as_ref().map(|e| &e.finalize),
                        execution_result
                            .as_ref()
                            .map(|e| e.finalize.fee_receipt.total_fees_charged()),
                        None,
                        TransactionStatus::DryRun,
                        Some(*execution_time),
                        Some(*finalized_time),
                    )
                })?;
            },
        }

        let transaction = self.store.with_read_tx(|tx| tx.transactions_get(tx_id))?;

        Ok(transaction)
    }

    pub fn fetch_all(
        &self,
        status: Option<TransactionStatus>,
        component: Option<ComponentAddress>,
    ) -> Result<Vec<WalletTransaction>, TransactionApiError> {
        let mut tx = self.store.create_read_tx()?;
        let transactions = tx.transactions_fetch_all(status, component)?;
        Ok(transactions)
    }

    pub async fn check_and_store_finalized_transaction(
        &self,
        transaction_id: TransactionId,
    ) -> Result<Option<WalletTransaction>, TransactionApiError> {
        // Multithreaded considerations: The transaction result could be requested more than once because db
        // transactions cannot be used around await points.
        let transaction = self.store.with_read_tx(|tx| tx.transactions_get(transaction_id))?;
        if transaction.finalize.is_some() {
            return Ok(Some(transaction));
        }

        let maybe_resp = self
            .network_interface
            .query_transaction_result(transaction_id)
            .await
            .optional()
            .map_err(|e| TransactionApiError::NetworkInterfaceError(e.to_string()))?;

        let Some(resp) = maybe_resp else {
            // TODO: if this happens forever we might want to resubmit or mark as invalid
            warn!( target: LOG_TARGET, "Transaction result not found for transaction with hash {}. Will check again later.", transaction_id);
            return Ok(None);
        };

        match resp.result {
            TransactionFinalizedResult::Pending => Ok(None),
            TransactionFinalizedResult::Finalized {
                final_decision,
                execution_result,
                execution_time,
                finalized_time,
                abort_details: _,
                ..
            } => {
                let new_status = if final_decision.is_commit() {
                    match execution_result.as_ref() {
                        Some(execution_result) => {
                            if execution_result.finalize.is_fee_only() {
                                TransactionStatus::OnlyFeeAccepted
                            } else {
                                TransactionStatus::Accepted
                            }
                        },
                        None => TransactionStatus::Accepted,
                    }
                } else {
                    TransactionStatus::Rejected
                };

                // let qc_resp = self.network_interface
                //     .fetch_transaction_quorum_certificates(GetTransactionQcsRequest { hash })
                //     .await
                //     .map_err(TransactionApiError::ValidatorNodeClientError)?;

                let transaction = self.store.with_write_tx(|tx| {
                    if !transaction.is_dry_run && final_decision.is_commit() {
                        let diff = execution_result
                            .as_ref()
                            .and_then(|e| e.finalize.result.accept())
                            .ok_or_else(|| TransactionApiError::InvalidTransactionQueryResponse {
                                details: format!(
                                    "NEVERHAPPEN: Finalize decision is COMMIT but transaction failed: {:?}",
                                    execution_result.as_ref().and_then(|e| e.finalize.result.reject())
                                ),
                            })?;

                        self.commit_result(tx, transaction_id, diff)?;
                    }

                    tx.transactions_set_result_and_status(
                        transaction_id,
                        execution_result.as_ref().map(|e| &e.finalize),
                        execution_result
                            .as_ref()
                            .map(|e| e.finalize.fee_receipt.total_fees_charged()),
                        // TODO: readd qcs
                        None,
                        // Some(&qc_resp.qcs),
                        new_status,
                        Some(execution_time),
                        Some(finalized_time),
                    )?;

                    // if the transaction being processed is confidential,
                    // we should make sure that the account's locked outputs
                    // are either set to spent or released, depending if the
                    // transaction was finalized or rejected. Always release for dry runs.
                    if transaction.is_dry_run || new_status != TransactionStatus::Accepted {
                        self.release_all_outputs_for_transaction_internal(tx, transaction_id)?;
                    } else {
                        let proof_ids = tx.proofs_get_by_transaction_id(transaction_id)?;
                        for proof_id in proof_ids {
                            tx.outputs_finalize_by_proof_id(proof_id)?;
                            tx.vaults_finalized_locked_revealed_funds(proof_id)?;
                        }
                    }

                    let transaction = tx.transactions_get(transaction_id)?;
                    Ok::<_, TransactionApiError>(transaction)
                })?;

                Ok(Some(transaction))
            },
        }
    }

    pub fn release_all_outputs_for_transaction(
        &self,
        transaction_id: TransactionId,
    ) -> Result<(), TransactionApiError> {
        self.store
            .with_write_tx(|tx| self.release_all_outputs_for_transaction_internal(tx, transaction_id))
    }

    fn release_all_outputs_for_transaction_internal(
        &self,
        tx: &mut <TStore as WalletStore>::WriteTransaction<'_>,
        transaction_id: TransactionId,
    ) -> Result<(), TransactionApiError> {
        let proof_ids = tx.proofs_get_by_transaction_id(transaction_id)?;

        debug!(target: LOG_TARGET, "Releasing {} proofs (and associated outputs) for transaction {} that was not committed", proof_ids.len(), transaction_id);
        for proof_id in proof_ids {
            tx.outputs_release_by_proof_id(proof_id)?;
        }

        Ok(())
    }

    fn commit_result(
        &self,
        tx: &mut TStore::WriteTransaction<'_>,
        transaction_id: TransactionId,
        diff: &SubstateDiff,
    ) -> Result<(), TransactionApiError> {
        let mut downed_substates_with_parents = HashMap::with_capacity(diff.down_len());
        for (id, _) in diff.down_iter() {
            if id.is_layer1_commitment() {
                info!(target: LOG_TARGET, "Layer 1 commitment {} downed", id);
                continue;
            }

            let Some(downed) = tx.substates_remove(id).optional()? else {
                warn!(target: LOG_TARGET, "Downed substate {} not found", id);
                continue;
            };

            if let Some(parent) = downed.parent_address {
                downed_substates_with_parents.insert(downed.address.substate_id, parent);
            }
        }

        let (components, mut rest) = diff.up_iter().partition::<Vec<_>, _>(|(addr, _)| addr.is_component());

        for (component_addr, substate) in components {
            let header = substate.substate_value().component().unwrap();

            debug!(target: LOG_TARGET, "Substate {} up", component_addr);
            tx.substates_upsert_root(
                transaction_id,
                VersionedSubstateId {
                    substate_id: component_addr.clone(),
                    version: substate.version(),
                },
                Some(header.module_name.clone()),
                Some(header.template_address),
            )?;

            let value = IndexedWellKnownTypes::from_value(header.state())?;

            for owned_addr in value.referenced_substates() {
                if let Some(pos) = rest.iter().position(|(addr, _)| addr == &owned_addr) {
                    let (_, s) = rest.swap_remove(pos);
                    // If there was a previous parent for this substate, we keep it as is.
                    let parent = downed_substates_with_parents
                        .get(&owned_addr)
                        .cloned()
                        .unwrap_or_else(|| component_addr.clone());
                    tx.substates_upsert_child(transaction_id, parent, VersionedSubstateId {
                        substate_id: owned_addr,
                        version: s.version(),
                    })?;
                }
            }
        }

        for (id, substate) in rest {
            if id.is_vault() {
                if let Some(vault) = tx.vaults_get(id).optional()? {
                    // The vault for an account may have been mutated without mutating the account component
                    // If we know this vault, set it as a child of the account
                    tx.substates_upsert_child(transaction_id, vault.account_address, VersionedSubstateId {
                        substate_id: id.clone(),
                        version: substate.version(),
                    })?;
                    continue;
                }
            }
            tx.substates_upsert_root(
                transaction_id,
                VersionedSubstateId {
                    substate_id: id.clone(),
                    version: substate.version(),
                },
                None,
                None,
            )?;
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Network interface error: {0}")]
    NetworkInterfaceError(String),
    #[error("Failed to extract known type data from value: {0}")]
    IndexedValueError(#[from] IndexedValueError),
    #[error("Invalid transaction query response: {details}")]
    InvalidTransactionQueryResponse { details: String },
}

impl IsNotFoundError for TransactionApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
