//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::cmp;

use digest::crypto_common::rand_core::OsRng;
use log::*;
use tari_bor::{Deserialize, Serialize};
use tari_common_types::types::PublicKey;
use tari_crypto::{commitment::HomomorphicCommitmentFactory, keys::PublicKey as _};
use tari_dan_common_types::optional::{IsNotFoundError, Optional};
use tari_dan_wallet_crypto::{ConfidentialOutputMaskAndValue, ConfidentialProofStatement};
use tari_engine_types::{
    component::new_account_address_from_parts,
    confidential::get_commitment_factory,
    substate::SubstateId,
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    models::{Amount, ComponentAddress, ResourceAddress},
};
use tari_transaction::Transaction;

use crate::{
    apis::{
        accounts::{AccountsApi, AccountsApiError},
        confidential_crypto::{ConfidentialCryptoApi, ConfidentialCryptoApiError},
        confidential_outputs::{ConfidentialOutputsApi, ConfidentialOutputsApiError},
        key_manager,
        key_manager::{KeyManagerApi, KeyManagerApiError},
        substate::{SubstateApiError, SubstatesApi, ValidatorScanResult},
    },
    models::{ConfidentialOutputModel, ConfidentialProofId, OutputStatus, VersionedSubstateId},
    network::WalletNetworkInterface,
    storage::{WalletStorageError, WalletStore},
};

const LOG_TARGET: &str = "tari::dan::wallet_sdk::apis::confidential_transfers";

pub struct ConfidentialTransferApi<'a, TStore, TNetworkInterface> {
    key_manager_api: KeyManagerApi<'a, TStore>,
    accounts_api: AccountsApi<'a, TStore>,
    outputs_api: ConfidentialOutputsApi<'a, TStore>,
    substate_api: SubstatesApi<'a, TStore, TNetworkInterface>,
    crypto_api: ConfidentialCryptoApi,
}

