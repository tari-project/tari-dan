//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{cmp, time::Duration};

use log::*;
use ootle_byte_type::{ConvertFromByteType, FromByteType, ToByteType};
use ootle_network::Network;
use tari_crypto::ristretto::RistrettoPublicKey;
use tari_engine_types::{
    limits::FREE_COMPUTE_GRACE_POINTS,
    stealth::{persisted_utxo_bytes, transfer_native_points_for_shape},
    substate::SubstateId,
};
use tari_ootle_address::{OotleAddress, RistrettoOotleAddress};
use tari_ootle_common_types::{InputDeclaration, displayable::Displayable, optional::Optional};
use tari_ootle_transaction::{Transaction, TransactionBuilder, UnsignedTransaction, args};
use tari_ootle_wallet_crypto::{memo::Memo, pay_to::PayTo};
use tari_template_lib::{
    models::Account as BuiltinAccount,
    types::{
        Amount,
        ComponentAddress,
        ResourceAddress,
        UtxoAddress,
        constants::TARI_TOKEN,
        stealth::{RevealedOutput, StealthTransferStatement, StealthUnspentOutput},
    },
};
use tokio::{sync::Semaphore, task::block_in_place};

use super::{
    BadgeUsage,
    TransferFeeParams,
    error::StealthTransferApiError,
    params::StealthTransferParams,
    types::{InputsToSpend, StaticallyPricedShape, StealthOutputToCreate, StealthTransferOutput},
};
use crate::{
    WalletSdkSpec,
    apis::{
        accounts::{AccountsApi, derive_account_address_from_public_key},
        confidential_transfer::UtxoInputSelection,
        config::ConfigApi,
        key_manager::KeyManagerApi,
        locks::LocksApi,
        stealth_outputs::{
            FEE_INTENT_INPUT_RESERVE,
            MAX_TRANSFER_INPUTS,
            StealthOutputsApi,
            StealthOutputsApiError,
            TransferStatementParams,
        },
        substate::{SubstatesApi, ValidatorScanResult},
    },
    models::{
        AccountWithAddress,
        KeyBranch,
        KeyId,
        OutputStatus,
        StealthOutputModel,
        StealthUtxoSpendKeyId,
        WalletLockDropGuard,
        WalletLockId,
        WalletPublicKey,
    },
};

const LOG_TARGET: &str = "tari::ootle::wallet_sdk::apis::stealth_transfers";

/// The share of [`FREE_COMPUTE_GRACE_POINTS`] a merged statement may occupy. That credit is the entire
/// allowance funding the fee intent until `pay_fee` settles, and the engine traps mid-execution once
/// native verification exceeds it, so the merge only fires with headroom against the engine's own price.
const MERGED_STATEMENT_CREDIT_UTILISATION_PERCENT: u64 = 80;

/// Whether a merged statement of this shape verifies within the credit funding the fee intent.
///
/// A merged statement sources the fee from its own revealed remainder, so all of its verification
/// precedes `pay_fee` and is funded by the credit alone rather than by the payment. Outputs dominate
/// the price, which is what bounds how many recipients one statement may serve.
fn merged_statement_fits_credit(num_inputs: usize, num_outputs: usize, has_view_key: bool) -> bool {
    let budget = FREE_COMPUTE_GRACE_POINTS / 100 * MERGED_STATEMENT_CREDIT_UTILISATION_PERCENT;
    transfer_native_points_for_shape(num_inputs, num_outputs, has_view_key) <= budget
}

pub struct StealthTransferApi<'a, TSpec: WalletSdkSpec> {
    accounts_api: AccountsApi<'a, TSpec>,
    outputs_api: StealthOutputsApi<'a, TSpec>,
    locks_api: LocksApi<'a, TSpec::Store>,
    substate_api: SubstatesApi<'a, TSpec::Store, TSpec::NetworkInterface>,
    key_manager_api: KeyManagerApi<'a, TSpec>,
    config_api: ConfigApi<'a, TSpec::Store>,
    semaphore: Semaphore,
}

impl<'a, TSpec: WalletSdkSpec> StealthTransferApi<'a, TSpec> {
    pub fn new(
        accounts_api: AccountsApi<'a, TSpec>,
        outputs_api: StealthOutputsApi<'a, TSpec>,
        locks_api: LocksApi<'a, TSpec::Store>,
        substate_api: SubstatesApi<'a, TSpec::Store, TSpec::NetworkInterface>,
        key_manager_api: KeyManagerApi<'a, TSpec>,
        config_api: ConfigApi<'a, TSpec::Store>,
    ) -> Self {
        Self {
            accounts_api,
            outputs_api,
            locks_api,
            substate_api,
            key_manager_api,
            config_api,
            semaphore: Semaphore::new(1),
        }
    }

