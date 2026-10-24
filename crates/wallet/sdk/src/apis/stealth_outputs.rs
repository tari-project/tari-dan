//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use ootle_byte_type::{FromByteType, ToByteType};
use ootle_network::Network;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, pedersen::PedersenCommitment},
};
use tari_engine_types::{Utxo, UtxoOutput, component::derive_component_address_from_public_key, limits};
use tari_ootle_address::RistrettoOotleAddress;
use tari_ootle_common_types::{
    displayable::Displayable,
    optional::{IsNotFoundError, Optional},
};
use tari_ootle_transaction::MAX_TRANSACTION_INPUTS;
use tari_ootle_wallet_crypto::{
    DecryptedData,
    OutputWitness,
    StealthCryptoApi,
    StealthCryptoApiError,
    StealthInputWitness,
    StealthOutputWitness,
    memo::Memo,
    pay_to::PayTo,
    stealth::pay_to_output_authorization,
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    EncryptedData,
    ResourceAddress,
    UtxoAddress,
    access_rules::AccessRule,
    crypto::PedersenCommitmentBytes,
    stealth::{RevealedOutput, SpendAuthorization, StealthTransferStatement},
};

use crate::{
    WalletSdkSpec,
    apis::{
        accounts::AccountsApiError,
        config::{ConfigApi, ConfigApiError},
        key_manager::{KeyManagerApi, KeyManagerApiError},
        stealth_transfer::{StealthOutputToCreate, UnblindedInputToSpend},
    },
    models::{
        AccountAndViewKeys,
        InputSpendData,
        KeyBranch,
        KeyId,
        OutputStatus,
        StealthBalance,
        StealthOutputInfo,
        StealthOutputModel,
        WalletLockId,
        input_selection,
        input_selection::{InputSelectionAlgorithm, branch_and_bound::KeyedInput},
    },
    storage::{
        CommittableStore,
        ReadableWalletStore,
        WalletStorageError,
        WalletStoreReader,
        WalletStoreWriter,
        WriteableWalletStore,
    },
};

/// Declared inputs a stealth transaction holds back for the substates around its UTXOs: the resources it reads, the
/// account and vault it may spend from, and any swap or badge inputs.
pub const NON_UTXO_INPUT_RESERVE: usize = 32;

/// Stealth inputs one transaction may spend, across every statement it carries.
///
/// The engine's ceiling is per transaction rather than per statement — `max_total_inputs_per_transaction` against
/// `max_inputs` — and a transfer that cannot source its fee from its own revealed remainder carries a second statement
/// with inputs of its own. Selecting against the per-statement limit for each of them builds a transaction the engine
/// refuses outright (`ExceedsStealthTransactionLimit { limit: "inputs" }`), and which ingress refuses before that, so
/// the two selections share this budget instead.
///
/// Each spent UTXO is also a declared input, so the budget must leave [`NON_UTXO_INPUT_RESERVE`] of
/// [`MAX_TRANSACTION_INPUTS`] for the transaction's other inputs.
pub const MAX_INPUTS_PER_TRANSACTION: usize = {
    let declared = MAX_TRANSACTION_INPUTS - NON_UTXO_INPUT_RESERVE;
    let stealth = limits::STEALTH_LIMITS.max_total_inputs_per_transaction;
    if declared < stealth { declared } else { stealth }
};

/// Inputs held back from a transfer's own selection for the statement that sources its fee.
///
/// The fee intent runs on `FREE_COMPUTE_GRACE_POINTS` of credit before anything is paid, and that credit is sized for
/// a fee-sourcing transfer of up to 64 dust inputs. A wider fee statement traps out of gas whatever else the
/// transaction does, so 64 is both what a transfer must leave behind and what the fee selection itself is capped at.
pub const FEE_INTENT_INPUT_RESERVE: usize = 64;

/// Inputs a transfer may select, leaving [`FEE_INTENT_INPUT_RESERVE`] for a fee statement.
pub const MAX_TRANSFER_INPUTS: usize = MAX_INPUTS_PER_TRANSACTION - FEE_INTENT_INPUT_RESERVE;

// A statement has a width limit of its own, below the per-transaction total these budgets are carved from, so a
// reserve much smaller than this one would leave a transfer free to select more inputs than any single statement may
// spend — and the statement it built would be refused while the transaction stayed inside its total.
const _: () = assert!(MAX_TRANSFER_INPUTS <= limits::STEALTH_LIMITS.max_inputs);
const _: () = assert!(FEE_INTENT_INPUT_RESERVE <= limits::STEALTH_LIMITS.max_inputs);

/// What a selection locked, and whether the input budget rather than the balance is what stopped it.
struct Selection {
    outputs: Vec<InputSpendData>,
    total: Amount,
    reached_input_limit: bool,
}

const LOG_TARGET: &str = "tari::ootle::wallet::apis::stealth_outputs";

pub struct StealthOutputsApi<'a, TSpec: WalletSdkSpec> {
    store: &'a TSpec::Store,
    key_manager_api: KeyManagerApi<'a, TSpec>,
    crypto_api: StealthCryptoApi,
    config_api: ConfigApi<'a, TSpec::Store>,
}

impl<'a, TSpec: WalletSdkSpec> StealthOutputsApi<'a, TSpec> {
    pub fn new(
        store: &'a TSpec::Store,
        key_manager_api: KeyManagerApi<'a, TSpec>,
        crypto_api: StealthCryptoApi,
        config_api: ConfigApi<'a, TSpec::Store>,
    ) -> Self {
        Self {
            store,
            key_manager_api,
            crypto_api,
            config_api,
        }
    }

