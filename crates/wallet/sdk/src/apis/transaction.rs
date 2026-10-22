//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_engine_types::{
    commit_result::ExecuteResult,
    indexed_value::{IndexedValueError, IndexedWellKnownTypes},
    substate::{SubstateDiff, SubstateId},
};
use tari_indexer_client::types::ProvenTransactionReceipt;
use tari_ootle_common_types::{
    Epoch,
    VersionedSubstateIdRef,
    optional::{IsNotFoundError, Optional},
    response_status::{ResponseErrorStatus, TransactionStatusResponseError},
    substate_type::check_diff_substate_types,
};
use tari_ootle_light_client::{LightClientError, check_result_matches_receipt};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_template_lib::types::{ComponentAddress, constants::TARI_TOKEN};

use crate::{
    apis::validator_set::{ValidatorSetApi, ValidatorSetApiError},
    models::{NewAccountData, TransactionStatus, WalletLockId, WalletTransaction, WalletTransactionUpdate},
    network::{TransactionFinalizedResult, WalletNetworkInterface},
    sdk::IndexerTrust,
    storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter, WriteableWalletStore},
};

const LOG_TARGET: &str = "tari::ootle::wallet_sdk::apis::transaction";

/// Epochs past its `max_epoch` that the wallet keeps waiting for the outcome of a transaction the network reports no
/// outcome for. No shard group starts a transaction after its `max_epoch`, but one that prepared it by then waits an
/// epoch longer for the other shard groups, and accepts their proposals from the epoch before its own, so the network
/// can finalize it up to two epochs past its `max_epoch`. The third leaves the indexer time to serve that result.
pub const TRANSACTION_EXPIRY_GRACE_EPOCHS: u64 = 3;

pub struct TransactionApi<'a, TStore, TNetworkInterface> {
    store: &'a TStore,
    network_interface: &'a TNetworkInterface,
    validator_set_api: ValidatorSetApi<'a, TStore, TNetworkInterface>,
    indexer_trust: IndexerTrust,
}

