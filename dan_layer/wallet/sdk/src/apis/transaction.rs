//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use log::*;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_engine_types::{
    substate::{SubstateAddress, SubstateDiff},
    TemplateAddress,
};
use tari_transaction::Transaction;
use tari_validator_node_client::{
    types::{GetTransactionQcsRequest, GetTransactionResultRequest, SubmitTransactionRequest},
    ValidatorNodeClient, ValidatorNodeClientError,
};

use crate::{
    models::{TransactionStatus, VersionedSubstateAddress, WalletTransaction},
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter},
};

const LOG_TARGET: &str = "tari::dan::wallet_sdk::apis::transaction";

pub struct TransactionApi<'a, TStore> {
    store: &'a TStore,
    validator_node_jrpc_endpoint: &'a str,
}

impl<'a, TStore: WalletStore> TransactionApi<'a, TStore> {
    pub fn new(store: &'a TStore, validator_node_jrpc_endpoint: &'a str) -> Self {
        Self {
            store,
            validator_node_jrpc_endpoint,
        }
    }

    pub fn get(&self, hash: FixedHash) -> Result<WalletTransaction, TransactionApiError> {
        let mut tx = self.store.create_read_tx()?;
        let transaction = tx.transaction_get(hash)?;
        Ok(transaction)
    }

    pub async fn submit_to_vn(&self, transaction: Transaction) -> Result<FixedHash, TransactionApiError> {
        self.submit_to_vn_internal(transaction, false).await
    }

    pub async fn submit_dry_run_to_vn(&self, transaction: Transaction) -> Result<FixedHash, TransactionApiError> {
        self.submit_to_vn_internal(transaction, true).await
    }

    async fn submit_to_vn_internal(
        &self,
        transaction: Transaction,
        is_dry_run: bool,
    ) -> Result<FixedHash, TransactionApiError> {
        self.store
            .with_write_tx(|tx| tx.transactions_insert(&transaction, is_dry_run))?;

        let mut client = self.get_validator_node_client()?;

        let resp = client
            .submit_transaction(SubmitTransactionRequest {
                transaction,
                wait_for_result: is_dry_run,
                wait_for_result_timeout: None,
                is_dry_run,
            })
            .await?;

        self.store.with_write_tx(|tx| {
            tx.transactions_set_result_and_status(
                resp.hash,
                resp.result.as_ref().map(|a| &a.finalize),
                resp.result.as_ref().and_then(|r| r.transaction_failure.as_ref()),
                resp.result
                    .as_ref()
                    .and_then(|a| a.fee_breakdown.as_ref())
                    .map(|b| b.total_fees_charged),
                None,
                if is_dry_run {
                    TransactionStatus::DryRun
                } else {
                    TransactionStatus::Pending
                },
            )
        })?;

        Ok(resp.hash)
    }

    pub fn fetch_all_by_status(
        &self,
        status: TransactionStatus,
    ) -> Result<Vec<WalletTransaction>, TransactionApiError> {
        let mut tx = self.store.create_read_tx()?;
        let transactions = tx.transactions_fetch_all_by_status(status)?;
        Ok(transactions)
    }

    fn get_validator_node_client(&self) -> Result<ValidatorNodeClient, TransactionApiError> {
        ValidatorNodeClient::connect(self.validator_node_jrpc_endpoint)
            .map_err(TransactionApiError::ValidatorNodeClientError)
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

        let mut client = self.get_validator_node_client()?;

        let maybe_resp = client
            .get_transaction_result(GetTransactionResultRequest { hash })
            .await
            .optional()
            .map_err(TransactionApiError::ValidatorNodeClientError)?;

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

        match resp.result {
            Some(result) => {
                let new_status = if result.finalize.result.is_accept() && result.transaction_failure.is_none() {
                    TransactionStatus::Accepted
                } else {
                    TransactionStatus::Rejected
                };

                let qc_resp = client
                    .get_transaction_quorum_certificates(GetTransactionQcsRequest { hash })
                    .await
                    .map_err(TransactionApiError::ValidatorNodeClientError)?;

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
                        Some(&qc_resp.qcs),
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
                    qcs: qc_resp.qcs,
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
        let mut component = None;
        let mut children = vec![];
        let mut downed_children = HashMap::<_, _>::new();

        for (addr, version) in diff.down_iter() {
            if addr.is_layer1_commitment() {
                info!(target: LOG_TARGET, "Layer 1 commitment {} downed", addr);
                continue;
            }
            let maybe_substate = tx
                .substates_remove(&VersionedSubstateAddress {
                    address: addr.clone(),
                    version: *version,
                })
                .optional()?;
            match maybe_substate {
                Some(substate) => {
                    if let Some(parent) = substate.parent_address {
                        downed_children.insert(substate.address.address, parent);
                    }
                },
                None => {
                    warn!(target: LOG_TARGET, "Downed substate {} not found", addr);
                },
            }
        }

        for (addr, substate) in diff.up_iter() {
            match addr {
                addr @ SubstateAddress::Component(_) => {
                    let header = substate.substate_value().component().unwrap();
                    tx.substates_insert_parent(
                        tx_hash,
                        VersionedSubstateAddress {
                            address: addr.clone(),
                            version: substate.version(),
                        },
                        header.module_name.clone(),
                        header.template_address,
                    )?;
                    component = Some(addr);
                },
                addr @ SubstateAddress::Resource(_)
                | addr @ SubstateAddress::Vault(_)
                | addr @ SubstateAddress::NonFungible(_)
                | addr @ SubstateAddress::NonFungibleIndex(_) => {
                    children.push(VersionedSubstateAddress {
                        address: addr.clone(),
                        version: substate.version(),
                    });
                },
                addr @ SubstateAddress::UnclaimedConfidentialOutput(_) => {
                    todo!("Not supported {}", addr);
                },
            }
        }

        for ch in children {
            match downed_children.remove(&ch.address) {
                Some(parent) => {
                    tx.substates_insert_child(
                        tx_hash,
                        parent,
                        VersionedSubstateAddress {
                            address: ch.address.clone(),
                            version: ch.version,
                        },
                    )?;
                },
                None => {
                    // FIXME: We dont really know what the parent is, so we just use a component from the transaction
                    //        because this is more likely than not to be correct. Obviously this is not good enough.
                    match component {
                        Some(parent) => {
                            warn!(
                                target: LOG_TARGET,
                                "Assuming parent component is {} for substate {} in transaction {}.",
                                parent,
                                ch,
                                tx_hash
                            );
                            tx.substates_insert_child(tx_hash, parent.clone(), ch)?;
                        },
                        None => {
                            warn!(
                                target: LOG_TARGET,
                                "No parent component found for substate {} in transaction {}.", ch, tx_hash
                            );
                            // FIXME: We don't have a component in this transaction with other upped substates.
                            tx.substates_insert_parent(
                                tx_hash,
                                ch,
                                "<unknown>".to_string(),
                                TemplateAddress::default(),
                            )?;
                        },
                    }
                },
            }
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Validator node client error: {0}")]
    ValidatorNodeClientError(#[from] ValidatorNodeClientError),
}

impl IsNotFoundError for TransactionApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