    /// Locks as many outputs required to reach at least the specified amount, selecting at most `max_inputs` of them
    /// (see [`MAX_TRANSFER_INPUTS`]). If there are insufficient funds, an `InsufficientFunds` error is returned and no
    /// outputs are locked; if the funds are there but no `max_inputs` of them reach the amount, the error says so
    /// rather than reporting the balance as insufficient.
    pub fn lock_outputs_for_at_least_amount<A: Into<Amount>>(
        &self,
        account_address: &ComponentAddress,
        resource_address: &ResourceAddress,
        lock_id: WalletLockId,
        amount: A,
        max_inputs: usize,
    ) -> Result<(Vec<InputSpendData>, Amount), StealthOutputsApiError> {
        let amount = amount
            .into()
            .non_negative_checked()
            .ok_or_else(|| StealthOutputsApiError::InvalidParameter {
                param: "amount",
                reason: "lock_outputs_for_at_least_amount: Amount must be non-negative".to_string(),
            })?;
        if amount.is_zero() {
            return Ok((Vec::new(), Amount::zero()));
        }

        self.store.with_write_tx(|tx| {
            let selection = self.lock_outputs_internal(
                tx,
                account_address,
                resource_address,
                amount,
                lock_id,
                InputSelectionAlgorithm::BranchAndBound,
                max_inputs,
            )?;

            if selection.total < amount {
                if selection.reached_input_limit {
                    return Err(StealthOutputsApiError::InputLimitReached {
                        max: max_inputs,
                        locked: selection.total,
                        required: amount,
                    });
                }
                return Err(StealthOutputsApiError::InsufficientFunds);
            }
            Ok((selection.outputs, selection.total))
        })
    }

    /// Locks as many outputs required to reach at least the specified amount, selecting at most `max_inputs` of them
    /// (see [`MAX_TRANSFER_INPUTS`]). If the available outputs or `max_inputs` fall short, all that could be locked is
    /// returned along with the total amount locked, for the caller to top up from revealed funds.
    pub fn lock_outputs_until_partial_amount(
        &self,
        account_address: &ComponentAddress,
        resource_address: &ResourceAddress,
        amount: Amount,
        locked_by_id: WalletLockId,
        max_inputs: usize,
    ) -> Result<(Vec<InputSpendData>, Amount), StealthOutputsApiError> {
        self.store.with_write_tx(|tx| {
            let selection = self.lock_outputs_internal(
                tx,
                account_address,
                resource_address,
                amount,
                locked_by_id,
                InputSelectionAlgorithm::SmallestFirst,
                max_inputs,
            )?;
            Ok((selection.outputs, selection.total))
        })
    }

