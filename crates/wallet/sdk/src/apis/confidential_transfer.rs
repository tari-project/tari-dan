//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::cmp;

use log::*;
use ootle_byte_type::{FromByteType, ToByteType};
use tari_bor::{Deserialize, Serialize};
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_ootle_address::OotleAddress;
use tari_ootle_common_types::{
    Epoch,
    SubstateRequirement,
    optional::{IsNotFoundError, Optional},
};
use tari_ootle_transaction::{Transaction, args};
use tari_ootle_wallet_crypto::{MaskAndValue, OutputWitness, memo::Memo};
use tari_template_lib::types::{
    Amount,
    ComponentAddress,
    ConfidentialOutputAddress,
    ResourceAddress,
    VaultId,
    constants::TARI_TOKEN,
    crypto::PedersenCommitmentBytes,
};

use crate::{
    apis::{
        accounts::{AccountsApi, AccountsApiError},
        confidential_crypto::{ConfidentialCryptoApi, ConfidentialCryptoApiError},
        confidential_outputs::{ConfidentialOutputsApi, ConfidentialOutputsApiError},
        config::{ConfigApi, ConfigApiError},
        key_manager::{KeyManagerApi, KeyManagerApiError},
        locks::{LocksApi, LocksApiError},
        substate::{SubstateApiError, SubstatesApi},
        transaction::{TransactionApi, TransactionApiError},
    },
    models::{ConfidentialOutputModel, KeyBranch, OutputStatus, WalletLockId},
    spec::WalletSdkSpec,
    storage::WalletStorageError,
};

const LOG_TARGET: &str = "tari::ootle::wallet_sdk::apis::confidential_transfers";

/// Only outputs the wallet has confirmed on-chain (`OutputStatus::Unspent`) are ever locked for spending, so
/// every locked input names a `ConfidentialOutput` substate that exists.
fn commitments_of(outputs: &[ConfidentialOutputModel]) -> Vec<PedersenCommitmentBytes> {
    outputs.iter().map(|o| o.commitment).collect()
}

pub struct ConfidentialTransferApi<'a, TSpec: WalletSdkSpec> {
    key_manager_api: KeyManagerApi<'a, TSpec>,
    locks_api: LocksApi<'a, TSpec::Store>,
    accounts_api: AccountsApi<'a, TSpec>,
    confidential_outputs_api: ConfidentialOutputsApi<'a, TSpec>,
    transaction_api: TransactionApi<'a, TSpec::Store, TSpec::NetworkInterface>,
    substate_api: SubstatesApi<'a, TSpec::Store, TSpec::NetworkInterface>,
    crypto_api: ConfidentialCryptoApi,
    config_api: ConfigApi<'a, TSpec::Store>,
}