impl<'a, TStore, TNetworkInterface> TransactionApi<'a, TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
    TNetworkInterface::Error: IsNotFoundError + TransactionStatusResponseError,
{
    pub fn new(
        store: &'a TStore,
        network_interface: &'a TNetworkInterface,
        validator_set_api: ValidatorSetApi<'a, TStore, TNetworkInterface>,
        indexer_trust: IndexerTrust,
    ) -> Self {
        Self {
            store,
            network_interface,
            validator_set_api,
            indexer_trust,
        }
    }

    pub fn get(&self, tx_id: TransactionId) -> Result<WalletTransaction, TransactionApiError> {
        let mut tx = self.store.create_read_tx()?;
        let transaction = tx.transactions_get(tx_id)?;
        Ok(transaction)
    }

    /// Inserts a new transaction into the wallet database with status `New`.
    ///
    /// `linked_accounts` are the wallet account(s) this transaction involves; they are persisted so the
    /// transaction list can be filtered per account. Accounts not known to this wallet are ignored.
    pub fn insert_new_transaction(
        &self,
        transaction: Transaction,
        new_account_info: Option<NewAccountData>,
        linked_accounts: &[ComponentAddress],
        is_dry_run: bool,
    ) -> Result<TransactionId, TransactionApiError> {
        let tx_id = transaction.calculate_id();
        self.store.with_write_tx(|tx| {
            tx.transactions_insert(&transaction, new_account_info.as_ref(), linked_accounts, is_dry_run)
        })?;

        Ok(tx_id)
    }

    /// Submits a transaction to the network. The transaction must be in the `New` status.
    /// If the submission is successful, the transaction status is updated to `Pending`.
    /// If the transaction is rejected, the status is updated to `InvalidTransaction` and the
    /// rejection reason is stored.
    /// A wallet that verifies its indexer but trusts no validator set yet, and cannot anchor on one now, returns
    /// [`Submission::Deferred`] and leaves the transaction `New`. The caller submits it again later.
    pub async fn submit_transaction(&self, transaction_id: TransactionId) -> Result<Submission, TransactionApiError> {
        let transaction = self.store.with_read_tx(|tx| tx.transactions_get(transaction_id))?;
        if transaction.is_dry_run {
            return Err(TransactionApiError::DryRunMismatchError {
                details: "Transaction is marked as dry run and cannot be submitted".to_string(),
            });
        }

        if !matches!(transaction.status, TransactionStatus::New) {
            return Err(TransactionApiError::StoreError(WalletStorageError::OperationError {
                operation: "submit_transaction",
                details: format!("Transaction {} is not in New status", transaction_id),
            }));
        }

        // A receipt proven in an epoch before the wallet's anchor never verifies, so a verifying wallet submits only
        // once it has an anchor.
        if self.indexer_trust.is_verify() &&
            self.validator_set_api.latest()?.is_none() &&
            let Err(err) = self.validator_set_api.sync().await
        {
            debug!(target: LOG_TARGET, "Transaction {transaction_id} waits to be submitted until the wallet trusts a validator set: {err}");
            return Ok(Submission::Deferred);
        }

        // Re-submission needs the full transaction (blob payloads); the WalletTransaction
        // returned above is the pruned API view.
        let full = self.store.with_read_tx(|tx| tx.transactions_get_full(transaction_id))?;
        let resp = self.network_interface.submit_transaction(full).await;

        match resp {
            Ok(_) => {
                self.store.with_write_tx(|tx| {
                    tx.transactions_update(
                        WalletTransactionUpdate::new(transaction_id).with_new_status(TransactionStatus::Pending),
                    )
                })?;
            },
            Err(err) => {
                return match err.get_status() {
                    ResponseErrorStatus::TransactionRejected { message } => {
                        warn!(target: LOG_TARGET, "Invalid transaction submission: {transaction_id} {message}");
                        self.store.with_write_tx(|tx| {
                            tx.transactions_update(
                                WalletTransactionUpdate::new(transaction_id)
                                    .with_new_status(TransactionStatus::InvalidTransaction)
                                    .with_invalid_reason(&message),
                            )
                        })?;
                        Ok(Submission::Rejected)
                    },
                    _ => Err(err.into()),
                };
            },
        }

        Ok(Submission::Submitted)
    }

    /// Executes `transaction` as a dry run and records what it cost.
    ///
    /// `settled_fee` is the fee to record as the one to submit with, for a caller that chose a
    /// figure and is only having the run confirm it. `None` records `required_fees` — what the run
    /// cost plus the allowance for the metering a wider `max_fee` causes — which is the right answer
    /// wherever the fee does not feed back into the transaction. Where it does, it is not: stealth
    /// input selection targets `amount + max_fee`, so submitting at any figure other than one a
    /// build was made at selects different inputs and prices a different transaction.
    pub async fn submit_dry_run_transaction(
        &self,
        transaction: Transaction,
        settled_fee: Option<u64>,
    ) -> Result<WalletTransaction, TransactionApiError> {
        if !transaction.is_dry_run() {
            return Err(TransactionApiError::DryRunMismatchError {
                details: "Transaction is not marked as dry run but submitted to dry-run".to_string(),
            });
        }

        self.store
            .with_write_tx(|tx| tx.transactions_insert(&transaction, None, &[], true))?;

        let tx_id = transaction.calculate_id();
        let result = self.network_interface.submit_dry_run_transaction(transaction).await;

        match result {
            Ok(query) => match &query.result {
                TransactionFinalizedResult::Pending => {
                    return Err(TransactionApiError::InvalidTransactionQueryResponse {
                        details: "Pending execution result returned from dry run".to_string(),
                    });
                },
                TransactionFinalizedResult::Rejected { details, .. } => {
                    self.store.with_write_tx(|tx| {
                        tx.transactions_update(
                            WalletTransactionUpdate::new(tx_id)
                                .with_new_status(TransactionStatus::DryRunFailed)
                                .with_invalid_reason(details),
                        )
                    })?;
                    return Err(TransactionApiError::InvalidTransactionQueryResponse {
                        details: format!("Transaction rejected by dry run: {details}"),
                    });
                },
                TransactionFinalizedResult::Finalized {
                    execution_result,
                    finalized_time,
                    execution_time,
                    ..
                } => {
                    if query.transaction_id != tx_id {
                        // This could indicate that there has been some breaking change that caused the indexer to
                        // calculate a different transaction ID
                        warn!(target: LOG_TARGET, "⚠️ Transaction ID mismatch in dry run response. Expected {}, got {}. Updating transaction status to DryRunFailed.", tx_id, query.transaction_id);
                    }

                    let final_fee =
                        settled_fee.or_else(|| execution_result.as_ref().map(|e| e.finalize.required_fees()));

                    self.store.with_write_tx(|tx| {
                        tx.transactions_update(
                            WalletTransactionUpdate::new(tx_id)
                                .with_result(execution_result.as_ref().map(|e| &e.finalize))
                                .with_final_fee(final_fee)
                                .with_new_status(TransactionStatus::DryRun)
                                .with_execution_time(*execution_time)
                                .with_finalized_time(*finalized_time),
                        )
                    })?;
                },
            },
            Err(err) => {
                self.store.with_write_tx(|tx| {
                    tx.transactions_update(
                        WalletTransactionUpdate::new(tx_id).with_new_status(TransactionStatus::DryRunFailed),
                    )
                })?;
                return Err(err.into());
            },
        }

        let transaction = self.store.with_read_tx(|tx| tx.transactions_get(tx_id))?;

        Ok(transaction)
    }

    pub fn fetch_all(
        &self,
        status: Option<TransactionStatus>,
        account: Option<ComponentAddress>,
    ) -> Result<Vec<WalletTransaction>, TransactionApiError> {
        let mut tx = self.store.create_read_tx()?;
        let transactions = tx.transactions_fetch_all(status, account)?;
        Ok(transactions)
    }

    /// Stores the result of `transaction_id` once the network has finalized it, and commits its changes to the wallet.
    /// Returns `None` while the transaction is pending.
    ///
    /// A transaction the network still reports no outcome for once it can no longer finalize is stored as invalid,
    /// and the inputs locked for it are released (see [`TRANSACTION_EXPIRY_GRACE_EPOCHS`]).
    ///
    /// Under [`IndexerTrust::Verify`] a committed transaction stays pending until the indexer serves a proof of its
    /// receipt that verifies against a validator set the wallet trusts, and its result must be the one that receipt
    /// commits.
    #[allow(clippy::too_many_lines)]
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
            .query_transaction_result(transaction_id, self.indexer_trust.is_verify())
            .await
            .optional()?;

        let Some(resp) = maybe_resp else {
            warn!( target: LOG_TARGET, "Transaction result not found for transaction with hash {}. Will check again later.", transaction_id);
            return self.expire_if_past_validity(&transaction).await;
        };

        match resp.result {
            TransactionFinalizedResult::Pending => self.expire_if_past_validity(&transaction).await,
            TransactionFinalizedResult::Rejected { details, rejected_time } => {
                let transaction = self.store.with_write_tx(|tx| {
                    tx.transactions_update(
                        WalletTransactionUpdate::new(transaction_id)
                            .with_new_status(TransactionStatus::InvalidTransaction)
                            .with_invalid_reason(&details)
                            .with_finalized_time(rejected_time),
                    )?;
                    self.release_all_locks_for_transaction_internal(tx, transaction_id)?;
                    let transaction = tx.transactions_get(transaction_id)?;
                    Ok::<_, TransactionApiError>(transaction)
                })?;

                Ok(Some(transaction))
            },
            TransactionFinalizedResult::Finalized {
                final_decision,
                execution_result,
                execution_time,
                finalized_time,
                ..
            } => {
                if let Some(diff) = execution_result.as_ref().and_then(|e| e.finalize.result.any_accept()) {
                    check_diff_substate_types(diff)
                        .map_err(|e| TransactionApiError::InvalidTransactionQueryResponse { details: e.to_string() })?;
                }

                if self.indexer_trust.is_verify() && !transaction.is_dry_run && final_decision.is_commit() {
                    let verified = self
                        .verify_committed_result(transaction_id, execution_result.as_deref(), resp.receipt.as_ref())
                        .await?;
                    if !verified {
                        return Ok(None);
                    }
                }

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

                let transaction = self.store.with_write_tx(|tx| {
                    if !transaction.is_dry_run && final_decision.is_commit() {
                        let diff = execution_result
                            .as_ref()
                            .and_then(|e| e.finalize.result.any_accept())
                            .ok_or_else(|| TransactionApiError::InvalidTransactionQueryResponse {
                                details: format!(
                                    "NEVERHAPPEN: Finalize decision is COMMIT but transaction failed: {:?}",
                                    execution_result.as_ref().and_then(|e| e.finalize.result.fee_reject())
                                ),
                            })?;

                        self.commit_diff(tx, diff)?;
                    }

                    let final_fee = execution_result
                        .as_ref()
                        .map(|e| e.finalize.fee_receipt.total_fees_paid());

                    tx.transactions_update(
                        WalletTransactionUpdate::new(transaction_id)
                            .with_result(execution_result.as_ref().map(|e| &e.finalize))
                            .with_final_fee(final_fee)
                            .with_new_status(new_status)
                            .with_execution_time(execution_time)
                            .with_finalized_time(finalized_time),
                    )?;


                    // Make sure that any locked outputs are either set to spent or released, depending on if the
                    // transaction was finalized or rejected. Always release for dry runs. The decision says whether
                    // the transaction spent its inputs: an abort releases them even when the execution before it
                    // accepted a diff.
                    if transaction.is_dry_run {
                        self.release_all_locks_for_transaction_internal(tx, transaction_id)?;
                    } else {
                        let maybe_diff = execution_result
                            .as_ref()
                            .and_then(|e| e.finalize.result.any_accept())
                            .filter(|_| final_decision.is_commit());
                        match maybe_diff {
                            Some(diff) => {
                                if let Some(lock_id) = tx.locks_get_by_transaction_id(transaction_id).optional()? {
                                    info!(target: LOG_TARGET, "Finalizing locked outputs for transaction {}: {}", transaction_id, lock_id);
                                    tx.locks_unlock_finalized(lock_id, diff)?;
                                }
                            }
                            None => {
                                self.release_all_locks_for_transaction_internal(tx, transaction_id)?;
                            }
                        }
                    }

                    let transaction = tx.transactions_get(transaction_id)?;
                    Ok::<_, TransactionApiError>(transaction)
                })?;

                Ok(Some(transaction))
            },
        }
    }

    /// Stores `transaction`, which the network reports no outcome for, as invalid once the current epoch is more than
    /// [`TRANSACTION_EXPIRY_GRACE_EPOCHS`] past its `max_epoch`, and releases the inputs locked for it. Returns `None`
    /// while the network may still finalize it.
    async fn expire_if_past_validity(
        &self,
        transaction: &WalletTransaction,
    ) -> Result<Option<WalletTransaction>, TransactionApiError> {
        let max_epoch = transaction.transaction.max_epoch();
        let Some(current_epoch) = self.current_epoch().await? else {
            return Ok(None);
        };
        if current_epoch.as_u64() <= max_epoch.as_u64().saturating_add(TRANSACTION_EXPIRY_GRACE_EPOCHS) {
            return Ok(None);
        }

        let transaction_id = transaction.id;
        let reason = format!(
            "Expired: the network reported no outcome by epoch {}, more than {TRANSACTION_EXPIRY_GRACE_EPOCHS} epochs \
             past the transaction's max_epoch {}",
            current_epoch.as_u64(),
            max_epoch.as_u64()
        );
        warn!(target: LOG_TARGET, "Transaction {transaction_id}: {reason}. Releasing its inputs.");
        let transaction = self.store.with_write_tx(|tx| {
            tx.transactions_update(
                WalletTransactionUpdate::new(transaction_id)
                    .with_new_status(TransactionStatus::InvalidTransaction)
                    .with_invalid_reason(&reason),
            )?;
            self.release_all_locks_for_transaction_internal(tx, transaction_id)?;
            let transaction = tx.transactions_get(transaction_id)?;
            Ok::<_, TransactionApiError>(transaction)
        })?;
        Ok(Some(transaction))
    }

    /// The epoch the network has reached, or `None` while the wallet cannot tell.
    ///
    /// Under [`IndexerTrust::Verify`] this is the epoch of the latest validator set the wallet trusts, which the
    /// network has reached whatever the indexer reports.
    async fn current_epoch(&self) -> Result<Option<Epoch>, TransactionApiError> {
        if self.indexer_trust.is_verify() {
            return Ok(self.validator_set_api.latest()?.map(|set| set.epoch()));
        }
        Ok(Some(self.network_interface.get_current_epoch().await?))
    }

    /// Verifies that the network committed `execution_result` as the result of `transaction_id`: the indexer's proof of
    /// the committed receipt verifies against a validator set the wallet trusts, and the result is the one that
    /// receipt commits. Returns `false` while there is no proof to verify yet.
    async fn verify_committed_result(
        &self,
        transaction_id: TransactionId,
        execution_result: Option<&ExecuteResult>,
        proven: Option<&ProvenTransactionReceipt>,
    ) -> Result<bool, TransactionApiError> {
        let Some((proven, proof)) = proven.and_then(|proven| proven.proof.as_ref().map(|proof| (proven, proof))) else {
            info!(
                target: LOG_TARGET,
                "Transaction {} committed, but the indexer served no proof of its receipt yet. Will check again later.",
                transaction_id
            );
            return Ok(false);
        };
        let Some(committee) = self
            .validator_set_api
            .verify_committee(proof.anchor.epoch, proof.anchor.shard_group)
            .await?
        else {
            info!(
                target: LOG_TARGET,
                "The receipt of transaction {} is proven in {}, for which the wallet trusts no validator set yet. Will check again later.",
                transaction_id,
                proof.anchor.epoch
            );
            return Ok(false);
        };
        let unverified = |source| TransactionApiError::FinalizationUnverified { transaction_id, source };
        committee.verify_receipt(transaction_id, proven).map_err(unverified)?;
        let execution_result =
            execution_result.ok_or_else(|| TransactionApiError::InvalidTransactionQueryResponse {
                details: format!("Transaction {transaction_id} committed, but the indexer served no result"),
            })?;
        check_result_matches_receipt(
            committee.network(),
            transaction_id,
            &proven.receipt,
            &execution_result.finalize,
        )
        .map_err(unverified)?;
        Ok(true)
    }

    pub fn release_all_locks_for_transaction(&self, transaction_id: TransactionId) -> Result<(), TransactionApiError> {
        self.store
            .with_write_tx(|tx| self.release_all_locks_for_transaction_internal(tx, transaction_id))
    }

    pub fn locks_set_transaction_id(
        &self,
        lock_id: WalletLockId,
        transaction_id: TransactionId,
    ) -> Result<(), TransactionApiError> {
        self.store
            .with_write_tx(|tx| tx.locks_link_transaction(lock_id, transaction_id))?;
        Ok(())
    }

    fn release_all_locks_for_transaction_internal(
        &self,
        tx: &mut <TStore as WriteableWalletStore>::WriteTransaction<'_>,
        transaction_id: TransactionId,
    ) -> Result<(), TransactionApiError> {
        if let Some(lock_id) = tx.locks_get_by_transaction_id(transaction_id).optional()? {
            debug!(target: LOG_TARGET, "Releasing lock {} (and associated outputs) for transaction {} that was not committed", lock_id, transaction_id);
            tx.locks_release(lock_id)?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn commit_diff(
        &self,
        tx: &mut TStore::WriteTransaction<'_>,
        diff: &SubstateDiff,
    ) -> Result<(), TransactionApiError> {
        let mut downed_substates_with_parents = HashMap::with_capacity(diff.down_len());
        for (id, _) in diff.down_iter() {
            if id.is_claimed_output_tombstone() {
                // Should never happen
                warn!(target: LOG_TARGET, "❓️ Claimed tombstone {} downed", id);
                continue;
            }

            let Some(downed) = tx.substates_remove(id).optional()? else {
                debug!(target: LOG_TARGET, "Downed substate {} not found", id);
                continue;
            };

            if let Some(parent) = downed.parent_address {
                downed_substates_with_parents.insert(downed.substate_id.into_substate_id(), parent);
            }
        }

        let (components, mut other_substates) = diff.up_iter().partition::<Vec<_>, _>(|(addr, _)| addr.is_component());

        for (component_addr, substate) in components {
            let component = substate.substate_value().component().ok_or_else(|| {
                TransactionApiError::InvalidTransactionQueryResponse {
                    details: format!("Substate {component_addr} in the transaction diff is not a component"),
                }
            })?;
            let indexed =
                IndexedWellKnownTypes::from_value(component.state()).map_err(TransactionApiError::IndexedValueError)?;

            debug!(target: LOG_TARGET, "Substate {} up", component_addr);
            tx.substates_upsert_root(
                VersionedSubstateIdRef::new(component_addr, substate.version()),
                indexed.referenced_substates().collect(),
                // TODO: we can clean out module name, this is never populated
                None,
                Some(*component.template_address()),
            )?;

            for owned_id in indexed.referenced_substates() {
                if let Some(pos) = other_substates.iter().position(|(addr, _)| *addr == owned_id) {
                    let (_, child) = other_substates.swap_remove(pos);
                    // If there was a previous parent for this substate, we keep it as is.
                    let parent = downed_substates_with_parents
                        .get(&owned_id)
                        .cloned()
                        .unwrap_or_else(|| component_addr.clone());

                    if let Some(vault_id) = owned_id.as_vault_id() {
                        if let Some(vault) = tx.vaults_get(&vault_id).optional()? {
                            // The vault for an account may have been mutated without mutating the account component
                            // If we know this vault, set it as a child of the account
                            tx.substates_upsert_child(
                                &vault.account_address.into(),
                                VersionedSubstateIdRef::new(&owned_id, child.version()),
                                [vault.resource_address.into()].into_iter().collect(),
                            )?;
                            if let Some(resource) = tx.substates_get(&vault.resource_address.into()).optional()? {
                                tx.substates_upsert_child(
                                    &vault.account_address.into(),
                                    resource.substate_id.as_versioned_ref(),
                                    HashSet::new(),
                                )?;
                            }
                        } else {
                            // A vault created by this transaction is not in the wallet's vault table yet,
                            // but it is referenced by this component, so record the parent linkage.
                            let vault = child.substate_value().vault().ok_or_else(|| {
                                TransactionApiError::InvalidTransactionQueryResponse {
                                    details: format!("Substate {owned_id} in the transaction diff is not a vault"),
                                }
                            })?;
                            tx.substates_upsert_child(
                                &parent,
                                VersionedSubstateIdRef::new(&owned_id, child.version()),
                                [(*vault.resource_address()).into()].into_iter().collect(),
                            )?;
                        }
                        continue;
                    }

                    let maybe_substate = tx.substates_get(&owned_id).optional()?;
                    tx.substates_upsert_child(
                        &parent,
                        VersionedSubstateIdRef::new(&owned_id, child.version()),
                        maybe_substate
                            .map(|s| s.referenced_substates.into_iter().collect())
                            .unwrap_or_default(),
                    )?;
                }
            }
        }

        for (id, substate) in other_substates {
            match id {
                SubstateId::Component(_) => unreachable!(),
                SubstateId::Resource(_) => match tx.substates_get(id).optional()? {
                    Some(known_substate) => {
                        tx.substates_upsert_root(
                            VersionedSubstateIdRef::new(id, substate.version()),
                            known_substate.referenced_substates.into_iter().collect(),
                            known_substate.module_name,
                            known_substate.template_address,
                        )?;
                    },
                    None => {
                        tx.substates_upsert_root(
                            VersionedSubstateIdRef::new(id, substate.version()),
                            Default::default(),
                            None,
                            None,
                        )?;
                    },
                },
                SubstateId::Vault(vault_id) => {
                    match tx.vaults_get(vault_id).optional()? {
                        Some(vault) => {
                            // The vault for an account may have been mutated without mutating the account component
                            // If we know this vault, set it as a child of the account
                            tx.substates_upsert_child(
                                &vault.account_address.into(),
                                VersionedSubstateIdRef::new(id, substate.version()),
                                [vault.resource_address.into()].into_iter().collect(),
                            )?;
                            if let Some(resource) = tx.substates_get(&vault.resource_address.into()).optional()? {
                                tx.substates_upsert_child(
                                    &vault.account_address.into(),
                                    resource.substate_id.as_versioned_ref(),
                                    HashSet::new(),
                                )?;
                            }
                        },
                        None => {
                            // We don't know the parent account of this vault.
                            debug!(target: LOG_TARGET, "Vault {} does not have a parent", vault_id);
                            // tx.substates_upsert_root(
                            //     VersionedSubstateIdRef::new(id, substate.version()),
                            //     [(*substate
                            //         .substate_value()
                            //         .vault()
                            //         .expect("should be vault")
                            //         .resource_address())
                            //     .into()]
                            //     .into_iter()
                            //     .collect(),
                            //     None,
                            //     None,
                            // )?;
                        },
                    }
                    continue;
                },
                SubstateId::ClaimedOutputTombstone(_) => {
                    tx.substates_upsert_root(
                        VersionedSubstateIdRef::new(id, substate.version()),
                        [TARI_TOKEN.into()].into_iter().collect(),
                        None,
                        None,
                    )?;
                },
                SubstateId::NonFungible(nft) => {
                    let resource_address = nft.resource_address();
                    let referenced_data = substate
                        .substate_value()
                        .non_fungible()
                        .and_then(|s| s.contents())
                        .map(|c| IndexedWellKnownTypes::from_value(c.data()))
                        .transpose()
                        .map_err(TransactionApiError::IndexedValueError)?;
                    let referenced_mdata = substate
                        .substate_value()
                        .non_fungible()
                        .and_then(|s| s.contents())
                        .map(|c| IndexedWellKnownTypes::from_value(c.mutable_data()))
                        .transpose()
                        .map_err(TransactionApiError::IndexedValueError)?;
                    tx.substates_upsert_child(
                        &SubstateId::Resource(*resource_address),
                        VersionedSubstateIdRef::new(id, substate.version()),
                        referenced_data
                            .into_iter()
                            .chain(referenced_mdata)
                            .flat_map(|s| s.into_referenced_substates())
                            .collect(),
                    )?;
                },
                SubstateId::TransactionReceipt(_) | SubstateId::Template(_) | SubstateId::ValidatorFeePool(_) => {
                    tx.substates_upsert_root(
                        VersionedSubstateIdRef::new(id, substate.version()),
                        Default::default(),
                        None,
                        None,
                    )?;
                },
                SubstateId::Utxo(addr) => {
                    tx.substates_upsert_root(
                        VersionedSubstateIdRef::new(id, substate.version()),
                        [(*addr.resource_address()).into()].into_iter().collect(),
                        None,
                        None,
                    )?;
                },
                SubstateId::ConfidentialOutput(addr) => {
                    tx.substates_upsert_root(
                        VersionedSubstateIdRef::new(id, substate.version()),
                        [(*addr.resource_address()).into()].into_iter().collect(),
                        None,
                        None,
                    )?;
                },
            }
        }

        Ok(())
    }
}

/// What [`TransactionApi::submit_transaction`] did with a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    /// The network accepted the transaction, which is now `Pending`.
    Submitted,
    /// The network refused the transaction, which is now `InvalidTransaction`.
    Rejected,
    /// The wallet verifies results and trusts no validator set yet, so the transaction stays `New` for a later
    /// submission.
    Deferred,
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    // `status` is the categorized variant used by callers to map to a response; `message` is the full
    // detail. Only `message` is shown here because `status`'s Display embeds the same message, which
    // would otherwise print the error twice.
    #[error("Network interface error: {message}")]
    NetworkInterfaceError {
        status: ResponseErrorStatus,
        message: String,
    },
    #[error("Failed to extract known type data from value: {0}")]
    IndexedValueError(IndexedValueError),
    #[error("Invalid transaction query response: {details}")]
    InvalidTransactionQueryResponse { details: String },
    #[error("Dry run transaction mismatch error: {details}")]
    DryRunMismatchError { details: String },
    #[error("The indexer's finalization of transaction {transaction_id} does not verify: {source}")]
    FinalizationUnverified {
        transaction_id: TransactionId,
        source: LightClientError,
    },
    #[error("Validator set error: {0}")]
    ValidatorSet(#[from] ValidatorSetApiError),
}

impl IsNotFoundError for TransactionApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}

impl<T: TransactionStatusResponseError> From<T> for TransactionApiError {
    fn from(value: T) -> Self {
        TransactionApiError::NetworkInterfaceError {
            status: value.get_status(),
            message: value.get_error_message(),
        }
    }
}