    /// Atomically resolves, validates, and locks the exact wallet-owned stealth outputs named by `utxo_addresses` for
    /// spending under `lock_id`. This is the specific-input counterpart to [`Self::lock_outputs_for_at_least_amount`]:
    /// the caller names precise UTXOs (by [`UtxoAddress`]) rather than an amount plus a selection strategy.
    ///
    /// Every requested output must use `resource_address`, belong to `account_address`, be wallet-owned (its one-time
    /// spend key is held) and currently eligible for the key-path statement flow — the same eligibility rules as
    /// [`WalletStoreReader::stealth_outputs_get_unspent_for_spending`] (owner key present, not burnt, not frozen,
    /// condition-spendable / key path), with status and on-chain handled as follows:
    /// - `Unspent` outputs must be confirmed on-chain.
    /// - `LockedUnconfirmed` outputs are accepted only when already associated with `lock_id` (they are off-chain by
    ///   construction, created earlier in this lock's transaction chain).
    /// - Any other status, a `LockedUnconfirmed` under a different lock, or any other off-chain output is rejected.
    ///
    /// Script-path/condition-tree outputs are rejected — spending those requires the separate generic `SpendWitness`
    /// follow-up.
    ///
    /// Resolution/validation and locking run inside a single write transaction, so the result is all-or-nothing: if any
    /// requested output is missing or ineligible the transaction rolls back and none are newly locked. The returned
    /// [`InputSpendData`] preserve the caller's requested `utxo_addresses` order.
    pub fn lock_specific_outputs(
        &self,
        account_address: &ComponentAddress,
        resource_address: &ResourceAddress,
        lock_id: WalletLockId,
        utxo_addresses: &[UtxoAddress],
    ) -> Result<Vec<InputSpendData>, StealthOutputsApiError> {
        if utxo_addresses.is_empty() {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "utxo_addresses",
                reason: "At least one UTXO address is required".to_string(),
            });
        }

        // Named inputs are spent by one transfer statement, the same as selected ones, so they take the same budget:
        // enough to keep that statement inside the per-statement cap and to leave the fee statement its reserve.
        if utxo_addresses.len() > MAX_TRANSFER_INPUTS {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "utxo_addresses",
                reason: format!(
                    "Requested {} inputs which exceeds the maximum of {} a transfer statement may spend",
                    utxo_addresses.len(),
                    MAX_TRANSFER_INPUTS
                ),
            });
        }

        // Reject wrong-resource / mixed-resource requests and duplicates before touching the database.
        let mut seen = HashSet::with_capacity(utxo_addresses.len());
        for address in utxo_addresses {
            if address.resource_address() != resource_address {
                return Err(StealthOutputsApiError::InvalidParameter {
                    param: "utxo_addresses",
                    reason: format!("UTXO {} does not use resource {}", address, resource_address),
                });
            }
            if !seen.insert(address) {
                return Err(StealthOutputsApiError::InvalidParameter {
                    param: "utxo_addresses",
                    reason: format!("Duplicate UTXO address {}", address),
                });
            }
        }

        self.store.with_write_tx(|tx| {
            let mut inputs = Vec::with_capacity(utxo_addresses.len());
            // Resolve and validate every requested UTXO before locking anything.
            for address in utxo_addresses {
                let commitment = address.id().into_commitment_bytes();
                let output = tx
                    .stealth_outputs_get_by_commitment(resource_address, &commitment)
                    .optional()?
                    .ok_or_else(|| StealthOutputsApiError::InvalidParameter {
                        param: "utxo_addresses",
                        reason: format!("UTXO {} is not known to this wallet", address),
                    })?;
                self.validate_output_for_specific_spend(account_address, lock_id, address, &output)?;
                inputs.push(output.into_spend_data());
            }

            // Account-scoped lock (defence in depth): ownership was validated above, but the storage update is also
            // restricted to this account so a caller-supplied commitment can never lock another account's row.
            let commitments = inputs.iter().map(|i| &i.commitment).collect::<Vec<_>>();
            tx.stealth_outputs_lock_many_for_account(account_address, resource_address, &commitments, lock_id)?;

            Ok(inputs)
        })
    }

    /// Validates that a resolved stealth output is a wallet-owned, key-path-spendable input for `account_address` that
    /// is currently eligible to be locked under `lock_id`. Mirrors the eligibility predicate of
    /// [`WalletStoreReader::stealth_outputs_get_unspent_for_spending`], with an explicit account-ownership check so a
    /// caller cannot select another account's output. Status and on-chain are evaluated together: an `Unspent` output
    /// must be on-chain, while a `LockedUnconfirmed` output is accepted only when already bound to `lock_id` (such
    /// outputs are off-chain by construction).
    fn validate_output_for_specific_spend(
        &self,
        account_address: &ComponentAddress,
        lock_id: WalletLockId,
        address: &UtxoAddress,
        output: &StealthOutputModel,
    ) -> Result<(), StealthOutputsApiError> {
        if output.owner_account != *account_address {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "utxo_addresses",
                reason: format!("UTXO {} does not belong to account {}", address, account_address),
            });
        }
        if output.owner_key_id.is_none() {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "utxo_addresses",
                reason: format!("UTXO {} is view-only and cannot be spent by this wallet", address),
            });
        }
        if output.is_burnt {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "utxo_addresses",
                reason: format!("UTXO {} is burnt", address),
            });
        }
        if output.is_frozen {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "utxo_addresses",
                reason: format!("UTXO {} is frozen", address),
            });
        }
        // Key-path eligibility: a script-path/condition-tree output (no owned spend key) cannot be spent through the
        // key-path statement flow. Those require the separate generic SpendWitness follow-up.
        if !output.is_condition_spendable {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "utxo_addresses",
                reason: format!(
                    "UTXO {} is a condition-tree output and is not key-path spendable",
                    address
                ),
            });
        }
        // Status and on-chain eligibility, evaluated together:
        // - Unspent outputs must be confirmed on-chain.
        // - LockedUnconfirmed outputs are accepted only when already bound to this lock (they are off-chain by
        //   construction, created earlier in this lock's transaction chain).
        // - Any other status, a LockedUnconfirmed under a different lock, or any other off-chain output is rejected.
        match output.status {
            OutputStatus::Unspent => {
                if !output.is_on_chain {
                    return Err(StealthOutputsApiError::InvalidParameter {
                        param: "utxo_addresses",
                        reason: format!("UTXO {} is Unspent but not yet on-chain", address),
                    });
                }
            },
            OutputStatus::LockedUnconfirmed => {
                if output.lock_id != Some(lock_id) {
                    return Err(StealthOutputsApiError::InvalidParameter {
                        param: "utxo_addresses",
                        reason: format!(
                            "UTXO {} is locked under a different lock (lock {:?}, requested lock {})",
                            address, output.lock_id, lock_id
                        ),
                    });
                }
                // LockedUnconfirmed is off-chain by construction; is_on_chain is not required here.
            },
            _ => {
                return Err(StealthOutputsApiError::InvalidParameter {
                    param: "utxo_addresses",
                    reason: format!(
                        "UTXO {} is not available to spend (status {:?}, lock {:?})",
                        address, output.status, output.lock_id
                    ),
                });
            },
        }
        Ok(())
    }

    fn lock_outputs_internal(
        &self,
        tx: &mut <TSpec::Store as WriteableWalletStore>::WriteTransaction<'_>,
        account_address: &ComponentAddress,
        resource_address: &ResourceAddress,
        amount: Amount,
        locked_by_id: WalletLockId,
        selection_algo: InputSelectionAlgorithm,
        max_inputs: usize,
    ) -> Result<Selection, StealthOutputsApiError> {
        if amount.is_negative() {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "amount",
                reason: "lock_outputs_internal: Amount cannot be negative".to_string(),
            });
        }
        // A selection's inputs are spent by one statement, so no selection may be given more than one statement can
        // carry — the per-transaction total is the sum of the budgets, not a bound on any one of them.
        if max_inputs == 0 || max_inputs > MAX_TRANSFER_INPUTS {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "max_inputs",
                reason: format!("Must be between 1 and {MAX_TRANSFER_INPUTS}, got {max_inputs}"),
            });
        }

        match selection_algo {
            InputSelectionAlgorithm::SmallestFirst => {
                let mut total_output_amount = Amount::zero();
                let mut outputs = Vec::new();
                let mut reached_input_limit = false;
                while total_output_amount < amount {
                    if outputs.len() >= max_inputs {
                        warn!(
                            target: LOG_TARGET,
                            "Reached the {} input limit for one transaction with {} of {} locked.",
                            max_inputs,
                            total_output_amount,
                            amount
                        );
                        reached_input_limit = true;
                        break;
                    }
                    let output = tx
                        .stealth_outputs_lock_smallest_amount(account_address, resource_address, locked_by_id)
                        .optional()?;
                    match output {
                        Some(output) => {
                            total_output_amount += Amount::from(output.value);
                            outputs.push(output);
                        },
                        None => {
                            debug!(
                                target: LOG_TARGET,
                                "No more outputs available to lock. Total locked amount: {}, required amount: {}",
                                total_output_amount,
                                amount
                            );
                            break;
                        },
                    }
                }

                Ok(Selection {
                    outputs: outputs.into_iter().map(|i| i.into_spend_data()).collect(),
                    total: total_output_amount,
                    reached_input_limit,
                })
            },
            InputSelectionAlgorithm::BranchAndBound => {
                let unspent =
                    tx.stealth_outputs_get_unspent_for_spending(account_address, resource_address, locked_by_id)?;

                let mut unspent = unspent
                    .into_iter()
                    .map(|o| (o.commitment, InputSpendData::from(o)))
                    .collect::<HashMap<_, _>>();

                let inputs = unspent
                    .values()
                    .map(|o| KeyedInput::new(o.commitment, o.value))
                    .collect::<Vec<_>>();

                // TODO: note that the behaviour of this implementation does not allow for partial selection, needed by
                // UtxoInputSelection::PreferConfidential. For now, we prevent running into this by only
                // using SmallestFirst.
                let Some(result) = input_selection::branch_and_bound::select(&inputs, amount, max_inputs) else {
                    // The search fails both when the outputs cannot reach the amount and when no `max_inputs` of them
                    // can, which are different answers for the caller: one is a balance, the other is fragmentation
                    // that consolidating would fix. The available total tells them apart.
                    let available = inputs.iter().map(|i| Amount::from(i.value())).sum::<Amount>();
                    return Ok(Selection {
                        outputs: Vec::new(),
                        total: Amount::zero(),
                        reached_input_limit: available >= amount,
                    });
                };

                let outputs = result
                    .selected_keys()
                    .iter()
                    .map(|selected| {
                        unspent
                            .remove(*selected)
                            .expect("selected an output not in the input key set")
                    })
                    .collect::<Vec<_>>();

                // Lock the selected outputs
                tx.stealth_outputs_lock_many(resource_address, result.selected_keys(), locked_by_id)?;

                Ok(Selection {
                    outputs,
                    total: result.total_value(),
                    reached_input_limit: false,
                })
            },
        }
    }

    /// Every stealth output held by a lock: both the inputs it locked for
    /// spending and the change outputs created alongside them. Callers filter
    /// by [`OutputStatus`] for the half they want.
    ///
    /// Returns `NotFound` when the lock holds no stealth outputs at all; use
    /// `.optional()` where that is not an error.
    pub fn get_locked_by_lock_id(
        &self,
        lock_id: WalletLockId,
    ) -> Result<Vec<StealthOutputModel>, StealthOutputsApiError> {
        let outputs = self
            .store
            .with_read_tx(|tx| tx.stealth_outputs_get_locked_by_lock_id(lock_id))?;
        Ok(outputs)
    }

    pub fn add_output(&self, output: &StealthOutputModel) -> Result<(), StealthOutputsApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.stealth_outputs_insert(output)?;
        tx.commit()?;
        Ok(())
    }

    /// Whether the wallet can auto-select a UTXO with this authorisation as a spend input. Only key-path UTXOs (whose
    /// one-time `spend_key` the wallet owns) are auto-spendable. Script-path (condition-tree) UTXOs are opaque from the
    /// committed root alone — satisfying a hidden leaf requires caller-provided conditions and witnesses — so they are
    /// not auto-selected and must be spent explicitly by the caller.
    pub fn is_spendable_auth(&self, auth: &SpendAuthorization) -> bool {
        auth.spend_key().is_some()
    }

    pub fn is_spendable_access_rule(&self, access_rule: &AccessRule) -> bool {
        match access_rule {
            AccessRule::AllowAll => true,
            AccessRule::DenyAll => false,
            // TODO: for now we'll mark complex access rules as unspendable (cant be selected as inputs), in future we
            // may have a good way to evaluate this
            AccessRule::Restricted(_) => false,
        }
    }

    fn resolve_output_masks_for_spending(
        &self,
        view_only_key_id: KeyId,
        inputs: &[InputSpendData],
    ) -> Result<Vec<UnblindedInputToSpend>, StealthOutputsApiError> {
        let mut inputs_with_masks = Vec::with_capacity(inputs.len());
        for input in inputs {
            // Derive the decryption key from the DHKE(sender's public nonce, encryption secret key);
            let nonce =
                input
                    .public_nonce
                    .try_from_byte_type()
                    .map_err(|e| StealthOutputsApiError::InvalidParameter {
                        param: "sender_public_nonce",
                        reason: format!("Sender public nonce bytes are not a canonical public key: {e}"),
                    })?;

            // Derive the view-only secret, of which the public key is used by senders to encrypt the value and mask.
            let decrypted = self.decrypt_value_and_mask(
                &input.encrypted_data,
                &input.commitment,
                view_only_key_id,
                &nonce,
                // We don't need to decrypt the memo to spend the output
                true,
            )?;

            inputs_with_masks.push(UnblindedInputToSpend {
                witness: StealthInputWitness::new(decrypted.mask_and_value),
            });
        }
        Ok(inputs_with_masks)
    }

    pub fn get_unspent_outputs_by_account(
        &self,
        account_address: &ComponentAddress,
        exclude_locked: bool,
    ) -> Result<Vec<StealthOutputInfo>, StealthOutputsApiError> {
        let balance = self
            .store
            .with_read_tx(|tx| tx.stealth_outputs_get_unspent_by_account(account_address, None, exclude_locked))?;
        Ok(balance)
    }

    pub fn get_unspent_balance_for_account_resource(
        &self,
        account_address: &ComponentAddress,
        resource_address: &ResourceAddress,
    ) -> Result<Amount, StealthOutputsApiError> {
        let outputs = self.store.with_read_tx(|tx| {
            tx.stealth_outputs_get_unspent_by_account(account_address, Some(resource_address), false)
        })?;
        Ok(outputs.into_iter().map(|o| Amount::from(o.value)).sum::<Amount>())
    }

    pub fn get_unspent_balance(
        &self,
        resource_address: &ResourceAddress,
    ) -> Result<StealthBalance, StealthOutputsApiError> {
        let balance = self
            .store
            .with_read_tx(|tx| tx.stealth_outputs_get_unspent_balance(resource_address))?;
        Ok(balance)
    }

    /// Counts the account's outputs that are unspent or locked by a pending transaction. A spend of
    /// any of them may still have to reach the wallet through the UTXO scanner.
    pub fn count_unspent_or_locked_outputs_for_account(
        &self,
        account_address: &ComponentAddress,
        resource_address: &ResourceAddress,
    ) -> Result<u64, StealthOutputsApiError> {
        let count = self.store.with_read_tx(|tx| {
            let mut count = 0;
            for status in [
                OutputStatus::Unspent,
                OutputStatus::LockedForSpend,
                OutputStatus::LockedUnconfirmed,
            ] {
                count += tx.stealth_outputs_count_by_status(account_address, resource_address, status)?;
            }
            Ok::<_, StealthOutputsApiError>(count)
        })?;
        Ok(count)
    }

    /// Inserts `utxo`, or updates the stored output's burnt and frozen flags and, while it is unspent, its status.
    /// Returns whether the stored output was inserted or changed.
    pub fn upsert_utxo(&self, utxo: &StealthOutputModel) -> Result<bool, StealthOutputsApiError> {
        let changed = self.store.with_write_tx(|tx| {
            // TODO(perf): consider a dedicated exists query
            let maybe_utxo = tx
                .stealth_outputs_get_by_commitment(&utxo.resource_address, &utxo.commitment)
                .optional()?;
            if let Some(prev_utxo) = maybe_utxo {
                let new_status = match prev_utxo.status {
                    OutputStatus::Unspent => Some(utxo.status),
                    // If not unspent, don't allow status to be changed.
                    // EDGE-CASE: scanning picks up a local UTXO that we know was spent
                    _ => None,
                };
                let changed = new_status.is_some_and(|status| status != prev_utxo.status) ||
                    prev_utxo.is_burnt != utxo.is_burnt ||
                    prev_utxo.is_frozen != utxo.is_frozen;
                let address = utxo.to_utxo_address();
                tx.stealth_outputs_update(&address, Some(utxo.is_burnt), new_status, Some(utxo.is_frozen))?;
                Ok::<_, WalletStorageError>(changed)
            } else {
                tx.stealth_outputs_insert(utxo)?;
                Ok(true)
            }
        })?;
        Ok(changed)
    }

    /// Sets the frozen flag of a stored output. Returns `None` when the wallet holds no output at `address`, and
    /// otherwise whether the flag changed.
    pub fn set_utxo_frozen(
        &self,
        address: &UtxoAddress,
        is_frozen: bool,
    ) -> Result<Option<bool>, StealthOutputsApiError> {
        let changed = self.store.with_write_tx(|tx| {
            let Some(prev_utxo) = tx
                .stealth_outputs_get_by_commitment(address.resource_address(), &address.id().into_commitment_bytes())
                .optional()?
            else {
                return Ok::<_, WalletStorageError>(None);
            };
            tx.stealth_outputs_update(address, None, None, Some(is_frozen))?;
            Ok(Some(prev_utxo.is_frozen != is_frozen))
        })?;
        Ok(changed)
    }

    pub fn update_utxo_status(
        &self,
        address: &UtxoAddress,
        is_burnt: Option<bool>,
        status: Option<OutputStatus>,
        is_frozen: Option<bool>,
    ) -> Result<(), StealthOutputsApiError> {
        self.store
            .with_write_tx(|tx| tx.stealth_outputs_update(address, is_burnt, status, is_frozen))?;
        Ok(())
    }

    pub fn utxo_exists(&self, address: &UtxoAddress) -> Result<bool, StealthOutputsApiError> {
        let exists = self
            .store
            .with_read_tx(|tx| {
                // TODO(perf): consider a dedicated exists query
                tx.stealth_outputs_get_by_commitment(address.resource_address(), &address.id().into_commitment_bytes())
                    .optional()
            })?
            .is_some();
        Ok(exists)
    }

    pub fn utxos_get_many(
        &self,
        resource_address: &ResourceAddress,
        account: Option<&ComponentAddress>,
        by_status: Option<OutputStatus>,
    ) -> Result<Vec<StealthOutputModel>, StealthOutputsApiError> {
        let outputs = self
            .store
            .with_read_tx(|tx| tx.stealth_outputs_get_many(resource_address, account, by_status))?;
        Ok(outputs)
    }

    fn all_used_view_only_keys(&self) -> Result<Vec<AccountAndViewKeys>, StealthOutputsApiError> {
        let mut keys = self
            .key_manager_api
            .get_all_derived_keys(KeyBranch::ViewOnlyKey)?
            .into_iter()
            .map(|view_key| {
                let account_key = self.key_manager_api.derive_account_key(
                    view_key
                        .key_id
                        .derived_index()
                        .expect("get_all_derived_keys returns only derived keys"),
                )?;
                Ok::<_, KeyManagerApiError>(AccountAndViewKeys {
                    account_public_key: account_key.to_public_key().to_byte_type(),
                    account_key: Some(account_key.into()),
                    view_only_key: view_key.into(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        // View-only accounts use an imported view key, which is not in the derived key branch above
        let accounts = self.store.with_read_tx(|tx| {
            let count = tx.accounts_count()?;
            tx.accounts_get_many(0, count as usize)
        })?;
        for account in accounts {
            if !matches!(account.view_only_key_id, KeyId::Imported { .. }) {
                continue;
            }
            let loaded = account
                .owner_key_id
                .map(|key_id| self.key_manager_api.get_key(key_id))
                .transpose()
                .and_then(|account_key| Ok((account_key, self.key_manager_api.get_key(account.view_only_key_id)?)));
            // One unreadable key must not stop outputs being checked for every other account
            let (account_key, view_only_key) = match loaded {
                Ok(keys) => keys,
                Err(e) => {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Skipping account {} when checking outputs: its keys could not be loaded: {}",
                        account.component_address,
                        e
                    );
                    continue;
                },
            };
            keys.push(AccountAndViewKeys {
                account_public_key: account.owner_public_key,
                account_key,
                view_only_key,
            });
        }
        Ok(keys)
    }

    #[allow(clippy::too_many_lines)]
    pub fn verify_and_update_outputs<'i, I: IntoIterator<Item = (UtxoAddress, &'i Utxo)>>(
        &self,
        outputs: I,
    ) -> Result<HashSet<(ComponentAddress, ResourceAddress)>, StealthOutputsApiError> {
        // The keys are only loaded once an output needs validating
        let mut all_used_view_only_keys = None;
        let network = self.config_api.get_network()?;

        let mut found_utxos_count = 0usize;
        let mut num_outputs = 0usize;
        let mut touched_balances = HashSet::new();
        for (addr, utxo) in outputs {
            num_outputs += 1;
            let commitment = addr.id().into_commitment_bytes();
            let resource_address = addr.resource_address();
            debug!(
                target: LOG_TARGET,
                "Validating UTXO for address: {}",
                addr,
            );

            // The keys are read through the store, so they are loaded outside the write transaction and the
            // output is checked again
            let (mut tx, exists) = loop {
                let mut tx = self.store.create_write_tx()?;
                let exists = tx
                    .stealth_outputs_get_by_commitment(resource_address, &commitment)
                    .optional()?
                    .is_some();
                if exists || utxo.output().is_none() || all_used_view_only_keys.is_some() {
                    break (tx, exists);
                }
                drop(tx);
                all_used_view_only_keys = Some(self.all_used_view_only_keys()?);
            };
            if exists {
                info!(
                    target: LOG_TARGET,
                    "Output already exists in the wallet. Updating. (commitment: {})",
                    commitment
                );
                if utxo.is_burnt() {
                    info!(
                        target: LOG_TARGET,
                        "🔥 Owned output is burnt with commitment: {}.",
                        commitment
                    );
                }
                if utxo.is_frozen() {
                    info!(
                        target: LOG_TARGET,
                        "❄️ Owned output is frozen with commitment: {}.",
                        commitment
                    );
                }

                // Update is_burnt and is_frozen status to false->true or true->false
                tx.stealth_outputs_update(&addr, Some(utxo.is_burnt()), None, Some(utxo.is_frozen()))?;
            } else {
                let is_frozen = utxo.is_frozen();
                let Some(output) = utxo.output() else {
                    debug!(
                        target: LOG_TARGET,
                        "Unknown Utxo output is burnt for commitment: {}. Skipping.",
                        commitment
                    );
                    continue;
                };

                // Output does not exist. Validate it and add it to the store
                match self.validate_utxo(
                    all_used_view_only_keys
                        .as_deref()
                        .expect("keys are loaded for an output that is not stored"),
                    network,
                    *resource_address,
                    commitment,
                    output,
                    is_frozen,
                ) {
                    Ok(Some(output)) => {
                        found_utxos_count += 1;
                        touched_balances.insert((output.owner_account, output.resource_address));
                        tx.stealth_outputs_insert(&output)?;
                    },
                    Ok(None) => {
                        debug!(
                            target: LOG_TARGET,
                            "🚮 wallet could not extract the value and mask for this output. Assuming it is not owned. (commitment: {})",
                            commitment
                        );
                    },
                    Err(e) => {
                        warn!(
                            target: LOG_TARGET,
                            "Output validation failed. Skipping. (commitment: {}, error: {})",
                            commitment,
                            e
                        );
                    },
                }
            }
            tx.commit()?;
        }

        if num_outputs > 0 {
            info!(
                target: LOG_TARGET,
                "✅️ Found {}/{} stealth outputs owned by this wallet.",
                found_utxos_count,
                num_outputs,
            );
        }

        Ok(touched_balances)
    }

    #[allow(clippy::too_many_lines)]
    pub fn validate_utxo(
        &self,
        all_used_account_view_only_keys: &[AccountAndViewKeys],
        network: Network,
        resource_address: ResourceAddress,
        commitment: PedersenCommitmentBytes,
        output: &UtxoOutput,
        is_frozen: bool,
    ) -> Result<Option<StealthOutputModel>, StealthOutputsApiError> {
        // Validate the commitment is well-formed.
        let _output_commitment: PedersenCommitment =
            commitment
                .try_from_byte_type()
                .map_err(|e| StealthOutputsApiError::InvalidParameter {
                    param: "commitment",
                    reason: format!("Invalid output commitment bytes: {}", e),
                })?;

        let output_stealth_public_nonce =
            output
                .output
                .public_nonce
                .try_from_byte_type()
                .map_err(|e| StealthOutputsApiError::InvalidParameter {
                    param: "stealth_public_nonce",
                    reason: format!("Failed to parse stealth public nonce: {}", e),
                })?;

        debug!(
            target: LOG_TARGET,
            "Validating output using {} key(s) for resource address: {}, commitment: {}, public nonce: {}",
            all_used_account_view_only_keys.len(),
            resource_address,
            commitment,
            output_stealth_public_nonce,
        );

        let mut invalid_output = None;
        for keys in all_used_account_view_only_keys {
            trace!(
                target: LOG_TARGET,
                "Attempting to unblind output with view key {}",
                keys.view_only_key.key_id,
            );
            let unblinded_result = self.crypto_api.decrypt_utxo_data(
                &output.output.encrypted_data,
                &commitment,
                &keys.view_only_key.secret,
                &output_stealth_public_nonce,
                false,
            );

            let (value, memo, status) = match unblinded_result {
                Ok(decrypted) => {
                    let account_public_key = keys.account_public_key.try_from_byte_type().map_err(|e| {
                        StealthOutputsApiError::InvalidParameter {
                            param: "account_public_key",
                            reason: format!("Invalid account public key bytes: {}", e),
                        }
                    })?;
                    let stealth_address = self
                        .crypto_api
                        .derive_stealth_owner_public_key_from_view_key(
                            network,
                            &account_public_key,
                            &keys.view_only_key.secret,
                            &output_stealth_public_nonce,
                        )
                        .to_byte_type();
                    // A key-path output is owned only if its spend_key is our derived stealth address. A
                    // condition-tree output (no spend_key) is recognised as viewable but not key-spendable here.
                    if keys.account_key.is_none() && output.auth.condition_root().is_some() {
                        // A condition tree lets whoever satisfies it spend the output, which a view key cannot rule
                        // out, so for a view-only account it never counts as balance
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ Output with a condition tree for view-only key {} cannot be confirmed as owned and will be stored as invalid.",
                            keys.view_only_key.key_id,
                        );
                        (decrypted.value(), decrypted.memo, OutputStatus::Invalid)
                    } else if output.auth.spend_key().is_none_or(|pk| *pk == stealth_address) {
                        if keys.account_key.is_none() {
                            info!(
                                target: LOG_TARGET,
                                "Output can only be viewed, not spent, as there is no owner key for view key {}",
                                keys.view_only_key.key_id,
                            );
                        }
                        (decrypted.value(), decrypted.memo, OutputStatus::Unspent)
                    } else {
                        warn!(
                            target: LOG_TARGET,
                            "⚠️ Output owner public key does not match the expected stealth address. (expected: {}, actual: {}). Utxo cannot be spent by this account and will be stored as invalid.",
                            stealth_address,
                            output.auth.spend_key().as_ref().display()
                        );
                        (decrypted.value(), decrypted.memo, OutputStatus::Invalid)
                    }
                },
                Err(e) => {
                    debug!(
                        target: LOG_TARGET,
                        "Failed to unblind output for key {}. (commitment: {}, error: {})",
                        keys.view_only_key.key_id,
                        commitment,
                        e
                    );
                    continue;
                },
            };

            let owner_account =
                derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &keys.account_public_key);
            info!(
                target: LOG_TARGET,
                "🟢 Unblinded output for account {}. (commitment: {}, value: {})",
                owner_account,
                commitment,
                value,
            );

            let model = StealthOutputModel {
                owner_account,
                // Note that this is not validated and depends on the caller ensuring the resource address belongs to
                // the stealth output.
                resource_address,
                commitment,
                value,
                sender_public_nonce: output_stealth_public_nonce.to_byte_type(),
                view_only_key_id: keys.view_only_key.key_id,
                owner_key_id: keys.account_key.as_ref().map(|k| k.key_id),
                encrypted_data: output.output.encrypted_data.clone(),
                tag_byte: output.tag,
                memo,
                auth: output.auth.clone(),
                minimum_value_promise: output.output.minimum_value_promise,
                status,
                is_burnt: false,
                is_frozen,
                is_on_chain: true,
                is_condition_spendable: self.is_spendable_auth(&output.auth),
                lock_id: None,
            };
            // Another key may still own the output, so an invalid result is only returned if none does
            if matches!(model.status, OutputStatus::Invalid) {
                invalid_output.get_or_insert(model);
                continue;
            }
            return Ok(Some(model));
        }

        Ok(invalid_output)
    }

    pub fn create_output_witness(
        &self,
        destination: &RistrettoOotleAddress,
        amount: u64,
        resource_address: &ResourceAddress,
        resource_view_key: Option<RistrettoPublicKey>,
        memo: Option<&Memo>,
        pay_to: PayTo,
    ) -> Result<StealthOutputWitness, StealthOutputsApiError> {
        let mask = self.key_manager_api.next_key(KeyBranch::StealthMask)?;

        let (nonce_secret, public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let encrypted_data = self.crypto_api.encrypt_value_and_mask(
            amount,
            &mask.key,
            destination.view_only_key(),
            &nonce_secret,
            memo,
        )?;

        let auth = pay_to_output_authorization(&pay_to, || {
            // Create stealth address that the destination can use at spend time
            self.crypto_api
                .derive_stealth_owner_public_key(
                    destination.network(),
                    destination.account_key(),
                    destination.view_only_key(),
                    &nonce_secret,
                )
                .to_byte_type()
        })?;

        let witness = OutputWitness {
            amount,
            mask: mask.key,
            sender_public_nonce: public_nonce,
            encrypted_data,
            minimum_value_promise: 0,
            resource_view_key,
        };

        let derived_tag = self.crypto_api.derive_stealth_output_tag(
            destination.network(),
            &nonce_secret,
            destination.view_only_key(),
            resource_address,
        );

        Ok(StealthOutputWitness {
            witness,
            auth,
            tag: derived_tag,
        })
    }

    pub fn generate_transfer_statement<'i, I>(
        &self,
        params: TransferStatementParams<'i, I>,
    ) -> Result<StealthTransferStatement, StealthOutputsApiError>
    where
        I: IntoIterator<Item = StealthOutputToCreate<'i>>,
    {
        let unblinded_inputs = self.resolve_output_masks_for_spending(params.view_only_key_id, params.inputs)?;
        let outputs = params
            .outputs
            .into_iter()
            .map(|output| {
                self.create_output_witness(
                    &output.owner_address,
                    output.amount,
                    params.resource_address,
                    params.resource_view_key.clone(),
                    output.memo,
                    output.pay_to,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let total_input_amount =
            unblinded_inputs.iter().map(|i| Amount::from(i.value())).sum::<Amount>() + params.input_revealed_amount;
        let total_output_amount = outputs.iter().map(|o| Amount::from(o.witness.amount)).sum::<Amount>() +
            params.revealed_output.map_or(Amount::ZERO, |r| r.amount);
        if total_input_amount != total_output_amount {
            return Err(StealthOutputsApiError::InvalidParameter {
                param: "inputs/outputs",
                reason: format!(
                    "Input and output amounts do not balance. Input: {}, Output: {}",
                    total_input_amount, total_output_amount
                ),
            });
        }

        let statement = self.crypto_api.generate_transfer_statement(
            unblinded_inputs.into_iter().map(|i| i.witness),
            params.input_revealed_amount,
            outputs.iter(),
            params.revealed_output,
        )?;
        Ok(statement)
    }

    pub fn decrypt_value_and_mask(
        &self,
        output_encrypted_value: &EncryptedData,
        output_commitment: &PedersenCommitmentBytes,
        claim_secret_key_id: KeyId,
        reciprocal_public_key: &RistrettoPublicKey,
        skip_memo: bool,
    ) -> Result<DecryptedData, StealthOutputsApiError> {
        let key = self.key_manager_api.get_key(claim_secret_key_id)?;
        let decrypted = self.crypto_api.decrypt_utxo_data(
            output_encrypted_value,
            output_commitment,
            &key.secret,
            reciprocal_public_key,
            skip_memo,
        )?;
        Ok(decrypted)
    }

    pub fn encrypt_value_and_mask(
        &self,
        amount: u64,
        public_key: &RistrettoPublicKey,
        memo: Option<&Memo>,
    ) -> Result<(RistrettoPublicKey, EncryptedData), StealthOutputsApiError> {
        let nonce_secret = self.key_manager_api.create_throwaway_nonce();
        let public_nonce = RistrettoPublicKey::from_secret_key(&nonce_secret);
        let mask = self.key_manager_api.next_key(KeyBranch::StealthMask)?;
        let data = self
            .crypto_api
            .encrypt_value_and_mask(amount, &mask.key, public_key, &nonce_secret, memo)?;
        Ok((public_nonce, data))
    }
}

pub struct TransferStatementParams<'a, I> {
    pub view_only_key_id: KeyId,
    pub resource_address: &'a ResourceAddress,
    pub resource_view_key: Option<RistrettoPublicKey>,
    pub inputs: &'a [InputSpendData],
    pub input_revealed_amount: Amount,
    pub outputs: I,
    /// The revealed output and the key authorised to take it. The receiver must be a signer of the transaction that
    /// carries this statement, or the engine will not create the bucket.
    pub revealed_output: Option<RevealedOutput>,
}

#[derive(Debug, thiserror::Error)]
pub enum StealthOutputsApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Crypto error: {0}")]
    Crypto(#[from] StealthCryptoApiError),
    #[error("Wallet crypto error: {0}")]
    WalletCrypto(#[from] tari_ootle_wallet_crypto::WalletCryptoError),
    #[error(
        "Reached the limit of {max} inputs for one transaction with {locked} of {required} covered: the balance is \
         spread across more stealth outputs than one transaction can spend, and has to be consolidated first"
    )]
    InputLimitReached {
        max: usize,
        locked: Amount,
        required: Amount,
    },
    #[error("Insufficient funds")]
    InsufficientFunds,
    #[error("Input selection error: {details}")]
    InputSelectionFailed { details: String },
    #[error("Key manager error: {0}")]
    KeyManager(#[from] KeyManagerApiError),
    #[error("Accounts API error: {0}")]
    Accounts(#[from] AccountsApiError),
    #[error("Invalid parameter `{param}`: {reason}")]
    InvalidParameter { param: &'static str, reason: String },
    #[error("Config error: {0}")]
    ConfigApiError(#[from] ConfigApiError),
}

impl IsNotFoundError for StealthOutputsApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