impl<'a, TStore, TNetworkInterface> ConfidentialTransferApi<'a, TStore, TNetworkInterface>
where
    TStore: WalletStore,
    TNetworkInterface: WalletNetworkInterface,
    TNetworkInterface::Error: IsNotFoundError,
{
    pub fn new(
        key_manager_api: KeyManagerApi<'a, TStore>,
        accounts_api: AccountsApi<'a, TStore>,
        outputs_api: ConfidentialOutputsApi<'a, TStore>,
        substate_api: SubstatesApi<'a, TStore, TNetworkInterface>,
        crypto_api: ConfidentialCryptoApi,
    ) -> Self {
        Self {
            key_manager_api,
            accounts_api,
            outputs_api,
            substate_api,
            crypto_api,
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn resolved_inputs_for_transfer(
        &self,
        params: &TransferParams,
    ) -> Result<OutputsToSpend, ConfidentialTransferApiError> {
        let src_vault = self
            .accounts_api
            .get_vault_by_resource(&params.from_account.into(), &params.resource_address)?;

        match &params.input_selection {
            ConfidentialTransferInputSelection::ConfidentialOnly => {
                let proof_id = self.outputs_api.add_proof(&src_vault.address)?;
                let (confidential_inputs, _) = self.outputs_api.lock_outputs_by_amount(
                    &src_vault.address,
                    params.amount,
                    proof_id,
                    params.is_dry_run,
                )?;
                let confidential_inputs = self
                    .outputs_api
                    .resolve_output_masks(confidential_inputs, key_manager::TRANSACTION_BRANCH)?;

                info!(
                    target: LOG_TARGET,
                    "ConfidentialOnly: Locked {} confidential inputs for transfer from {} to {}",
                    confidential_inputs.len(),
                    src_vault.address,
                    params.destination_public_key
                );

                Ok(OutputsToSpend {
                    confidential: confidential_inputs,
                    proof_id: Some(proof_id),
                    revealed: Amount::zero(),
                })
            },
            ConfidentialTransferInputSelection::RevealedOnly => {
                if src_vault.revealed_balance < params.amount {
                    return Err(ConfidentialTransferApiError::InsufficientFunds);
                }

                info!(
                    target: LOG_TARGET,
                    "RevealedOnly: Spending {} revealed balance for transfer from {} to {}",
                    params.amount,
                    src_vault.address,
                    params.destination_public_key
                );

                Ok(OutputsToSpend {
                    confidential: vec![],
                    proof_id: None,
                    revealed: params.amount,
                })
            },
            ConfidentialTransferInputSelection::PreferRevealed => {
                let revealed_to_spend = cmp::min(src_vault.revealed_balance, params.amount);
                let confidential_to_spend = params.amount - revealed_to_spend;
                if confidential_to_spend.is_zero() {
                    info!(
                        target: LOG_TARGET,
                        "PreferRevealed: Spending {} revealed balance for transfer from {} to {}",
                        revealed_to_spend,
                        src_vault.address,
                        params.destination_public_key
                    );

                    return Ok(OutputsToSpend {
                        confidential: vec![],
                        proof_id: None,
                        revealed: revealed_to_spend,
                    });
                }

                let proof_id = self.outputs_api.add_proof(&src_vault.address)?;
                let (confidential_inputs, _) = self.outputs_api.lock_outputs_by_amount(
                    &src_vault.address,
                    confidential_to_spend,
                    proof_id,
                    params.is_dry_run,
                )?;
                let confidential_inputs = self
                    .outputs_api
                    .resolve_output_masks(confidential_inputs, key_manager::TRANSACTION_BRANCH)?;

                info!(
                    target: LOG_TARGET,
                    "PreferRevealed: Locked {} confidential inputs for transfer from {} to {}",
                    confidential_inputs.len(),
                    src_vault.address,
                    params.destination_public_key
                );

                Ok(OutputsToSpend {
                    confidential: confidential_inputs,
                    proof_id: Some(proof_id),
                    revealed: revealed_to_spend,
                })
            },
            ConfidentialTransferInputSelection::PreferConfidential => {
                let proof_id = self.outputs_api.add_proof(&src_vault.address)?;
                let (confidential_inputs, amount_locked) = self.outputs_api.lock_outputs_until_partial_amount(
                    &src_vault.address,
                    params.total_amount(),
                    proof_id,
                    params.is_dry_run,
                )?;

                let revealed_to_spend =
                    params
                        .total_amount()
                        .saturating_sub(amount_locked.try_into().map_err(|_| {
                            ConfidentialTransferApiError::InvalidParameter {
                                param: "transfer_param",
                                reason: "Attempt to spend more than Amount::MAX".to_string(),
                            }
                        })?);

                if src_vault.revealed_balance < revealed_to_spend {
                    return Err(ConfidentialTransferApiError::InsufficientFunds);
                }

                let confidential_inputs = self
                    .outputs_api
                    .resolve_output_masks(confidential_inputs, key_manager::TRANSACTION_BRANCH)?;

                Ok(OutputsToSpend {
                    confidential: confidential_inputs,
                    proof_id: Some(proof_id),
                    revealed: revealed_to_spend,
                })
            },
        }
    }

    async fn resolve_destination_account(
        &self,
        destination_pk: &PublicKey,
    ) -> Result<(VersionedSubstateId, bool), ConfidentialTransferApiError> {
        let account_component = new_account_address_from_parts(&ACCOUNT_TEMPLATE_ADDRESS, destination_pk);
        match self
            .substate_api
            .scan_for_substate(&account_component.into(), None)
            .await
            .optional()?
        {
            Some(ValidatorScanResult { address, .. }) => Ok((address, true)),
            None => Ok((
                VersionedSubstateId {
                    substate_id: account_component.into(),
                    version: 0,
                },
                false,
            )),
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn transfer(&self, params: TransferParams) -> Result<TransferOutput, ConfidentialTransferApiError> {
        let from_account = self.accounts_api.get_account_by_address(&params.from_account.into())?;
        let (to_account, dest_account_exists) =
            self.resolve_destination_account(&params.destination_public_key).await?;
        let from_account_address = from_account.address.as_component_address().unwrap();

        // Determine Transaction Inputs
        let mut inputs = Vec::new();

        if dest_account_exists {
            inputs.push(to_account.clone());
        }

        let account = self.accounts_api.get_account_by_address(&params.from_account.into())?;
        let account_substate = self.substate_api.get_substate(&params.from_account.into())?;
        inputs.push(account_substate.address);

        // Add all versioned account child addresses as inputs
        let child_addresses = self.substate_api.load_dependent_substates(&[&account.address])?;
        inputs.extend(child_addresses);

        let src_vault = self
            .accounts_api
            .get_vault_by_resource(&account.address, &params.resource_address)?;
        let src_vault_substate = self.substate_api.get_substate(&src_vault.address)?;
        inputs.push(src_vault_substate.address);

        // add the input for the resource address to be transferred
        let maybe_known_resource = self
            .substate_api
            .get_substate(&params.resource_address.into())
            .optional()?;
        let resource_substate = self
            .substate_api
            .scan_for_substate(
                &SubstateId::Resource(params.resource_address),
                maybe_known_resource.map(|r| r.address.version),
            )
            .await?;
        inputs.push(resource_substate.address.clone());

        if let Some(ref resource_address) = params.proof_from_resource {
            let maybe_known_resource = self.substate_api.get_substate(&(*resource_address).into()).optional()?;
            let resource_substate = self
                .substate_api
                .scan_for_substate(
                    &SubstateId::Resource(*resource_address),
                    maybe_known_resource.map(|r| r.address.version),
                )
                .await?;
            inputs.push(resource_substate.address.clone());
        }

        // Reserve and lock input funds
        let outputs_to_spend = self.resolved_inputs_for_transfer(&params).await?;

        // Generate outputs
        let output_mask = self.key_manager_api.next_key(key_manager::TRANSACTION_BRANCH)?;
        let (nonce, public_nonce) = PublicKey::random_keypair(&mut OsRng);

        let encrypted_data = self.crypto_api.encrypt_value_and_mask(
            params.amount.as_u64_checked().unwrap(),
            &output_mask.key,
            &params.destination_public_key,
            &nonce,
        )?;

        let resource_view_key = resource_substate
            .substate
            .as_resource()
            .ok_or_else(|| ConfidentialTransferApiError::UnexpectedIndexerResponse {
                details: format!(
                    "Expected indexer to return resource for address {}. It returned {}",
                    params.resource_address, resource_substate.address
                ),
            })?
            .view_key()
            .cloned();

        let output_statement = ConfidentialProofStatement {
            amount: params.confidential_amount(),
            mask: output_mask.key,
            sender_public_nonce: public_nonce,
            encrypted_data,
            minimum_value_promise: 0,
            reveal_amount: params.revealed_amount(),
            resource_view_key: resource_view_key.clone(),
        };

        let account_secret = self
            .key_manager_api
            .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;

        let change_amount = outputs_to_spend.total_amount() - params.amount;
        let maybe_change_statement = if change_amount.is_zero() {
            None
        } else {
            let change_mask = self.key_manager_api.next_key(key_manager::TRANSACTION_BRANCH)?;
            let (_, public_nonce) = PublicKey::random_keypair(&mut OsRng);
            let change_value = change_amount
                .as_u64_checked()
                .unwrap_or_else(|| panic!("BUG: Change out of range: {}", change_amount));

            let encrypted_data = self.crypto_api.encrypt_value_and_mask(
                change_value,
                &change_mask.key,
                &public_nonce,
                &account_secret.key,
            )?;

            if !params.is_dry_run {
                self.outputs_api.add_output(ConfidentialOutputModel {
                    account_address: account.address,
                    vault_address: src_vault.address,
                    commitment: get_commitment_factory().commit_value(&change_mask.key, change_value),
                    value: change_value,
                    sender_public_nonce: Some(public_nonce.clone()),
                    encryption_secret_key_index: account_secret.key_index,
                    encrypted_data: encrypted_data.clone(),
                    public_asset_tag: None,
                    status: OutputStatus::LockedUnconfirmed,
                    locked_by_proof: outputs_to_spend.proof_id,
                })?;
            }

            Some(ConfidentialProofStatement {
                amount: change_amount,
                mask: change_mask.key,
                sender_public_nonce: public_nonce,
                minimum_value_promise: 0,
                encrypted_data,
                reveal_amount: Amount::zero(),
                resource_view_key,
            })
        };

        let proof = self.crypto_api.generate_withdraw_proof(
            &outputs_to_spend.confidential,
            outputs_to_spend.revealed,
            &output_statement,
            maybe_change_statement.as_ref(),
        )?;

        // TODO: support paying fees from confidential outputs
        let mut builder =
            Transaction::builder().fee_transaction_pay_from_component(from_account_address, params.max_fee);

        if let Some(ref badge) = params.proof_from_resource {
            builder = builder
                .call_method(from_account_address, "create_proof_for_resource", args![badge])
                .put_last_instruction_output_on_workspace("proof");
        }

        builder = builder
            .call_method(from_account_address, "withdraw_confidential", args![
                params.resource_address,
                proof
            ])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(
                to_account.substate_id.as_component_address().unwrap(),
                "deposit",
                args![Workspace("bucket")],
            );

        if params.proof_from_resource.is_some() {
            builder = builder.drop_all_proofs_in_workspace();
        }

        let transaction = builder.sign(&account_secret.key).build();

        if let Some(proof_id) = outputs_to_spend.proof_id {
            self.outputs_api
                .proofs_set_transaction_hash(proof_id, *transaction.id())?;
        }

        Ok(TransferOutput { transaction, inputs })
    }
}

pub struct TransferOutput {
    pub transaction: Transaction,
    pub inputs: Vec<VersionedSubstateId>,
}

#[derive(Debug)]
pub struct TransferParams {
    /// Spend from this account
    pub from_account: ComponentAddress,
    /// Strategy for input selection
    pub input_selection: ConfidentialTransferInputSelection,
    /// Amount to spend to destination
    pub amount: Amount,
    /// Destination public key used to derive the destination account component
    pub destination_public_key: PublicKey,
    /// Address of the resource to transfer
    pub resource_address: ResourceAddress,
    /// Fee to lock for the transaction
    pub max_fee: Amount,
    /// If true, the output will contain only a revealed amount. Otherwise, only confidential amounts.
    pub output_to_revealed: bool,
    /// If some, instructions are added that create a access rule proof for this resource before calling withdraw
    pub proof_from_resource: Option<ResourceAddress>,
    /// Run as a dry run, no funds will be transferred if true
    pub is_dry_run: bool,
}

impl TransferParams {
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

impl TransferParams {
    pub fn total_amount(&self) -> Amount {
        self.amount + self.max_fee
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub enum ConfidentialTransferInputSelection {
    ConfidentialOnly,
    RevealedOnly,
    PreferRevealed,
    PreferConfidential,
}

#[derive(Debug)]
pub struct OutputsToSpend {
    pub confidential: Vec<ConfidentialOutputMaskAndValue>,
    pub proof_id: Option<ConfidentialProofId>,
    pub revealed: Amount,
}

impl OutputsToSpend {
    pub fn total_amount(&self) -> Amount {
        let confidential_amt = self.confidential.iter().map(|o| o.value).sum::<u64>();
        Amount::try_from(confidential_amt).unwrap() + self.revealed
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
}

impl IsNotFoundError for ConfidentialTransferApiError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, Self::StoreError(e) if e.is_not_found_error() )
    }
}