impl<'a, TSpec: WalletSdkSpec> ConfidentialTransferApi<'a, TSpec>
where TSpec: WalletSdkSpec
{
    pub fn new(
        key_manager_api: KeyManagerApi<'a, TSpec>,
        accounts_api: AccountsApi<'a, TSpec>,
        locks_api: LocksApi<'a, TSpec::Store>,
        confidential_outputs_api: ConfidentialOutputsApi<'a, TSpec>,
        substate_api: SubstatesApi<'a, TSpec::Store, TSpec::NetworkInterface>,
        transaction_api: TransactionApi<'a, TSpec::Store, TSpec::NetworkInterface>,
        crypto_api: ConfidentialCryptoApi,
        config_api: ConfigApi<'a, TSpec::Store>,
    ) -> Self {
        Self {
            key_manager_api,
            locks_api,
            accounts_api,
            confidential_outputs_api,
            substate_api,
            transaction_api,
            crypto_api,
            config_api,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn resolved_inputs_for_transfer(
        &self,
        lock_id: WalletLockId,
        from_account: ComponentAddress,
        resource_address: ResourceAddress,
        spend_amount: Amount,
        input_selection: UtxoInputSelection,
    ) -> Result<InputsToSpend, ConfidentialTransferApiError> {
        let src_vault = self
            .accounts_api
            .get_vault_by_resource(&from_account, &resource_address)?;

        let available_revealed_funds = src_vault.available_revealed_balance();

        match &input_selection {
            UtxoInputSelection::ConfidentialOnly => {
                let (confidential_inputs, _) =
                    self.confidential_outputs_api
                        .lock_outputs_by_amount(lock_id, &src_vault.id, spend_amount)?;
                let commitments = commitments_of(&confidential_inputs);
                let confidential_inputs = self
                    .confidential_outputs_api
                    .resolve_output_masks(confidential_inputs)?;

                info!(
                    target: LOG_TARGET,
                    "ConfidentialOnly: Locked {} confidential inputs for transfer from {}",
                    confidential_inputs.len(),
                    src_vault.id,
                );

                Ok(InputsToSpend {
                    confidential: confidential_inputs,
                    revealed: Amount::zero(),
                    commitments,
                })
            },
            UtxoInputSelection::RevealedOnly => {
                if available_revealed_funds < spend_amount {
                    return Err(ConfidentialTransferApiError::InsufficientFunds);
                }

                self.locks_api
                    .lock_funds_in_vault(lock_id, &src_vault.id, spend_amount)?;

                info!(
                    target: LOG_TARGET,
                    "RevealedOnly: Spending {} revealed balance for transfer from {}",
                    spend_amount,
                    src_vault.id,
                );

                Ok(InputsToSpend {
                    confidential: vec![],
                    revealed: spend_amount,
                    commitments: vec![],
                })
            },
            UtxoInputSelection::PreferRevealed => {
                let revealed_to_spend = cmp::min(available_revealed_funds, spend_amount);
                let confidential_to_spend = spend_amount - revealed_to_spend;
                if confidential_to_spend.is_zero() {
                    info!(
                        target: LOG_TARGET,
                        "PreferRevealed: Spending {} revealed balance for transfer from {}",
                        revealed_to_spend,
                        src_vault.id,
                    );

                    self.locks_api
                        .lock_funds_in_vault(lock_id, &src_vault.id, revealed_to_spend)?;

                    return Ok(InputsToSpend {
                        confidential: vec![],
                        revealed: revealed_to_spend,
                        commitments: vec![],
                    });
                }

                let (confidential_inputs, _) = self.confidential_outputs_api.lock_outputs_by_amount(
                    lock_id,
                    &src_vault.id,
                    confidential_to_spend,
                )?;
                let commitments = commitments_of(&confidential_inputs);
                let confidential_inputs = self
                    .confidential_outputs_api
                    .resolve_output_masks(confidential_inputs)?;

                let total_confidential_spent = confidential_inputs
                    .iter()
                    .map(|i| Amount::from(i.value))
                    .sum::<Amount>();

                self.locks_api
                    .lock_funds_in_vault(lock_id, &src_vault.id, revealed_to_spend)?;

                info!(
                    target: LOG_TARGET,
                    "PreferRevealed: Locked {} confidential inputs (target: {}, spent: {}) and {} revealed for amount {} from {}",
                    confidential_inputs.len(),
                    confidential_to_spend,
                    total_confidential_spent,
                    revealed_to_spend,
                    spend_amount,
                    src_vault.id,
                );

                Ok(InputsToSpend {
                    confidential: confidential_inputs,
                    revealed: revealed_to_spend,
                    commitments,
                })
            },
            UtxoInputSelection::PreferConfidential => {
                let (confidential_inputs, amount_locked) = self
                    .confidential_outputs_api
                    .lock_outputs_until_partial_amount(lock_id, &src_vault.id, spend_amount)?;

                let revealed_to_spend = spend_amount.saturating_sub(amount_locked);

                if src_vault.revealed_balance < revealed_to_spend {
                    return Err(ConfidentialTransferApiError::InsufficientFunds);
                }

                self.locks_api
                    .lock_funds_in_vault(lock_id, &src_vault.id, revealed_to_spend)?;

                let commitments = commitments_of(&confidential_inputs);
                let confidential_inputs = self
                    .confidential_outputs_api
                    .resolve_output_masks(confidential_inputs)?;

                Ok(InputsToSpend {
                    confidential: confidential_inputs,
                    revealed: revealed_to_spend,
                    commitments,
                })
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn transfer(
        &self,
        params: ConfidentialTransferParams,
    ) -> Result<TransferOutput, ConfidentialTransferApiError> {
        let from_account = self.accounts_api.get_account_by_address(&params.from_account)?;
        let to_account = self
            .accounts_api
            .resolve_account_by_public_key(params.destination_address.account_public_key())
            .await?;

        let account_owner_key_id =
            from_account
                .owner_key_id()
                .ok_or_else(|| ConfidentialTransferApiError::InvalidParameter {
                    param: "from_account",
                    reason: "From account does not have an owner key".to_string(),
                })?;

        // Determine Transaction Inputs
        let mut inputs = Vec::new();

        let dest_account_exists = to_account.exists_on_chain;
        if dest_account_exists {
            inputs.push(SubstateRequirement::unversioned(to_account.address));
            // Only the destination's vault for this resource is touched by the deposit (if it has none, the
            // deposit creates one). For an account we do not own, we only know its vault ids, so all of them
            // are declared.
            match self
                .accounts_api
                .get_vault_by_resource(&to_account.address, &params.resource_address)
                .optional()?
            {
                Some(vault) => inputs.push(SubstateRequirement::unversioned(vault.id)),
                None => inputs.extend(to_account.vaults.iter().copied().map(SubstateRequirement::unversioned)),
            }
        }

        let account_substate = self.substate_api.get_substate(&params.from_account.into())?;
        inputs.push(account_substate.substate_id.into_unversioned_requirement());

        // Fees are paid out of this account's TARI vault, so that vault and its resource are mutated too.
        if let Some(vault) = self
            .accounts_api
            .get_vault_by_resource(from_account.component_address(), &TARI_TOKEN)
            .optional()?
        {
            inputs.push(SubstateRequirement::unversioned(vault.id));
            inputs.push(SubstateRequirement::unversioned(vault.resource_address));
        }

        let src_vault = self
            .accounts_api
            .get_vault_by_resource(from_account.component_address(), &params.resource_address)?;
        if !src_vault.resource_type.is_confidential() {
            return Err(ConfidentialTransferApiError::InvalidParameter {
                param: "resource_address",
                reason: format!(
                    "Resource {} is {}. Expected confidential.",
                    params.resource_address, src_vault.resource_type
                ),
            });
        }
        let src_vault_substate = self.substate_api.get_substate(&src_vault.id.into())?;
        inputs.push(src_vault_substate.substate_id.into_unversioned_requirement());

        // add the input for the resource address to be transferred
        inputs.push(SubstateRequirement::unversioned(params.resource_address));

        // We need to fetch the resource substate to check if there is a view key present.
        let resource = self.substate_api.fetch_resource(params.resource_address).await?;

        // The badge proof is created from the badge's vault in this account, so both are inputs.
        if let Some(ref badge_resource_address) = params.proof_from_resource {
            inputs.push(SubstateRequirement::unversioned(*badge_resource_address));
            if let Some(badge_vault) = self
                .accounts_api
                .get_vault_by_resource(from_account.component_address(), badge_resource_address)
                .optional()?
            {
                inputs.push(SubstateRequirement::unversioned(badge_vault.id));
            }
        }

        // Reserve and lock input funds for fees
        let max_fee = params.max_fee;

        let account_key = self.key_manager_api.get_key(account_owner_key_id)?;
        // Change comes back to this account, so it is encrypted to this account's own view key, for the same
        // reason destination outputs are.
        let account_view_key = self.key_manager_api.get_key(from_account.account.view_only_key_id)?;
        let account_view_public_key = account_view_key.to_public_key();

        // Reserve and lock input funds
        let lock = self.locks_api.create_lock()?;
        let inputs_to_spend = match self.resolved_inputs_for_transfer(
            lock.id(),
            params.from_account,
            params.resource_address,
            params.amount,
            params.input_selection,
        ) {
            Ok(inputs) => inputs,
            Err(e) => {
                warn!(target: LOG_TARGET, "Unlocking fee fund locks after error: {}", e);
                return Err(e);
            },
        };

        // Each confidential input is a ConfidentialOutput substate that the withdraw downs, so it must be a
        // transaction input. The commitments are only named inside the opaque withdraw proof, so input
        // detection cannot infer these: the address must be derived from (resource, commitment).
        inputs.extend(
            inputs_to_spend
                .commitments
                .iter()
                .map(|commitment| ConfidentialOutputAddress::new(params.resource_address, *commitment))
                .map(SubstateRequirement::unversioned),
        );

        // Generate outputs
        let resource_view_key = resource
            .view_key()
            .map(|k| k.try_from_byte_type())
            .transpose()
            .map_err(|e| ConfidentialTransferApiError::InvalidParameter {
                param: "resource_view_key",
                reason: format!("Invalid resource view key: {e}"),
            })?;
        // Outputs are encrypted to the recipient's view key: that is the key the receiving wallet scans with
        // (see `ConfidentialOutputsApi::verify_and_update_confidential_outputs`), so encrypting to any other
        // key leaves the recipient unable to recover the value and mask.
        let destination_pk = params
            .destination_address
            .view_only_key()
            .try_from_byte_type()
            .map_err(|e| ConfidentialTransferApiError::InvalidParameter {
                param: "destination_view_key",
                reason: format!("Invalid destination view key: {e}"),
            })?;

        let output_statement = if params.confidential_amount().is_zero() {
            None
        } else {
            Some(self.create_confidential_proof_statement(
                &destination_pk,
                params.confidential_amount(),
                resource_view_key.clone(),
                params.memo.as_ref(),
            )?)
        };

        let remaining_left_to_pay = params.amount.checked_sub(inputs_to_spend.revealed).unwrap_or_else(|| {
            panic!(
                "BUG: paid more revealed funds ({}) than the amount to pay ({})",
                inputs_to_spend.revealed, params.amount
            )
        });
        let change_confidential_amount = inputs_to_spend.total_confidential_amount() - remaining_left_to_pay;

        let maybe_change_statement = if change_confidential_amount.is_positive() {
            let statement = self.create_confidential_proof_statement(
                &account_view_public_key,
                change_confidential_amount,
                resource_view_key,
                None,
            )?;

            let change_value = statement.amount;

            if change_value > 0 {
                self.confidential_outputs_api.add_output(ConfidentialOutputModel {
                    account_address: *from_account.component_address(),
                    vault_id: src_vault.id,
                    commitment: statement.to_commitment().to_byte_type(),
                    value: change_value.into(),
                    sender_public_nonce: Some(statement.sender_public_nonce.to_byte_type()),
                    view_only_key_id: account_view_key.key_id,
                    owner_key_id: Some(account_key.key_id),
                    encrypted_data: statement.encrypted_data.clone(),
                    public_asset_tag: None,
                    memo: None,
                    status: OutputStatus::LockedUnconfirmed,
                    lock_id: Some(lock.id()),
                })?;
            }

            Some(statement)
        } else {
            None
        };

        let proof = self.crypto_api.generate_withdraw_proof(
            &inputs_to_spend.confidential,
            inputs_to_spend.revealed,
            output_statement.as_ref(),
            params.revealed_amount(),
            maybe_change_statement.as_ref(),
            Amount::zero(),
        )?;

        let network = self.config_api.get_network()?;
        let transaction = Transaction::builder(network.as_byte(), params.max_epoch)
            .with_dry_run(params.is_dry_run)
            // TODO: we assume that from_account has TARI
            .pay_fee_from_component(*from_account.component_address(), max_fee)
            .create_account(*params.destination_address.account_public_key())
            .then(|builder| {
                if let Some(ref badge) = params.proof_from_resource {
                    builder
                        .call_method(*from_account.component_address(), "create_proof_for_resource", args![badge])
                        .put_last_instruction_output_on_workspace("proof")
                } else {
                    builder
                }
            })
            .call_method(*from_account.component_address(), "withdraw_confidential", args![
                params.resource_address,
                proof
            ])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(to_account.address, "deposit", args![Workspace("bucket")])
            .then(|builder| {
                if params.proof_from_resource.is_some() {
                    builder.drop_all_proofs_in_workspace()
                } else {
                    builder
                }
            })
            .with_inputs(inputs)
            .build_and_seal(&account_key.secret);

        let tx_id = transaction.calculate_id();
        self.transaction_api.locks_set_transaction_id(lock.id(), tx_id)?;

        let lock_id = lock.keep_locked();

        Ok(TransferOutput {
            transaction,
            transaction_proof_id: lock_id,
        })
    }

    fn create_confidential_proof_statement(
        &self,
        dest_public_key: &RistrettoPublicKey,
        confidential_amount: Amount,
        resource_view_key: Option<RistrettoPublicKey>,
        memo: Option<&Memo>,
    ) -> Result<OutputWitness, ConfidentialTransferApiError> {
        if !confidential_amount.is_positive() {
            return Err(ConfidentialTransferApiError::InvalidParameter {
                param: "confidential_amount",
                reason: "Confidential amount must be positive".to_string(),
            });
        }

        let mask = self.key_manager_api.next_key(KeyBranch::ConfidentialMask)?;
        let amount =
            confidential_amount
                .to_u64_checked()
                .ok_or_else(|| ConfidentialTransferApiError::AmountOverflow {
                    param: "confidential_amount",
                    details: "Confidential amount exceeds u64. This is currently a limitation due to the format of \
                              EncryptedData"
                        .to_string(),
                })?;

        let (nonce, public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let encrypted_data =
            self.crypto_api
                .encrypt_value_and_mask(amount, &mask.key, dest_public_key, &nonce, memo)?;

        Ok(OutputWitness {
            amount,
            mask: mask.key,
            sender_public_nonce: public_nonce,
            encrypted_data,
            minimum_value_promise: 0,
            resource_view_key,
        })
    }
}

pub struct TransferOutput {
    pub transaction: Transaction,
    pub transaction_proof_id: WalletLockId,
}

#[derive(Debug)]
pub struct ConfidentialTransferParams {
    /// Spend from this account
    pub from_account: ComponentAddress,
    /// Strategy for input selection
    pub input_selection: UtxoInputSelection,
    /// Amount to spend to destination
    pub amount: Amount,
    /// Destination address used to derive the destination account component
    pub destination_address: OotleAddress,
    /// Address of the resource to transfer
    pub resource_address: ResourceAddress,
    /// Fee to lock for the transaction
    pub max_fee: u64,
    /// If true, the output will contain only a revealed amount. Otherwise, only confidential amounts.
    pub output_to_revealed: bool,
    /// If some, instructions are added that create a access rule proof for this resource before calling withdraw
    pub proof_from_resource: Option<ResourceAddress>,
    /// A memo to include in the output, if any. This memo is encrypted in the output and can only be decrypted by the
    /// recipient
    pub memo: Option<Memo>,
    /// The last epoch the built transaction may be sequenced in. Mandatory: every transaction
    /// carries a bounded validity window, so the caller decides how long this one stays
    /// submittable.
    pub max_epoch: Epoch,
    /// Run as a dry run, no funds will be transferred if true
    pub is_dry_run: bool,
}

impl ConfidentialTransferParams {
    pub fn confidential_amount(&self) -> Amount {
        if self.output_to_revealed {
            Amount::zero()
        } else {
            self.amount
        }
    }

    pub fn revealed_amount(&self) -> Amount {
        if self.output_to_revealed {
            self.amount
        } else {
            Amount::zero()
        }
    }
}

impl ConfidentialTransferParams {
    pub fn total_amount(&self) -> Amount {
        self.amount + self.max_fee
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum UtxoInputSelection {
    ConfidentialOnly,
    RevealedOnly,
    PreferRevealed,
    PreferConfidential,
}

#[derive(Debug)]
pub struct InputsToSpend {
    pub confidential: Vec<MaskAndValue>,
    pub revealed: Amount,
    /// Commitments of the confidential inputs, in the same order as `confidential`. A `MaskAndValue` does not
    /// carry the commitment, and each spent commitment names a [`ConfidentialOutputAddress`] that must be
    /// declared as a transaction input.
    pub commitments: Vec<PedersenCommitmentBytes>,
}

impl InputsToSpend {
    pub fn total_amount(&self) -> Amount {
        self.total_confidential_amount() + self.revealed
    }

    pub fn total_confidential_amount(&self) -> Amount {
        self.confidential.iter().map(|o| Amount::from(o.value)).sum()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfidentialTransferApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("Confidential crypto error: {0}")]
    ConfidentialCrypto(#[from] ConfidentialCryptoApiError),
    #[error("Confidential outputs error: {0}")]
    OutputsApi(#[from] ConfidentialOutputsApiError),
    #[error("Substate API error: {0}")]
    SubstateApi(#[from] SubstateApiError),
    #[error("Insufficient funds")]
    InsufficientFunds,
    #[error("Key manager error: {0}")]
    KeyManager(#[from] KeyManagerApiError),
    #[error("Accounts API error: {0}")]
    Accounts(#[from] AccountsApiError),
    #[error("Invalid parameter `{param}`: {reason}")]
    InvalidParameter { param: &'static str, reason: String },
    #[error("Unexpected indexer response: {details}")]
    UnexpectedIndexerResponse { details: String },
    #[error("Config API error: {0}")]
    ConfigApi(#[from] ConfigApiError),
    #[error("Amount overflow for parameter `{param}`: {details}")]
    AmountOverflow { param: &'static str, details: String },
    #[error("Transaction API error: {0}")]
    TransactionApiError(#[from] TransactionApiError),
    #[error("Lock error: {0}")]
    LocksApiError(#[from] LocksApiError),
}

impl IsNotFoundError for ConfidentialTransferApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}

pub struct ResolvedAccountDetails {
    pub address: ComponentAddress,
    pub vaults: Vec<VaultId>,
    pub exists_on_chain: bool,
}
