//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_engine_types::{
    indexed_value::{IndexedValueError, IndexedWellKnownTypes},
    substate::SubstateDiff,
};
use tari_template_lib::prelude::ComponentAddress;
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};

use crate::{
    models::{TransactionStatus, VersionedSubstateAddress, WalletTransaction},
    network::{TransactionFinalizedResult, TransactionQueryResult, WalletNetworkInterface},
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

    pub async fn submit_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionId, TransactionApiError> {
        self.store
            .with_write_tx(|tx| tx.transactions_insert(&transaction, false))?;

        let transaction_id = *transaction.id();
        self.network_interface
            .submit_transaction(transaction, required_substates)
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

        Ok(transaction_id)
    }

    pub async fn submit_dry_run_transaction(
        &self,
        transaction: Transaction,
        required_substates: Vec<SubstateRequirement>,
    ) -> Result<TransactionQueryResult, TransactionApiError> {
        self.store
            .with_write_tx(|tx| tx.transactions_insert(&transaction, true))?;

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
                            .and_then(|e| e.fee_receipt.as_ref())
                            .map(|f| f.total_fees_charged()),
                        None,
                        TransactionStatus::DryRun,
                        Some(*execution_time),
                        Some(*finalized_time),
                    )
                })?;
            },
        }

        Ok(query)
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
                json_results,
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

                self.store.with_write_tx(|tx| {
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
                            .and_then(|e| e.fee_receipt.as_ref())
                            .map(|f| f.total_fees_charged()),
                        // TODO: readd qcs
                        None,
                        // Some(&qc_resp.qcs),
                        new_status,
                        Some(execution_time),
                        Some(finalized_time),
                    )?;
                    if !transaction.is_dry_run {
                        // if the transaction being processed is confidential,
                        // we should make sure that the account's locked outputs
                        // are either set to spent or released, depending if the
                        // transaction was finalized or rejected
                        if let Some(proof_id) = tx.proofs_get_by_transaction_id(transaction_id).optional()? {
                            if new_status == TransactionStatus::Accepted {
                                tx.outputs_finalize_by_proof_id(proof_id)?;
                            } else {
                                tx.outputs_release_by_proof_id(proof_id)?;
                            }
                        }
                    }

                    Ok::<_, TransactionApiError>(())
                })?;
                Ok(Some(WalletTransaction {
                    transaction: transaction.transaction,
                    status: new_status,
                    finalize: execution_result.as_ref().map(|e| e.finalize.clone()),
                    final_fee: execution_result
                        .as_ref()
                        .and_then(|e| e.fee_receipt.as_ref())
                        .map(|f| f.total_fees_charged()),
                    // TODO: re-add QCs
                    // qcs: qc_resp.qcs,
                    qcs: vec![],
                    is_dry_run: transaction.is_dry_run,
                    execution_time: Some(execution_time),
                    finalized_time: Some(finalized_time),
                    json_result: Some(json_results),
                    // This is not precise, we should read it back from DB, but it's not critical
                    last_update_time: chrono::Utc::now().naive_utc(),
                }))
            },
        }
    }

    fn commit_result(
        &self,
        tx: &mut TStore::WriteTransaction<'_>,
        transaction_id: TransactionId,
        diff: &SubstateDiff,
    ) -> Result<(), TransactionApiError> {
        for (addr, _) in diff.down_iter() {
            if addr.is_layer1_commitment() {
                info!(target: LOG_TARGET, "Layer 1 commitment {} downed", addr);
                continue;
            }

            if tx.substates_remove(addr).optional()?.is_none() {
                warn!(target: LOG_TARGET, "Downed substate {} not found", addr);
            }
        }

        let (components, mut rest) = diff.up_iter().partition::<Vec<_>, _>(|(addr, _)| addr.is_component());

        for (component_addr, substate) in components {
            let header = substate.substate_value().component().unwrap();

            tx.substates_insert_root(
                transaction_id,
                VersionedSubstateAddress {
                    address: component_addr.clone(),
                    version: substate.version(),
                },
                Some(header.module_name.clone()),
                Some(header.template_address),
            )?;

            let value = IndexedWellKnownTypes::from_value(header.state())?;

            for owned_addr in value.referenced_substates() {
                if let Some(pos) = rest.iter().position(|(addr, _)| addr == &owned_addr) {
                    let (_, s) = rest.swap_remove(pos);
                    tx.substates_insert_child(transaction_id, component_addr.clone(), VersionedSubstateAddress {
                        address: owned_addr,
                        version: s.version(),
                    })?;
                }
            }
        }

        for (addr, substate) in rest {
            tx.substates_insert_root(
                transaction_id,
                VersionedSubstateAddress {
                    address: addr.clone(),
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