    /// Locks the inputs a transfer statement will spend, leaving [`FEE_INTENT_INPUT_RESERVE`] of the transaction's
    /// input budget for a statement that sources the fee.
    ///
    /// The reserve is held back unconditionally, including on the merged-statement path that may not need a fee
    /// statement at all: whether the merge fits the fee-intent credit is only known once the realised input count is,
    /// which is after selection, and a selection that had already claimed the whole budget could not then fall back.
    pub fn lock_inputs_for_transfer(
        &self,
        lock_id: WalletLockId,
        owner_account_component_address: &ComponentAddress,
        resource_address: ResourceAddress,
        spend_amount: Amount,
        input_selection: UtxoInputSelection,
    ) -> Result<InputsToSpend, StealthTransferApiError> {
        self.lock_inputs_within_budget(
            lock_id,
            owner_account_component_address,
            resource_address,
            spend_amount,
            input_selection,
            MAX_TRANSFER_INPUTS,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn lock_inputs_within_budget(
        &self,
        lock_id: WalletLockId,
        owner_account_component_address: &ComponentAddress,
        resource_address: ResourceAddress,
        spend_amount: Amount,
        input_selection: UtxoInputSelection,
        max_inputs: usize,
    ) -> Result<InputsToSpend, StealthTransferApiError> {
        if !spend_amount.is_positive() {
            return Err(StealthTransferApiError::InvalidParameter {
                param: "spend_amount",
                reason: "Spend amount must be positive".to_string(),
            });
        }

        let maybe_src_vault = self
            .accounts_api
            .get_vault_by_resource(owner_account_component_address, &resource_address)
            .optional()?;

        let available_revealed_funds = maybe_src_vault
            .as_ref()
            .map(|v| v.available_revealed_balance())
            .unwrap_or_else(Amount::zero);

        match input_selection {
            UtxoInputSelection::ConfidentialOnly => {
                let (inputs, total_locked) = self.outputs_api.lock_outputs_for_at_least_amount(
                    owner_account_component_address,
                    &resource_address,
                    lock_id,
                    spend_amount,
                    max_inputs,
                )?;

                info!(
                    target: LOG_TARGET,
                    "ConfidentialOnly: Locked {} confidential inputs for transfer worth {}",
                    inputs.len(),
                    total_locked
                );

                Ok(InputsToSpend {
                    inputs,
                    revealed: Amount::zero(),
                })
            },
            UtxoInputSelection::RevealedOnly => {
                if available_revealed_funds < spend_amount {
                    return Err(StealthTransferApiError::InsufficientFunds {
                        details: "RevealedOnly: Not enough revealed funds to spend.".to_string(),
                        available: available_revealed_funds,
                        required: spend_amount,
                    });
                }

                let src_vault =
                    maybe_src_vault
                        .as_ref()
                        .ok_or_else(|| StealthTransferApiError::InsufficientRevealedFunds {
                            details: format!(
                                "No vault found for resource {} in account {}",
                                resource_address, owner_account_component_address
                            ),
                        })?;

                self.locks_api
                    .lock_funds_in_vault(lock_id, &src_vault.id, spend_amount)?;

                info!(
                    target: LOG_TARGET,
                    "RevealedOnly: Spending {} revealed balance for transfer from {}",
                    spend_amount,
                    maybe_src_vault.as_ref().map(|v| v.id).display()
                );

                Ok(InputsToSpend {
                    inputs: vec![],
                    revealed: spend_amount,
                })
            },
            UtxoInputSelection::PreferRevealed => {
                let revealed_to_spend = cmp::min(available_revealed_funds, spend_amount);
                let utxo_amount_to_spend = spend_amount - revealed_to_spend;
                if let Some(ref src_vault) = maybe_src_vault &&
                    utxo_amount_to_spend.is_zero()
                {
                    info!(
                        target: LOG_TARGET,
                        "PreferRevealed: Spending {} revealed balance (available: {}) for transfer from {}",
                        revealed_to_spend,
                        available_revealed_funds,
                        src_vault.id
                    );

                    self.locks_api
                        .lock_funds_in_vault(lock_id, &src_vault.id, revealed_to_spend)?;

                    return Ok(InputsToSpend {
                        inputs: vec![],
                        revealed: revealed_to_spend,
                    });
                }

                if maybe_src_vault.is_none() && revealed_to_spend.is_positive() {
                    // No vault containing revealed funds was found
                    return Err(StealthTransferApiError::InsufficientRevealedFunds {
                        details: format!(
                            "PreferRevealed: No vault found for resource {} in account {}. Need to spend {} revealed \
                             funds",
                            resource_address, owner_account_component_address, revealed_to_spend
                        ),
                    });
                }

                let (inputs, _) = self.outputs_api.lock_outputs_for_at_least_amount(
                    owner_account_component_address,
                    &resource_address,
                    lock_id,
                    utxo_amount_to_spend,
                    max_inputs,
                )?;

                let total_confidential_spent = inputs.iter().map(|i| Amount::from(i.value)).sum::<Amount>();

                if let Some(ref src_vault) = maybe_src_vault {
                    self.locks_api
                        .lock_funds_in_vault(lock_id, &src_vault.id, revealed_to_spend)?;
                }

                info!(
                    target: LOG_TARGET,
                    "PreferRevealed: Locked {} confidential inputs (target: {}, spent: {}) and {} revealed for amount {} from {}",
                    inputs.len(),
                    utxo_amount_to_spend,
                    total_confidential_spent,
                    revealed_to_spend,
                    spend_amount,
                    maybe_src_vault.as_ref().map(|v| v.id).display()
                );

                Ok(InputsToSpend {
                    inputs,
                    revealed: revealed_to_spend,
                })
            },
            UtxoInputSelection::PreferConfidential => {
                let (inputs, blinded_amount_locked) = self.outputs_api.lock_outputs_until_partial_amount(
                    owner_account_component_address,
                    &resource_address,
                    spend_amount,
                    lock_id,
                    max_inputs,
                )?;

                let revealed_to_spend = spend_amount.saturating_sub(blinded_amount_locked);

                if available_revealed_funds < revealed_to_spend {
                    return Err(StealthTransferApiError::InsufficientFunds {
                        details: "PreferConfidential: Not enough revealed funds to spend.".to_string(),
                        available: available_revealed_funds,
                        required: revealed_to_spend,
                    });
                }

                if revealed_to_spend.is_positive() {
                    let vault =
                        maybe_src_vault
                            .as_ref()
                            .ok_or_else(|| StealthTransferApiError::InsufficientRevealedFunds {
                                details: format!(
                                    "PreferConfidential: No vault found for resource {} in account {}. Need to spend \
                                     {} revealed funds",
                                    resource_address, owner_account_component_address, revealed_to_spend
                                ),
                            })?;

                    self.locks_api
                        .lock_funds_in_vault(lock_id, &vault.id, revealed_to_spend)?;
                }

                Ok(InputsToSpend {
                    inputs,
                    revealed: revealed_to_spend,
                })
            },
        }
    }

    fn lock_fee_inputs<A: Into<Amount>>(
        &self,
        lock_id: WalletLockId,
        owner_account_address: &ComponentAddress,
        max_fee: A,
        fee_params: &TransferFeeParams,
    ) -> Result<InputsToSpend, StealthTransferApiError> {
        let (resource, amount) = match fee_params.pay_fee_with_swap.as_ref() {
            Some(swap) => (swap.input_resource, swap.input_amount),
            None => (TARI_TOKEN, max_fee.into()),
        };
        self.lock_inputs_within_budget(
            lock_id,
            owner_account_address,
            resource,
            amount,
            fee_params.input_selection,
            FEE_INTENT_INPUT_RESERVE,
        )
    }

    /// Locks fee inputs and builds the statement that sources the fee, registering its change output as
    /// unconfirmed so that a same-resource transfer can spend it. Used whenever the fee cannot be
    /// sourced by the transfer statement itself: a differing fee resource, a swap, or a merged statement
    /// that would not fit the fee-intent credit.
    fn build_fee_statement(
        &self,
        lock_id: WalletLockId,
        owner_account: &AccountWithAddress,
        params: &StealthTransferParams,
        owner_address: &RistrettoOotleAddress,
        account_key_id: KeyId,
    ) -> Result<(InputsToSpend, WalletPublicKey, StealthTransferStatement), StealthTransferApiError> {
        let fee_inputs_to_spend = self.lock_fee_inputs(
            lock_id,
            owner_account.component_address(),
            params.max_fee,
            &params.fee_params,
        )?;

        debug!(
            target: LOG_TARGET,
            "🔒️ Locked {} fee inputs for fee spending worth {} (max fee {})",
            fee_inputs_to_spend.inputs.len(),
            fee_inputs_to_spend.total_stealth_input_amount(),
            params.max_fee,
        );

        // When paying with a swap, the amount to spend is the swap input_amount (non-TARI token),
        // not max_fee (which is in microtari and has a different exchange rate).
        let fee_amount_to_spend = match params.fee_params.pay_fee_with_swap.as_ref() {
            Some(swap) => swap.input_amount,
            None => params.max_fee.into(),
        };

        let fee_stealth_change_amt = fee_inputs_to_spend
            .total_stealth_input_amount()
            .saturating_sub(fee_amount_to_spend)
            .to_u64_checked()
            .ok_or_else(|| {
                StealthTransferApiError::InvariantViolation {
                    // Technically, you could create multiple outputs, but for simplicity and because this is
                    // extremely unlikely to be needed, we only create one here
                    details: "Fee change amount exceeds u64".to_string(),
                }
            })?;

        // Generate fee change outputs if required
        let fee_change_output = Some(fee_stealth_change_amt)
            .filter(|amt| *amt > 0)
            .map(|amt| StealthOutputToCreate {
                owner_address: owner_address.clone(),
                amount: amt,
                memo: None,
                pay_to: PayTo::StealthPublicKey,
            });

        // Figure out which signing key to use - if there are revealed funds, we need to use an account
        // withdraw auth signature, otherwise we can use a "throw-away" and private nonce.
        let must_sign_with_account_key = fee_inputs_to_spend.revealed.is_positive();
        let signing_key_id = if must_sign_with_account_key {
            account_key_id
        } else {
            self.key_manager_api.next_derived_key_id(KeyBranch::Nonce)?.into()
        };
        let fee_signer = self.key_manager_api.get_public_key(signing_key_id)?;

        let fee_resource = params
            .fee_params
            .pay_fee_with_swap
            .as_ref()
            .map(|swap| swap.input_resource)
            .unwrap_or(TARI_TOKEN);
        // Generate fee transfer statement
        let fee_transfer_statement = self.outputs_api.generate_transfer_statement(TransferStatementParams {
            view_only_key_id: owner_account.view_only_key_id(),
            resource_address: &fee_resource,
            resource_view_key: None,
            inputs: &fee_inputs_to_spend.inputs,
            input_revealed_amount: fee_inputs_to_spend.revealed,
            outputs: fee_change_output,
            // The fee signer is the only key that authorises this statement's revealed funds.
            revealed_output: (!fee_amount_to_spend.is_zero())
                .then(|| RevealedOutput::new(fee_amount_to_spend, fee_signer.public_key().to_byte_type())),
        })?;

        // Add the unconfirmed fee change output to the wallet store
        if let Some(output) = fee_transfer_statement.outputs_statement.outputs.first() {
            debug!(
                target: LOG_TARGET,
                "Adding FEE unconfirmed output with commitment {} for amount {} to account {}",
                output.output.commitment,
                fee_stealth_change_amt,
                owner_account.component_address()
            );
            self.add_unconfirmed_output_from_statement(
                lock_id,
                owner_account,
                fee_resource,
                output,
                fee_stealth_change_amt,
                None,
            )?;
        }

        Ok((fee_inputs_to_spend, fee_signer, fee_transfer_statement))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn transfer(
        &self,
        owner_account: AccountWithAddress,
        params: StealthTransferParams,
    ) -> Result<(WalletLockDropGuard<'a, TSpec::Store>, StealthTransferOutput), StealthTransferApiError> {
        let network = self.config_api.get_network()?;
        params.validate(network)?;

        let Some(account_key_id) = owner_account.owner_key_id() else {
            return Err(StealthTransferApiError::InvalidParameter {
                param: "owner_account",
                reason: format!(
                    "Account {} is view only and does not have the required secrets for transfers",
                    owner_account.component_address()
                ),
            });
        };

        // Determine Transaction Inputs
        let mut substate_inputs = Vec::new();
        let owner_address: RistrettoOotleAddress =
            owner_account
                .address()
                .try_from_byte_type()
                .map_err(|e| StealthTransferApiError::InvalidParameter {
                    param: "owner_account",
                    reason: format!("Non-canonical owner account address: {e}"),
                })?;

        // A transfer reads the resource without altering it.
        substate_inputs.push(InputDeclaration::read(params.resource_address));

        for output in &params.outputs {
            // No revealed outputs, no need to use the account
            if !output.revealed_amount.is_positive() {
                continue;
            }
            self.determine_destination_account_inputs(&output.address, &params.resource_address, &mut substate_inputs)
                .await?;
        }

        // We need to fetch the resource substate to check if there is a view key present.
        let resource = self.substate_api.fetch_resource(params.resource_address).await?;

        // Resolve the swap pool's dependent substates (e.g. its vaults) before entering the critical section
        let swap_pool_deps = if let Some(swap) = params.fee_params.pay_fee_with_swap.as_ref() {
            self.substate_api
                .locate_dependent_substates(&[swap.pool_address.into()], true)
                .await?
        } else {
            Default::default()
        };

        // Generate outputs
        let resource_view_key = resource
            .view_key()
            .map(RistrettoPublicKey::convert_from_byte_type)
            .transpose()
            .map_err(|e| StealthTransferApiError::InvalidParameter {
                param: "resource_view_key",
                reason: format!("Invalid resource view key: {e}"),
            })?;
        let resource_has_view_key = resource_view_key.is_some();

        // Critical section - TODO: use a DB transaction
        let _permit = self.semaphore.acquire().await.expect("semaphore is never closed");

        block_in_place(|| {
            // Create a lock with a timeout, the lock timeout will be removed if the lock is assigned a transaction
            let lock = self.locks_api.create_lock_with_timeout(Duration::from_secs(5 * 60))?;

            // A transfer statement whose revealed remainder covers the fee replaces the separate fee
            // statement, doing in one statement what otherwise takes two. It requires the fee and the
            // transfer resource to coincide, and the merged statement's whole verification then runs in
            // the fee intent ahead of `pay_fee`, funded by FREE_COMPUTE_GRACE_POINTS alone.
            let merge_candidate =
                params.fee_params.pay_fee_with_swap.is_none() && params.resource_address == TARI_TOKEN;

            // On the separate-statement path the fee half must run before transfer input selection, so
            // that its change output reaches the store in time for the transfer to be able to spend it.
            // Merged, there is no fee half and the single selection covers the fee as well.
            let mut fee_half = if merge_candidate {
                None
            } else {
                Some(self.build_fee_statement(lock.id(), &owner_account, &params, &owner_address, account_key_id)?)
            };

            // The fee the transfer statement itself reveals. Non-zero only while the merge holds: on
            // fallback the fee reverts to a statement of its own and this drops back to zero, so the
            // transfer statement reveals only what the recipients are owed and the surplus locked against
            // the fee becomes change.
            let mut merged_fee_amount = if merge_candidate {
                Amount::from(params.max_fee)
            } else {
                Amount::zero()
            };

            let fee_resource = params
                .fee_params
                .pay_fee_with_swap
                .as_ref()
                .map(|swap| swap.input_resource)
                .unwrap_or(TARI_TOKEN);

            let inputs_to_spend = self.lock_inputs_for_transfer(
                lock.id(),
                owner_account.account().component_address(),
                params.resource_address,
                params.total_output_amount_with_fee(merged_fee_amount)?,
                params.input_selection,
            )?;

            // Output count drives the price and is known up front, but the realised input count is not,
            // so the credit check waits for selection. Falling back is not a failure: past the credit the
            // separate-statement shape is genuinely cheaper, since only the fee statement's inputs are
            // funded by the credit and the transfer's move to the payment-funded main intent.
            if merge_candidate {
                let num_blinded_outputs = params.outputs.iter().filter(|o| o.blinded_amount > 0).count();
                let has_change =
                    inputs_to_spend.total_amount() > params.total_output_amount_with_fee(merged_fee_amount)?;
                let num_outputs = num_blinded_outputs + usize::from(has_change);
                if !merged_statement_fits_credit(inputs_to_spend.inputs.len(), num_outputs, resource_view_key.is_some())
                {
                    debug!(
                        target: LOG_TARGET,
                        "Merged fee/transfer statement ({} inputs, {} outputs) exceeds the fee-intent credit; \
                         sourcing the fee from a statement of its own",
                        inputs_to_spend.inputs.len(),
                        num_outputs,
                    );
                    // The transfer inputs already cover the fee, so this locks more of the account than is
                    // strictly needed. The surplus comes back as transfer change.
                    fee_half = Some(self.build_fee_statement(
                        lock.id(),
                        &owner_account,
                        &params,
                        &owner_address,
                        account_key_id,
                    )?);
                    // The fee is now sourced separately, so the transfer statement must stop revealing it.
                    merged_fee_amount = Amount::zero();
                }
            }

            let is_merged = fee_half.is_none();

            let (fee_inputs_to_spend, fee_signer, fee_transfer_statement) = match fee_half {
                Some((inputs, signer, statement)) => (inputs, signer, Some(statement)),
                None => {
                    // Merged: the transfer statement is itself the fee source, so its signer is the only
                    // signer the transaction has.
                    let signing_key_id = if !params.badge_usage.is_none() || inputs_to_spend.revealed.is_positive() {
                        account_key_id
                    } else {
                        self.key_manager_api.next_derived_key_id(KeyBranch::Nonce)?.into()
                    };
                    (
                        InputsToSpend::empty(),
                        self.key_manager_api.get_public_key(signing_key_id)?,
                        None,
                    )
                },
            };

            // Signing key for main transfer intent. A merged transaction has no main intent to sign for.
            let main_intent_signer = if is_merged {
                None
            } else {
                let must_sign_with_account_key =
                    !params.badge_usage.is_none() || inputs_to_spend.revealed.is_positive();
                let signing_key_id = if must_sign_with_account_key {
                    Some(account_key_id)
                } else {
                    None
                };

                // No need to add another signature if the fee signer is the same as the main signer
                signing_key_id
                    .filter(|key_id| *key_id != fee_signer.key_id())
                    .map(|key_id| self.key_manager_api.get_public_key(key_id))
                    .transpose()?
            };

            // If we're spending from the owner account, add the inputs
            if inputs_to_spend.revealed.is_positive() || fee_inputs_to_spend.revealed.is_positive() {
                substate_inputs.push(InputDeclaration::write(*owner_account.component_address()));

                // Add the vaults for XTR (fees) and the spending resource if different
                if let Some(vault) = self
                    .accounts_api
                    .get_vault_by_resource(owner_account.component_address(), &TARI_TOKEN)
                    .optional()?
                {
                    substate_inputs.push(InputDeclaration::write(vault.id));
                    substate_inputs.push(InputDeclaration::read(vault.resource_address));
                }
                if params.resource_address != TARI_TOKEN &&
                    let Some(vault) = self
                        .accounts_api
                        .get_vault_by_resource(owner_account.component_address(), &params.resource_address)
                        .optional()?
                {
                    substate_inputs.push(InputDeclaration::write(vault.id));
                    substate_inputs.push(InputDeclaration::read(vault.resource_address));
                }
            }

            // Any change outputs? When merged, the fee is revealed by this statement too, so it comes out
            // of the inputs before any change is left over.
            let amount_to_spend = params.total_output_amount_with_fee(merged_fee_amount)?;
            let change_amount = inputs_to_spend
                .total_amount()
                .checked_sub(amount_to_spend)
                .ok_or_else(|| StealthTransferApiError::InvariantViolation {
                    details: format!(
                        "Total input amount {} is less than total output amount {}",
                        inputs_to_spend.total_amount(),
                        amount_to_spend
                    ),
                })?;

            let change_output = Some(StealthOutputToCreate {
                owner_address,
                amount: change_amount
                    .to_u64_checked()
                    .ok_or_else(|| StealthTransferApiError::InvariantViolation {
                        // Technically, you could create multiple outputs, but for simplicity and because this is
                        // extremely unlikely to be needed, we only create one here
                        details: "Change amount exceeds u64".to_string(),
                    })?,
                memo: None,
                pay_to: PayTo::StealthPublicKey,
            });

            // Merged, the fee is drawn from this statement's revealed remainder alongside any revealed
            // amounts owed to recipients.
            let output_revealed_amount = params
                .total_revealed_output_amount()
                .and_then(|revealed| revealed.checked_add(merged_fee_amount))
                .ok_or_else(|| StealthTransferApiError::InvariantViolation {
                    details: "Total revealed output amount plus the fee exceeds the maximum amount".to_string(),
                })?;
            let outputs_to_create = params
                .outputs
                .iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, StealthOutputsApiError>>()?;

            let transfer_statement = self.outputs_api.generate_transfer_statement(TransferStatementParams {
                view_only_key_id: owner_account.view_only_key_id(),
                resource_address: &params.resource_address,
                resource_view_key,
                inputs: &inputs_to_spend.inputs,
                input_revealed_amount: inputs_to_spend.revealed,
                outputs: outputs_to_create
                    .into_iter()
                    .chain(change_output)
                    .filter(|o| o.amount > 0),
                // The main intent's signer takes this statement's revealed funds; a merged transaction has no main
                // intent of its own, so the fee signer is the transaction's only signer.
                revealed_output: (!output_revealed_amount.is_zero()).then(|| {
                    let signer = main_intent_signer.as_ref().unwrap_or(&fee_signer);
                    RevealedOutput::new(output_revealed_amount, signer.public_key().to_byte_type())
                }),
            })?;

            // Add the unconfirmed change output to the wallet store
            // NOTE: we can get the nth element because outputs are guaranteed to be in the order we pass them to
            // generate_transfer_statement
            if change_amount.is_positive() &&
                let Some(output) = transfer_statement.outputs_statement.outputs.last()
            {
                debug!(
                target: LOG_TARGET,
                "Adding TRANSFER unconfirmed output with commitment {} for amount {} to account {}",
                output.output.commitment,
                change_amount,
                owner_account.component_address()
                );
                self.add_unconfirmed_output_from_statement(
                    lock.id(),
                    &owner_account,
                    params.resource_address,
                    output,
                    change_amount
                        .to_u64_checked()
                        .ok_or_else(|| StealthTransferApiError::InvariantViolation {
                            details: "Change amount exceeds u64".to_string(),
                        })?,
                    None,
                )?;
            }

            // Add all input UTXO substates to transaction inputs
            substate_inputs.extend(
                fee_inputs_to_spend
                    .inputs
                    .iter()
                    // We may lock the fee change UTXO for spending, however since this does not exist yet, we do not include it as a tx input
                    .filter(|i| i.is_on_chain)
                    .map(|i| &i.commitment)
                    .map(|commitment| UtxoAddress::new(fee_resource, (*commitment).into()))
                    .map(InputDeclaration::write),
            );

            substate_inputs.extend(
                inputs_to_spend
                    .inputs
                    .iter()
                    .filter(|i| i.is_on_chain)
                    .map(|i| &i.commitment)
                    .map(|commitment| UtxoAddress::new(params.resource_address, (*commitment).into()))
                    .map(InputDeclaration::write),
            );

            // Add any swap-related inputs if any
            if let Some(swap) = params.fee_params.pay_fee_with_swap.as_ref() {
                substate_inputs.push(InputDeclaration::read(swap.input_resource));
                substate_inputs.push(InputDeclaration::write(swap.pool_address));
                // Add the pool component's dependent substates (e.g. its vaults)
                substate_inputs.extend(swap_pool_deps.into_iter().map(InputDeclaration::from));
            }

            // Add badge vault if needed
            if let Some(badge_resource_address) = params.badge_usage.resource_address() {
                let badge_vault = self
                    .accounts_api
                    .get_vault_by_resource(owner_account.component_address(), badge_resource_address)
                    .optional()?
                    .ok_or_else(|| StealthTransferApiError::BadgeVaultNotFound {
                        resource_address: *badge_resource_address,
                    })?;
                substate_inputs.push(InputDeclaration::write(badge_vault.id));
            }

            // We assume that all inputs being spent require a signature. This is fine because we currently filter out
            // inputs that have complex access rules from input selection.
            let utxo_spend_keys = inputs_to_spend
                .inputs
                .iter()
                .chain(&fee_inputs_to_spend.inputs)
                .map(|i| StealthUtxoSpendKeyId {
                    account_key_id,
                    public_nonce: i.public_nonce,
                })
                .collect();

            // Only the shape the static fee estimate models: merged, so the statement itself
            // reveals the fee, and free of anything that would add an instruction the estimate does
            // not price. A badge proof, a withdraw of revealed inputs and a revealed payout each
            // call into a template, bringing a template load and WASM execution with them.
            let statically_priced_shape = (is_merged &&
                params.badge_usage.is_none() &&
                inputs_to_spend.revealed.is_zero() &&
                !params.has_revealed_outputs() &&
                !transfer_statement.outputs_statement.outputs.is_empty())
            .then(|| {
                StaticallyPricedShape::new(
                    inputs_to_spend.inputs.len(),
                    transfer_statement.outputs_statement.outputs.len(),
                    transfer_statement
                        .outputs_statement
                        .outputs
                        .iter()
                        .map(persisted_utxo_bytes)
                        .sum(),
                    resource_has_view_key,
                )
            });

            let transaction = self.generate_transfer_transaction(
                network,
                &owner_account,
                params,
                substate_inputs,
                fee_transfer_statement,
                transfer_statement,
            )?;

            Ok((lock, StealthTransferOutput {
                transaction,
                fee_inputs: fee_inputs_to_spend,
                transfer_inputs: inputs_to_spend,
                utxo_spend_keys,
                additional_signer: main_intent_signer,
                main_signer: fee_signer,
                statically_priced_shape,
            }))
        })
    }

    /// Determine if the destination account exists and add any required inputs to the substate inputs list
    /// If the account exists on-chain, returns true. If not, returns false.
    async fn determine_destination_account_inputs(
        &self,
        address: &OotleAddress,
        resource_address: &ResourceAddress,
        substate_inputs: &mut Vec<InputDeclaration>,
    ) -> Result<bool, StealthTransferApiError> {
        let destination_account = derive_account_address_from_public_key(address.account_public_key());

        // Local account? (Saves a call to the indexer)
        match self
            .accounts_api
            .get_account_by_address(&destination_account)
            .optional()?
        {
            Some(local_account) => {
                if local_account.is_confirmed_on_chain() {
                    substate_inputs.push(InputDeclaration::write(destination_account));
                    if let Some(vault) = self
                        .accounts_api
                        .get_vault_by_resource(local_account.component_address(), resource_address)
                        .optional()?
                    {
                        substate_inputs.push(InputDeclaration::write(vault.id));
                    }

                    Ok(true)
                } else {
                    Ok(false)
                }
            },
            None => {
                // We need to determine if the account exists and fetch any existing vault for that resource
                let to_account_substate = self
                    .substate_api
                    .fetch_substate_from_network(&SubstateId::Component(destination_account), None)
                    .await
                    .optional()?;

                if let Some(ValidatorScanResult { id: address, substate }) = to_account_substate {
                    substate_inputs.push(InputDeclaration::write(destination_account));

                    let account =
                        substate
                            .component()
                            .ok_or_else(|| StealthTransferApiError::UnexpectedIndexerResponse {
                                details: format!(
                                    "Expected indexer to return component for address {}. It returned {}",
                                    destination_account, address
                                ),
                            })?;
                    let dest_account = BuiltinAccount::from_value(account.state()).map_err(|e| {
                        StealthTransferApiError::UnexpectedIndexerResponse {
                            details: format!("Failed to convert component substate to account: {e}"),
                        }
                    })?;
                    // If they have an existing vault, we need to add it as an input
                    if let Some(vault) = dest_account.get_vault_by_resource(resource_address) {
                        debug!(
                            target: LOG_TARGET,
                            "Found existing vault {} for resource {} in destination account {}",
                            vault.vault_id(),
                            resource_address,
                            destination_account
                        );
                        substate_inputs.push(InputDeclaration::write(vault.vault_id()));
                    } else {
                        debug!(
                            target: LOG_TARGET,
                            "No existing vault found for resource {} in destination account {}. It will be created.",
                            resource_address,
                            destination_account
                        );
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    fn generate_transfer_transaction(
        &self,
        network: Network,
        owner_account: &AccountWithAddress,
        params: StealthTransferParams,
        inputs: Vec<InputDeclaration>,
        fee_transfer_statement: Option<StealthTransferStatement>,
        transfer_statement: StealthTransferStatement,
    ) -> Result<UnsignedTransaction, StealthTransferApiError> {
        // Without a fee statement the transfer statement is itself the fee source, which only works
        // while it precedes `pay_fee` - so the whole flow moves into the fee intent.
        let Some(fee_transfer_statement) = fee_transfer_statement else {
            return self.generate_merged_transfer_transaction(
                network,
                owner_account,
                params,
                inputs,
                transfer_statement,
            );
        };

        let revealed_input_amount = transfer_statement.inputs_statement.revealed_amount;
        let revealed_output_amount = transfer_statement.outputs_statement.revealed_output_amount();

        let transaction = Transaction::builder(network.as_byte(), params.max_epoch)
            .with_dry_run(params.is_dry_run)
            .with_fee_instructions_builder(|builder| {
                let fee_resource = params.fee_params.pay_fee_with_swap.as_ref().map(|swap| swap.input_resource).unwrap_or(TARI_TOKEN);
                builder
                    .then(|b| {
                        // When there are no stealth inputs or outputs, skip the stealth transfer instruction
                        // and use a standard withdraw instead. A stealth transfer would still work correctly,
                        // but a plain withdraw is more fee-efficient.
                        let has_no_inputs_or_outputs = fee_transfer_statement.outputs_statement.outputs.is_empty() && fee_transfer_statement.inputs_statement.inputs.is_empty();
                        if has_no_inputs_or_outputs {
                            return b.call_method(*owner_account.component_address(), "withdraw", args![
                                fee_resource,
                                fee_transfer_statement.inputs_statement.revealed_amount,
                            ]);
                        }

                        if fee_transfer_statement.inputs_statement.revealed_amount.is_positive() {
                            // Withdraw from the owner's account
                            b.call_method(*owner_account.component_address(), "withdraw", args![
                                fee_resource,
                                fee_transfer_statement.inputs_statement.revealed_amount
                            ])
                            .put_last_instruction_output_on_workspace("fee_other_resx_input_bucket")
                            // Transfer
                            .stealth_transfer_with_input_bucket(fee_resource, fee_transfer_statement, "fee_other_resx_input_bucket")
                        } else {
                            b.stealth_transfer(fee_resource, fee_transfer_statement)
                        }
                    })
                    .then(|b| {
                        if let Some(swap) = params.fee_params.pay_fee_with_swap.as_ref() {
                            b.put_last_instruction_output_on_workspace("fee_swap_input_bucket")
                             .call_method(swap.pool_address, "swap", args![Workspace("fee_swap_input_bucket")])
                             .put_last_instruction_output_on_workspace("fee_swap_output_bucket")
                             // Slippage protection - ensure that the output from the swap is at least the minimum fee amount
                             .assert_bucket_contains_at_least("fee_swap_output_bucket", TARI_TOKEN, swap.min_xtr_output_amount)
                             // Take only max_fee from the swap output for the fee payment
                             .take_from_bucket("fee_swap_output_bucket", params.max_fee, "fee_input_bucket")
                             // Deposit any excess TARI back to the user's account
                             .call_method(*owner_account.component_address(), "deposit", args![Workspace("fee_swap_output_bucket")])
                        } else {
                            b.put_last_instruction_output_on_workspace("fee_input_bucket")
                        }
                    })
                    .pay_fee_from_bucket("fee_input_bucket")
            })
            .then(|builder| {
                // Badge if required
                match &params.badge_usage {
                    BadgeUsage::None => builder,
                    BadgeUsage::Resource(resx) => builder
                        .call_method(*owner_account.component_address(), "create_proof_for_resource", args![
                            resx
                        ])
                        .put_last_instruction_output_on_workspace("proof")
                        .add_input(*resx),
                    BadgeUsage::NonFungible(nft) => builder
                        .call_method(
                            *owner_account.component_address(),
                            "create_proof_by_non_fungible",
                            args![nft],
                        )
                        .put_last_instruction_output_on_workspace("proof")
                        .add_input(*nft.resource_address())
                        .add_input(nft.clone()),
                    BadgeUsage::AmountOfResource { amount, resource } => builder
                        .call_method(*owner_account.component_address(), "create_proof_by_amount", args![
                            resource, amount
                        ])
                        .put_last_instruction_output_on_workspace("proof")
                        .add_input(*resource),
                }
            })
            .then(|builder| {
                // When there are no stealth inputs or outputs, skip the stealth transfer instruction
                // and use a standard withdraw instead. A stealth transfer would still work correctly,
                // but a plain withdraw is more fee-efficient.
                let has_no_inputs_or_outputs = transfer_statement.outputs_statement.outputs.is_empty() && transfer_statement.inputs_statement.inputs.is_empty();
                if has_no_inputs_or_outputs {
                    return withdraw_from_account(
                        builder,
                        *owner_account.component_address(),
                        params.resource_address,
                        revealed_input_amount,
                        &params.badge_usage,
                    );
                }

                if revealed_input_amount.is_positive() {
                    withdraw_from_account(
                        builder,
                        owner_account.account.component_address,
                        params.resource_address,
                        revealed_input_amount,
                        &params.badge_usage,
                    )
                        .put_last_instruction_output_on_workspace("input_bucket")
                        .stealth_transfer_with_input_bucket(params.resource_address, transfer_statement, "input_bucket")
                } else {
                    builder.stealth_transfer(params.resource_address, transfer_statement)
                }
            })
            .then(|builder| {
                // revealed_to_account may be Some, but we only use it if revealed_output_amount is greater than zero.
                if revealed_output_amount.is_zero() {
                    return builder;
                }

                // If the transfer creates revealed outputs, deposit the bucket into the destination account.
                builder
                    .put_last_instruction_output_on_workspace("output_bucket")
                    .then(|builder| {
                        params.outputs.iter().enumerate().fold(builder, |builder, (i, output)| {
                            if !output.revealed_amount.is_positive() {
                                return builder;
                            }
                            let needs_to_split = params.outputs.len() > 1;

                            if needs_to_split {
                                let sub_bucket_name = format!("output-sub-bucket-{i}");
                                builder
                                    .take_from_bucket("output_bucket", output.revealed_amount, &sub_bucket_name)
                                    .create_account_with_bucket(*output.address.account_public_key(), sub_bucket_name)
                            } else {
                                builder
                                    .create_account_with_bucket(*output.address.account_public_key(), "output_bucket")
                            }
                        })
                    })
            })
            .then(|builder| {
                if params.badge_usage.is_none() {
                    builder
                } else {
                    builder.drop_all_proofs_in_workspace()
                }
            })
            .with_inputs(inputs)
            .build_unsigned();

        Ok(transaction)
    }

    /// Builds a transaction whose single stealth transfer both pays the recipients and reveals the fee.
    /// Every instruction sits in the fee intent: the statement has to precede `pay_fee` for its revealed
    /// remainder to fund the fee at all, which is the whole point of merging, so the main intent is empty.
    #[allow(clippy::too_many_lines)]
    fn generate_merged_transfer_transaction(
        &self,
        network: Network,
        owner_account: &AccountWithAddress,
        params: StealthTransferParams,
        inputs: Vec<InputDeclaration>,
        transfer_statement: StealthTransferStatement,
    ) -> Result<UnsignedTransaction, StealthTransferApiError> {
        let revealed_input_amount = transfer_statement.inputs_statement.revealed_amount;
        let pays_revealed_to_recipients = params.has_revealed_outputs();
        let account_address = *owner_account.component_address();

        let transaction = Transaction::builder(network.as_byte(), params.max_epoch)
            .with_dry_run(params.is_dry_run)
            .with_fee_instructions_builder(|builder| {
                builder
                    // A badge proof has to exist before the transfer relying on it. The merged flow is
                    // wholly contained in this intent, so the proof is created and dropped here too.
                    .then(|b| match &params.badge_usage {
                        BadgeUsage::None => b,
                        BadgeUsage::Resource(resx) => b
                            .call_method(account_address, "create_proof_for_resource", args![resx])
                            .put_last_instruction_output_on_workspace("proof"),
                        BadgeUsage::NonFungible(nft) => b
                            .call_method(account_address, "create_proof_by_non_fungible", args![nft])
                            .put_last_instruction_output_on_workspace("proof"),
                        BadgeUsage::AmountOfResource { amount, resource } => b
                            .call_method(account_address, "create_proof_by_amount", args![resource, amount])
                            .put_last_instruction_output_on_workspace("proof"),
                    })
                    .then(|b| {
                        // When there are no stealth inputs or outputs, skip the stealth transfer
                        // instruction and use a standard withdraw instead. A stealth transfer would still
                        // work correctly, but a plain withdraw is more fee-efficient.
                        let has_no_inputs_or_outputs = transfer_statement.outputs_statement.outputs.is_empty() &&
                            transfer_statement.inputs_statement.inputs.is_empty();
                        if has_no_inputs_or_outputs {
                            return withdraw_from_account(
                                b,
                                account_address,
                                params.resource_address,
                                revealed_input_amount,
                                &params.badge_usage,
                            );
                        }

                        if revealed_input_amount.is_positive() {
                            withdraw_from_account(
                                b,
                                account_address,
                                params.resource_address,
                                revealed_input_amount,
                                &params.badge_usage,
                            )
                            .put_last_instruction_output_on_workspace("input_bucket")
                            .stealth_transfer_with_input_bucket(
                                params.resource_address,
                                transfer_statement,
                                "input_bucket",
                            )
                        } else {
                            b.stealth_transfer(params.resource_address, transfer_statement)
                        }
                    })
                    .put_last_instruction_output_on_workspace("revealed_bucket")
                    .then(|b| {
                        // The revealed remainder covers the fee plus anything owed to recipients as
                        // revealed funds. With nothing owed, the whole bucket is the fee.
                        if pays_revealed_to_recipients {
                            b.take_from_bucket("revealed_bucket", params.max_fee, "fee_input_bucket")
                                .pay_fee_from_bucket("fee_input_bucket")
                        } else {
                            b.pay_fee_from_bucket("revealed_bucket")
                        }
                    })
                    .then(|b| {
                        if !pays_revealed_to_recipients {
                            return b;
                        }
                        params.outputs.iter().enumerate().fold(b, |b, (i, output)| {
                            if !output.revealed_amount.is_positive() {
                                return b;
                            }
                            let needs_to_split = params.outputs.len() > 1;

                            if needs_to_split {
                                let sub_bucket_name = format!("output-sub-bucket-{i}");
                                b.take_from_bucket("revealed_bucket", output.revealed_amount, &sub_bucket_name)
                                    .create_account_with_bucket(*output.address.account_public_key(), sub_bucket_name)
                            } else {
                                b.create_account_with_bucket(*output.address.account_public_key(), "revealed_bucket")
                            }
                        })
                    })
                    .then(|b| {
                        if params.badge_usage.is_none() {
                            b
                        } else {
                            b.drop_all_proofs_in_workspace()
                        }
                    })
            })
            .then(|builder| match &params.badge_usage {
                BadgeUsage::None => builder,
                BadgeUsage::Resource(resx) => builder.add_input(*resx),
                BadgeUsage::NonFungible(nft) => builder.add_input(*nft.resource_address()).add_input(nft.clone()),
                BadgeUsage::AmountOfResource { resource, .. } => builder.add_input(*resource),
            })
            .with_inputs(inputs)
            .build_unsigned();

        Ok(transaction)
    }

    fn add_unconfirmed_output_from_statement(
        &self,
        lock_id: WalletLockId,
        account: &AccountWithAddress,
        resource_address: ResourceAddress,
        output: &StealthUnspentOutput,
        value: u64,
        memo: Option<Memo>,
    ) -> Result<(), StealthTransferApiError> {
        self.outputs_api.add_output(&StealthOutputModel {
            owner_account: *account.component_address(),
            resource_address,
            commitment: output.output.commitment,
            value,
            sender_public_nonce: output.output.sender_public_nonce,
            view_only_key_id: account.view_only_key_id(),
            owner_key_id: account.owner_key_id(),
            encrypted_data: output.output.encrypted_data.clone(),
            status: OutputStatus::LockedUnconfirmed,
            memo,
            auth: output.auth.clone(),
            minimum_value_promise: output.output.minimum_value_promise,
            tag_byte: output.tag,
            lock_id: Some(lock_id),
            is_burnt: false,
            is_frozen: false,
            is_on_chain: false,
            is_condition_spendable: self.outputs_api.is_spendable_auth(&output.auth),
        })?;
        Ok(())
    }
}

/// Withdraws `amount` of `resource` from the owner's account, handing the badge proof to the account's frame when
/// one is in use. A frame authorizes with the badges it is stamped with and the proofs it is passed as arguments,
/// so a badge reaches the account's withdraw rule as an argument.
fn withdraw_from_account<D>(
    builder: TransactionBuilder<D>,
    account: ComponentAddress,
    resource: ResourceAddress,
    amount: Amount,
    badge_usage: &BadgeUsage,
) -> TransactionBuilder<D> {
    if badge_usage.is_none() {
        builder.call_method(account, "withdraw", args![resource, amount])
    } else {
        builder.call_method(account, "withdraw_with_auth", args![
            resource,
            amount,
            Workspace("proof")
        ])
    }
}

#[cfg(test)]
mod merged_credit_tests {
    use tari_engine_types::limits::{FREE_COMPUTE_GRACE_POINTS, NativeExecutionPoints as P};

    use super::{MERGED_STATEMENT_CREDIT_UTILISATION_PERCENT, merged_statement_fits_credit};

    fn budget() -> u64 {
        FREE_COMPUTE_GRACE_POINTS / 100 * MERGED_STATEMENT_CREDIT_UTILISATION_PERCENT
    }

    #[test]
    fn a_single_recipient_send_merges() {
        // One recipient output plus one change output, the overwhelmingly common shape.
        assert!(merged_statement_fits_credit(2, 2, false));
    }

    #[test]
    fn two_recipients_plus_change_is_the_ceiling() {
        // Three no-view-key outputs is the most the credit funds; the fourth tips it over even with
        // zero inputs, which is what caps a merged send at two recipients.
        assert!(merged_statement_fits_credit(0, 3, false));
        assert!(!merged_statement_fits_credit(0, 4, false));
    }

    #[test]
    fn inputs_can_exhaust_the_credit_for_a_mergeable_output_count() {
        // Outputs dominate, but enough inputs still push a 3-output statement past the budget and force
        // the fallback - the case the runtime input count guards after selection.
        let per_output_room = budget() - (P::PER_STATEMENT + 3 * P::PER_OUTPUT);
        let max_inputs = (per_output_room / P::PER_INPUT) as usize;
        assert!(merged_statement_fits_credit(max_inputs, 3, false));
        assert!(!merged_statement_fits_credit(max_inputs + 1, 3, false));
    }

    #[test]
    fn a_view_key_resource_is_priced_more_strictly() {
        // The per-output ElGamal surcharge lowers the output ceiling for view-key resources.
        assert!(merged_statement_fits_credit(0, 3, false));
        assert!(!merged_statement_fits_credit(0, 3, true));
    }
}
