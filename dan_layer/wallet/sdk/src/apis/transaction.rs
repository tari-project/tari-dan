//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{
    optional::{IsNotFoundError, Optional},
    PayloadId,
};
use tari_engine_types::{
    indexed_value::{IndexedValue, ValueVisitorError},
    substate::SubstateDiff,
};
use tari_transaction::Transaction;

use crate::{
    models::{TransactionStatus, VersionedSubstateAddress, WalletTransaction},
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
    substate_provider::WalletNetworkInterface,
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

    pub fn get(&self, hash: FixedHash) -> Result<WalletTransaction, TransactionApiError> {
        let mut tx = self.store.create_read_tx()?;
        let transaction = tx.transaction_get(hash)?;
        Ok(transaction)
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<FixedHash, TransactionApiError> {
        self.submit_transaction_internal(transaction, false).await
    }

    pub async fn submit_dry_run_transaction(&self, transaction: Transaction) -> Result<FixedHash, TransactionApiError> {
        self.submit_transaction_internal(transaction, true).await
    }

    async fn submit_transaction_internal(
        &self,
        transaction: Transaction,
        is_dry_run: bool,
    ) -> Result<FixedHash, TransactionApiError> {
        self.store
            .with_write_tx(|tx| tx.transactions_insert(&transaction, is_dry_run))?;

        let hash = self
            .network_interface
            .submit_transaction(transaction, is_dry_run)
            .await
            .map_err(|e| TransactionApiError::NetworkInterfaceError(e.to_string()))?;

        self.store.with_write_tx(|tx| {
            tx.transactions_set_result_and_status(
                hash,
                None,
                None,
                None,
                None,
                if is_dry_run {
                    TransactionStatus::DryRun
                } else {
                    TransactionStatus::Pending
                },
            )
        })?;

        Ok(hash)
    }

    pub fn fetch_all_by_status(
        &self,
        status: TransactionStatus,
    ) -> Result<Vec<WalletTransaction>, TransactionApiError> {
        let mut tx = self.store.create_read_tx()?;
        let transactions = tx.transactions_fetch_all_by_status(status)?;
        Ok(transactions)
    }

    pub async fn check_and_store_finalized_transaction(
        &self,
        hash: FixedHash,
    ) -> Result<Option<WalletTransaction>, TransactionApiError> {
        // Multithreaded considerations: The transaction result could be requested more than once because db
        // transactions cannot be used around await points.
        let transaction = self.store.with_read_tx(|tx| tx.transaction_get(hash))?;
        if transaction.finalize.is_some() {
            return Ok(Some(transaction));
        }

        let maybe_resp = self
            .network_interface
            .query_transaction_result(PayloadId::new(hash))
            .await
            .optional()
            .map_err(|e| TransactionApiError::NetworkInterfaceError(e.to_string()))?;

        let Some(resp) = maybe_resp else {
            warn!( target: LOG_TARGET, "Transaction result not found for transaction with hash {}. Marking transaction as invalid", hash);
            self.store.with_write_tx(|tx| {
                tx.transactions_set_result_and_status(
                    hash,
                    None,
                    None,
                    None,
                    None,
                    TransactionStatus::InvalidTransaction,
                )
            })?;

            // Not found - TODO: this probably means the transaction was rejected in the mempool, but we cant be sure. Perhaps we should store it in its entirety and allow the user to resubmit it.
            return Ok(Some(WalletTransaction {
                transaction: transaction.transaction,
                status: TransactionStatus::InvalidTransaction,
                finalize: None,
                transaction_failure: None,
                final_fee: None,
                qcs: vec![],
                is_dry_run: transaction.is_dry_run,
            }));
        };

        match resp.execution_result {
            Some(result) => {
                let new_status = if result.finalize.result.is_accept() && result.transaction_failure.is_none() {
                    TransactionStatus::Accepted
                } else {
                    TransactionStatus::Rejected
                };

                // let qc_resp = self.network_interface
                //     .fetch_transaction_quorum_certificates(GetTransactionQcsRequest { hash })
                //     .await
                //     .map_err(TransactionApiError::ValidatorNodeClientError)?;

                self.store.with_write_tx(|tx| {
                    if !transaction.is_dry_run {
                        if let Some(diff) = result.finalize.result.accept() {
                            self.commit_result(tx, hash, diff)?;
                        }
                    }

                    tx.transactions_set_result_and_status(
                        hash,
                        Some(&result.finalize),
                        result.transaction_failure.as_ref(),
                        result.fee_receipt.as_ref().map(|f| f.total_fees_charged()),
                        // TODO: readd qcs
                        None,
                        // Some(&qc_resp.qcs),
                        new_status,
                    )?;
                    if !transaction.is_dry_run {
                        // if the transaction being processed is confidential,
                        // we should make sure that the account's locked outputs
                        // are either set to spent or released, depending if the
                        // transaction was finalized or rejected
                        if let Some(proof_id) = tx.proofs_get_by_transaction_hash(hash).optional()? {
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
                    finalize: Some(result.finalize),
                    transaction_failure: result.transaction_failure,
                    final_fee: result.fee_receipt.as_ref().map(|f| f.total_fees_charged()),
                    // TODO: re-add QCs
                    // qcs: qc_resp.qcs,
                    qcs: vec![],
                    is_dry_run: transaction.is_dry_run,
                }))
            },
            None => Ok(None),
        }
    }

    fn commit_result(
        &self,
        tx: &mut TStore::WriteTransaction<'_>,
        tx_hash: FixedHash,
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
                tx_hash,
                VersionedSubstateAddress {
                    address: component_addr.clone(),
                    version: substate.version(),
                },
                Some(header.module_name.clone()),
                Some(header.template_address),
            )?;

            let value = IndexedValue::from_raw(&header.state.state)?;

            for owned_addr in value.owned_substates() {
                if let Some(pos) = rest.iter().position(|(addr, _)| addr == &owned_addr) {
                    let (_, s) = rest.swap_remove(pos);
                    tx.substates_insert_child(
                        tx_hash,
                        component_addr.clone(),
                        VersionedSubstateAddress {
                            address: owned_addr,
                            version: s.version(),
                        },
                    )?;
                }
            }
        }

        for (addr, substate) in rest {
            tx.substates_insert_root(
                tx_hash,
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
    ValueVisitorError(#[from] ValueVisitorError),
}

impl IsNotFoundError for TransactionApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
