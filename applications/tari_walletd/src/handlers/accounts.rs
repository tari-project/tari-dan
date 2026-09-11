//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, iter, path::Path, time::Duration};

use anyhow::anyhow;
use axum_extra::headers::authorization::Bearer;
use indexmap::{IndexMap, IndexSet};
use log::*;
use ootle_byte_type::{FromByteType, ToByteType};
use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey};
use tari_engine_types::{
    commit_result::RejectReason,
    component::derive_component_address_from_public_key,
    confidential::ClaimBurnOutputData,
    fees::FEE_ESTIMATE_ALLOWANCE,
    substate::SubstateId,
};
use tari_ootle_app_utilities::fee_tables::fee_rates_by_network;
use tari_ootle_common_types::{Epoch, SubstateRequirement, optional::Optional};
use tari_ootle_transaction::{Transaction, args};
use tari_ootle_wallet_crypto::{
    OutputWitness,
    StealthCryptoApiError,
    StealthInputWitness,
    StealthOutputWitness,
    WalletCryptoError,
    memo::Memo,
};
use tari_ootle_wallet_sdk::{
    apis::{
        confidential_transfer::ConfidentialTransferParams,
        stealth_outputs::{StealthOutputsApiError, TransferStatementParams},
        stealth_transfer::{InputsToSpend, StealthTransferParams, TransferOutput},
        substate::ValidatorScanResult,
    },
    models::{
        AccountWithAddress,
        KeyBranch,
        NewAccountData,
        StealthUtxoSpendKeyId,
        TransactionContext,
        TransactionContextKind,
        TransactionSubmittedEvent,
    },
};
use tari_ootle_wallet_sdk_services::transaction_service::TransactionServiceHandle;
use tari_ootle_walletd_client::{
    ComponentAddressOrName,
    permissions::{Crud, Permission},
    types::{
        AccountGetByKeyIndexRequest,
        AccountGetDefaultRequest,
        AccountGetRequest,
        AccountGetResponse,
        AccountInfo,
        AccountSetDefaultRequest,
        AccountSetDefaultResponse,
        AccountsAssociateStealthResourceRequest,
        AccountsAssociateStealthResourceResponse,
        AccountsCreateFreeTestCoinsRequest,
        AccountsCreateFreeTestCoinsResponse,
        AccountsCreateOrGetRequest,
        AccountsCreateOrGetResponse,
        AccountsCreateRequest,
        AccountsCreateResponse,
        AccountsCreateStealthTransferStatementRequest,
        AccountsCreateStealthTransferStatementResponse,
        AccountsGetBalanceChangesRequest,
        AccountsGetBalanceChangesResponse,
        AccountsGetBalancesRequest,
        AccountsGetBalancesResponse,
        AccountsListRequest,
        AccountsListResponse,
        AccountsRenameRequest,
        AccountsRenameResponse,
        AccountsTransferRequest,
        AccountsTransferResponse,
        BalanceEntry,
        ClaimBurnProof,
        ClaimBurnProofContents,
        ClaimBurnRequest,
        ClaimBurnResponse,
        ConfidentialTransferRequest,
        ConfidentialTransferResponse,
        StealthTransferRequest,
        StealthTransferResponse,
    },
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib_types::{
    Amount,
    ResourceType,
    constants::{
        STEALTH_TARI_RESOURCE_ADDRESS,
        TARI_TOKEN,
        XTR_FAUCET_AMOUNT,
        XTR_FAUCET_CLAIM_RESOURCE_ADDRESS,
        XTR_FAUCET_COMPONENT_ADDRESS,
        XTR_FAUCET_VAULT_ADDRESS,
    },
    stealth::SpendAuthorization,
};
use tokio::task;

use super::context::HandlerContext;
use crate::handlers::{
    auth::jwt::enforce_scopes,
    helpers::{
        complete_burn_proof_to_contents,
        faucet_already_claimed,
        general_error,
        get_account,
        get_account_by_key_index,
        get_account_or_default,
        get_account_with_inputs,
        invalid_params,
        invalid_request,
        not_found,
        transaction_rejected,
        validate_burn_proof_file_name,
        wait_for_result,
        wait_for_result_and_account,
    },
};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::handlers::transaction";

pub async fn handle_create(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsCreateRequest,
) -> Result<AccountsCreateResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Accounts(Crud::Create, None)])?;
    let sdk = context.wallet_sdk();
    let accounts_api = sdk.accounts_api();

    let set_as_default = req
        .is_default
        .map(Ok)
        .unwrap_or_else(|| accounts_api.any_accounts_exist().map(|b| !b))?;

    let owner_address = match req.key_index {
        Some(id) => sdk.key_manager_api().derive_account_address(id)?,
        None => sdk.key_manager_api().next_account_address()?,
    };

    let acc = accounts_api
        .create_account(req.account_name.as_deref(), set_as_default, owner_address)
        .map_err(|e| {
            if e.is_name_exists_error() {
                invalid_request(e)
            } else {
                general_error(e)
            }
        })?;

    info!(
        target: LOG_TARGET,
        "Created account: {acc}."
    );

    Ok(AccountsCreateResponse {
        account: acc.account,
        address: acc.address,
    })
}

pub async fn handle_create_or_get(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsCreateOrGetRequest,
) -> Result<AccountsCreateOrGetResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Accounts(Crud::Create, None)])?;
    let sdk = context.wallet_sdk();
    let accounts_api = sdk.accounts_api();

    let existing_account = match req.account {
        Some(ComponentAddressOrName::ComponentAddress(addr)) => {
            // In this case, we error if a specific address is specified
            let account = accounts_api
                .get_account_by_address(&addr)
                .optional()?
                .ok_or_else(|| not_found(format!("Account with address {addr} not found")))?;
            Some(account)
        },
        // If we cannot find an account with this name, we'll create one
        Some(ComponentAddressOrName::Name(ref name)) => accounts_api.get_account_by_name(name).optional()?,
        // If we cannot find an account with this key index, we'll create one
        None => req
            .key_index
            .map(|index| get_account_by_key_index(sdk, index).optional())
            .transpose()?
            .flatten(),
    };

    if let Some(account) = existing_account {
        info!(
            target: LOG_TARGET,
            "Account already exists: {account}."
        );
        return Ok(AccountsCreateOrGetResponse {
            account: account.account,
            address: account.address,
            created: false,
        });
    }

    let set_as_default = req
        .is_default
        .map(Ok)
        .unwrap_or_else(|| accounts_api.any_accounts_exist().map(|b| !b))?;

    let wallet_keys = match req.key_index {
        Some(id) => sdk.key_manager_api().derive_account_address(id)?,
        None => sdk.key_manager_api().next_account_address()?,
    };

    let acc = accounts_api
        .create_account(req.account.as_ref().and_then(|a| a.name()), set_as_default, wallet_keys)
        .map_err(|e| {
            if e.is_name_exists_error() {
                invalid_request(e)
            } else {
                general_error(e)
            }
        })?;

    info!(
        target: LOG_TARGET,
        "Created account: {acc}."
    );

    Ok(AccountsCreateOrGetResponse {
        account: acc.account,
        address: acc.address,
        created: true,
    })
}

pub async fn handle_set_default(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountSetDefaultRequest,
) -> Result<AccountSetDefaultResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Accounts(Crud::Update, None)])?;
    let sdk = context.wallet_sdk();
    let account = get_account(&req.account, &sdk.accounts_api())?;
    sdk.accounts_api().set_default_account(account.component_address())?;
    Ok(AccountSetDefaultResponse {})
}

pub async fn handle_rename(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsRenameRequest,
) -> Result<AccountsRenameResponse, anyhow::Error> {
    // Resolve before scope-check, but require a valid token first so the
    // resolve doesn't leak account existence to unauthenticated callers.
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk();
    let account = get_account(&req.account, &sdk.accounts_api())?;
    enforce_scopes(&granted, &[Permission::Accounts(
        Crud::Update,
        Some(*account.component_address()),
    )])?;
    sdk.accounts_api()
        .rename_account(account.component_address(), &req.new_name)?;
    Ok(AccountsRenameResponse {})
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsListRequest,
) -> Result<AccountsListResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Accounts(Crud::Read, None)])?;
    let sdk = context.wallet_sdk();
    let limit = usize::try_from(req.limit)
        .map_err(|e| invalid_params("limit", Some(&format!("limit overflowed usize: {}", e))))?;
    let offset = usize::try_from(req.offset)
        .map_err(|e| invalid_params("offset", Some(&format!("offset overflowed usize: {}", e))))?;
    let accounts_api = sdk.accounts_api();
    let accounts = accounts_api.get_many(offset, limit)?;
    let total = accounts_api.count()?;
    let accounts = accounts
        .into_iter()
        .map(|a| {
            let address = accounts_api.get_address_for_account(&a)?;
            Ok(AccountInfo {
                account: a,
                address: address.to_byte_type(),
            })
        })
        .collect::<Result<_, anyhow::Error>>()?;

    Ok(AccountsListResponse { accounts, total })
}

pub async fn handle_get_balances(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsGetBalancesRequest,
) -> Result<AccountsGetBalancesResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk();
    let account = get_account_or_default(req.account.as_ref(), &sdk.accounts_api())?;
    enforce_scopes(&granted, &[Permission::Accounts(
        Crud::Read,
        Some(*account.component_address()),
    )])?;
    if req.refresh {
        context
            .account_monitor()
            .refresh_account_with_utxos(*account.component_address())
            .await?;
    }
    let vaults = sdk.accounts_api().get_vaults_by_account(account.component_address())?;
    let stealth_outputs = sdk
        .stealth_outputs_api()
        .get_unspent_outputs_by_account(account.component_address(), false)?;

    let mut balances = Vec::with_capacity(vaults.len());
    let mut vaulted_resources = HashSet::new();
    for vault in vaults {
        let confidential_balance = if vault.resource_type.is_stealth() {
            let stealth_balance = stealth_outputs
                .iter()
                .filter(|o| o.resource_address == vault.resource_address)
                .map(|o| Amount::from(o.value))
                .sum::<Amount>();

            if stealth_balance.is_positive() {
                // If the vault has a confidential balance, we don't want to add it to the balances list
                // as it is already included in the vault's revealed balance.
                vaulted_resources.insert(vault.resource_address);
            }
            stealth_balance
        } else {
            vault.confidential_balance
        };

        balances.push(BalanceEntry {
            vault_address: Some(vault.id),
            resource_address: vault.resource_address,
            balance: vault.revealed_balance,
            resource_type: vault.resource_type,
            confidential_balance,
            token_symbol: vault.token_symbol,
            divisibility: vault.divisibility,
        })
    }

    let stealth_outputs = stealth_outputs
        .into_iter()
        .filter(|o| !vaulted_resources.contains(&o.resource_address))
        // NOTE: indexemap used to ensure a consistent order (HashMap causes UI to randomly switch positions for multiple stealth resources)
        .fold(IndexMap::new(), |mut acc, o| {
            acc.entry(o.resource_address)
                .and_modify(|v| *v += Amount::from(o.value))
                .or_insert(Amount::from(o.value));
            acc
        });

    let all_resources = sdk.resources_api().get_many(stealth_outputs.keys())?;

    for (resource_address, total_value) in stealth_outputs {
        let resource = all_resources.get(&resource_address);
        balances.push(BalanceEntry {
            vault_address: None,
            resource_address,
            balance: Amount::zero(),
            resource_type: ResourceType::Stealth,
            confidential_balance: total_value,
            // It's not guaranteed by the wallet that we know the resource, so instead of erroring, we'll return
            // something
            token_symbol: resource.as_ref().and_then(|r| r.token_symbol()).map(|s| s.to_owned()),
            divisibility: resource.as_ref().map(|r| r.divisibility()).unwrap_or(0),
        });
    }

    Ok(AccountsGetBalancesResponse {
        address: *account.component_address(),
        balances,
    })
}

pub async fn handle_get_balance_changes(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsGetBalanceChangesRequest,
) -> Result<AccountsGetBalanceChangesResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk();
    let account = get_account(&req.account, &sdk.accounts_api())
        .optional()?
        .ok_or_else(|| not_found(format!("Account '{}' not found", req.account)))?;
    enforce_scopes(&granted, &[Permission::Accounts(
        Crud::Read,
        Some(*account.component_address()),
    )])?;
    const MAX_BALANCE_CHANGE_LIMIT: usize = 200;
    let limit = usize::try_from(req.limit)
        .map_err(|e| invalid_params("limit", Some(&format!("limit overflowed usize: {e}"))))?
        .min(MAX_BALANCE_CHANGE_LIMIT);
    let offset = usize::try_from(req.offset)
        .map_err(|e| invalid_params("offset", Some(&format!("offset overflowed usize: {e}"))))?;
    let accounts_api = sdk.accounts_api();
    let page = accounts_api.get_balance_changes(
        account.component_address(),
        offset,
        limit,
        req.resource_address.as_ref(),
        req.transaction_id.as_ref(),
        req.source_type,
    )?;
    Ok(AccountsGetBalanceChangesResponse {
        changes: page.changes,
        total: page.total,
    })
}

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountGetRequest,
) -> Result<AccountGetResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk();
    let account = get_account(&req.name_or_address, &sdk.accounts_api())
        .optional()?
        .ok_or_else(|| {
            not_found(format!(
                "Account with name or address '{}' not found",
                req.name_or_address
            ))
        })?;
    enforce_scopes(&granted, &[Permission::Accounts(
        Crud::Read,
        Some(*account.component_address()),
    )])?;
    Ok(AccountGetResponse {
        account: account.account,
        address: account.address,
    })
}

pub async fn handle_get_by_key_index(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountGetByKeyIndexRequest,
) -> Result<AccountGetResponse, anyhow::Error> {
    // Resolve by key index then scope-check against the resolved address
    // so a scoped agent can fetch its own account this way.
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk();
    let account = get_account_by_key_index(sdk, req.key_index)
        .optional()?
        .ok_or_else(|| not_found(format!("Account with key index {} not found", req.key_index)))?;
    enforce_scopes(&granted, &[Permission::Accounts(
        Crud::Read,
        Some(*account.component_address()),
    )])?;
    Ok(AccountGetResponse {
        account: account.account,
        address: account.address,
    })
}

pub async fn handle_get_default(
    context: &HandlerContext,
    token: Option<&Bearer>,
    _req: AccountGetDefaultRequest,
) -> Result<AccountGetResponse, anyhow::Error> {
    // Resolve default then scope-check so a scoped agent can fetch the
    // default if the default happens to be its account.
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk();
    let account = get_account_or_default(None, &sdk.accounts_api())?;
    enforce_scopes(&granted, &[Permission::Accounts(
        Crud::Read,
        Some(*account.component_address()),
    )])?;
    Ok(AccountGetResponse {
        account: account.account,
        address: account.address,
    })
}

pub async fn handle_claim_burn(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ClaimBurnRequest,
) -> Result<ClaimBurnResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk();
    let network = sdk.network();

    let ClaimBurnRequest {
        account,
        claim_proof,
        max_fee,
        is_dry_run,
    } = req;

    let max_fee = max_fee.max(1);

    let accounts_api = sdk.accounts_api();
    let account = get_account(&account, &accounts_api)?;
    enforce_scopes(&granted, &[Permission::Transfer(
        Crud::Create,
        Some(*account.component_address()),
    )])?;

    // Capture the file name before resolving so we can mark it as claimed after submission
    let proof_file_name = match &claim_proof {
        ClaimBurnProof::FromFile { file_name } => Some(file_name.clone()),
        _ => None,
    };

    let proof_dir = context.config().get_burn_proof_dir(network);
    let proof_contents = resolve_claim_proof(&proof_dir, claim_proof).await.map_err(|e| {
        error!(target: LOG_TARGET, "Error resolving claim proof: {}", e);
        invalid_request(format!("Could not resolve claim proof: {e}"))
    })?;

    execute_claim_burn(
        sdk,
        context.transaction_service(),
        &account,
        proof_contents,
        max_fee,
        context.transaction_max_epoch().await?,
        is_dry_run,
        proof_file_name,
    )
    .await
}

/// Core claim burn logic: decrypts the burn proof, builds and signs the claim transaction,
/// and submits it (or performs a dry run). Shared between the interactive RPC handler
/// and the automatic background [`AutoClaimBurnService`].
#[allow(clippy::too_many_lines)]
pub(crate) async fn execute_claim_burn(
    sdk: &crate::WalletSdk,
    transaction_service: &TransactionServiceHandle,
    account: &AccountWithAddress,
    proof_contents: ClaimBurnProofContents,
    max_fee: u64,
    max_epoch: Epoch,
    is_dry_run: bool,
    proof_file_name: Option<String>,
) -> Result<ClaimBurnResponse, anyhow::Error> {
    let ClaimBurnProofContents {
        encrypted_data: claimed_encrypted_data,
        claim_proof,
    } = proof_contents;

    let account_owner_key_id = account
        .owner_key_id()
        .ok_or_else(|| invalid_params("account", Some("cannot claim burn to an account without an owner key")))?;

    let network = sdk.config_api().get_network()?;
    // We derive secrets directly here because claim burn is a unique case, making it difficult to use the higher
    // level stealth output api that takes care of keys but assumes that this is a regular transfer.
    let account_owner_key = sdk.key_manager_api().get_key(account_owner_key_id)?;

    // Get the sender_offset_public_key (R) and use it to create a DH with the account owner key
    let sender_offset_pub_key: RistrettoPublicKey = claim_proof
        .sender_offset_public_key
        .try_from_byte_type()
        .map_err(|e| invalid_params("claim_proof.sender_offset_public_key", Some(e)))?;

    // Stealth spend secret `s = H(R·p) + p`. The L1 ownership proof commits the burn to `C = s·G`,
    // so this is the only key that can sign the claim transaction and satisfy the spend condition
    // on the just-minted burn UTXO.
    let stealth_secret = sdk
        .stealth_crypto_api()
        .derive_burn_claim_stealth_secret(account_owner_key.secret(), &sender_offset_pub_key);
    let stealth_claim_pk = RistrettoPublicKey::from_secret_key(&stealth_secret).to_byte_type();

    if !sdk.stealth_crypto_api().validate_burn_claim_ownership_proof(
        network,
        &claim_proof.ownership_proof,
        &claim_proof.commitment,
        claim_proof.value,
        &stealth_claim_pk,
    ) {
        return Err(invalid_params(
            "claim_proof.ownership_proof",
            Some("ownership proof validation failed"),
        ));
    }

    info!(
        target: LOG_TARGET,
        "ℹ️ Signing claim burn for account {} (stealth claim pk: {})",
        account_owner_key.to_public_key().to_byte_type(),
        stealth_claim_pk,
    );

    let decrypted = sdk.stealth_crypto_api().decrypt_utxo_data(
        &claimed_encrypted_data,
        &claim_proof.commitment,
        account_owner_key.secret(),
        &sender_offset_pub_key,
        true,
    )?;

    let mask = sdk.key_manager_api().next_key(KeyBranch::StealthMask)?;

    let final_amount = decrypted
        .value()
        .checked_sub(max_fee)
        .ok_or_else(|| invalid_params("max_fee", Some("more fees paid than claimed amount")))?;

    if final_amount == 0 {
        return Err(invalid_params("max_fee", Some("fee equals or exceeds claimed amount")));
    }

    let (nonce, output_public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let account_owner = sdk.key_manager_api().get_public_key(account_owner_key_id)?;
    let view_only = sdk.key_manager_api().get_public_key(account.view_only_key_id())?;
    let memo = Memo::new_message("Burnt funds claimed from L1").expect("valid memo");

    let encrypted_data = sdk.stealth_crypto_api().encrypt_value_and_mask(
        final_amount,
        &mask.key,
        view_only.public_key(),
        &nonce,
        Some(&memo),
    )?;

    let tag = sdk.stealth_crypto_api().derive_stealth_output_tag(
        network,
        &nonce,
        view_only.public_key(),
        &STEALTH_TARI_RESOURCE_ADDRESS,
    );

    // Create stealth address - used during spend time
    let stealth_output_owner_public_key =
        sdk.stealth_crypto_api()
            .derive_stealth_owner_public_key(network, account_owner.public_key(), &nonce);

    let output_witness = StealthOutputWitness {
        witness: OutputWitness {
            amount: final_amount,
            mask: mask.key,
            sender_public_nonce: output_public_nonce.clone(),
            minimum_value_promise: 0,
            encrypted_data,
            resource_view_key: None,
        },
        auth: SpendAuthorization::Key(stealth_output_owner_public_key.to_byte_type()),
        tag,
    };

    // Package the secrets required to spend the claimed output
    let input = StealthInputWitness::new(decrypted.into_mask_and_value());

    let pay_fee_and_mint_output = sdk.stealth_crypto_api().generate_transfer_statement(
        iter::once(input),
        0,
        iter::once(&output_witness),
        max_fee,
    )?;
    // We'll create an output with the same encrypted data that was used on L1 burn. Note that this is not strictly
    // necessary. The engine will create the output with whatever you give it, so we could reencrypt.
    let output_data = ClaimBurnOutputData {
        encrypted_data: claimed_encrypted_data,
    };

    let transaction = Transaction::builder(network.as_byte(), max_epoch)
        .with_fee_instructions_builder(|fee_builder| {
            fee_builder
                // Mint the UTXO
                .claim_burn(claim_proof, output_data)
                // Transfer the UTXO to another UTXO with some revealed output for fees
                .stealth_transfer(TARI_TOKEN, pay_fee_and_mint_output)
                // Pay fee
                .put_last_instruction_output_on_workspace("fee")
                .pay_fee_from_bucket("fee")
        })
        .with_dry_run(is_dry_run)
        .finish();

    let transaction = sdk.signer_api().sign_with_explicit_key(&stealth_secret, transaction)?;

    if is_dry_run {
        let transaction_id = transaction.calculate_id();
        let result = transaction_service.submit_dry_run_transaction(transaction).await?;
        let required_fees = result.finalize.required_fees();
        return Ok(ClaimBurnResponse {
            transaction_id,
            required_fees: Some(required_fees),
            dry_run_result: Some(result),
        });
    }

    // Link the transaction to the account the burn is claimed into, and (if present) carry the proof
    // file name so the claim-burn monitor can track it.
    let mut tx_context = TransactionContext::with_accounts([*account.component_address()]);
    if let Some(file_name) = proof_file_name {
        tx_context = tx_context.with_kind(TransactionContextKind::ClaimBurn { file_name });
    }
    let tx_id = transaction_service
        .submit_transaction_with_opts(transaction, Some(tx_context), None)
        .await?;

    Ok(ClaimBurnResponse {
        transaction_id: tx_id,
        required_fees: None,
        dry_run_result: None,
    })
}

/// Burn proofs are small fixed-size JSON. Cap reads at 1 MiB so a caller can't point us
/// at `/dev/zero` (or a huge attacker-planted file) and exhaust memory.
const MAX_BURN_PROOF_BYTES: u64 = 1 << 20;

async fn resolve_claim_proof<P: AsRef<Path>>(
    base_path: P,
    proof: ClaimBurnProof,
) -> anyhow::Result<ClaimBurnProofContents> {
    match proof {
        ClaimBurnProof::Contents(contents) => Ok(*contents),
        ClaimBurnProof::FromFile { file_name } => {
            validate_burn_proof_file_name(&file_name)?;
            let path = base_path.as_ref().join(&file_name);
            let metadata = tokio::fs::metadata(&path)
                .await
                .map_err(|_| anyhow!("Burn proof file not found: {file_name}"))?;
            if metadata.len() > MAX_BURN_PROOF_BYTES {
                return Err(anyhow!("Burn proof file too large: {file_name}"));
            }
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|_| anyhow!("Burn proof file not found: {file_name}"))?;
            let proof = serde_json::from_slice(&bytes).map_err(|_| anyhow!("Invalid burn proof file: {file_name}"))?;
            complete_burn_proof_to_contents(proof)
        },
    }
}

/// Takes tXTR from the testnet faucet and deposits them into an existing account.
#[allow(clippy::too_many_lines)]
pub async fn handle_create_free_test_coins(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsCreateFreeTestCoinsRequest,
) -> Result<AccountsCreateFreeTestCoinsResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Admin])?;
    let sdk = context.wallet_sdk();
    let accounts_api = sdk.accounts_api();

    let AccountsCreateFreeTestCoinsRequest { account, max_fee } = req;
    // Fixed amount: always 1,000 TARI (matches the on-chain faucet template constant)
    let amount = Amount::from(XTR_FAUCET_AMOUNT);

    let max_fee = max_fee.max(1);

    let account = get_account(&account, &accounts_api)
        .optional()?
        .ok_or_else(|| not_found(format!("Account with name or address '{}' not found", account,)))?;

    let account_owner_key_id = account.owner_key_id().ok_or_else(|| {
        invalid_params(
            "account",
            Some("cannot create free test coins for an account without an owner key"),
        )
    })?;

    info!(
        target: LOG_TARGET,
        "💰️ Creating free test coins for account: {} with amount: {} and max fee: {}",
        account.account.component_address,
        amount,
        max_fee
    );

    let mut inputs = vec![
        SubstateRequirement::unversioned(XTR_FAUCET_COMPONENT_ADDRESS),
        SubstateRequirement::unversioned(XTR_FAUCET_VAULT_ADDRESS),
        SubstateRequirement::unversioned(XTR_FAUCET_CLAIM_RESOURCE_ADDRESS),
    ];

    if account.is_confirmed_on_chain() {
        info!(
            target: LOG_TARGET,
            "💰️ create free test coins: Account {} is on-chain",
            account.account.component_address
        );
        // Add account inputs
        let account_substate = sdk
            .substate_api()
            .get_substate(&account.account.component_address.into())?;
        inputs.push(account_substate.substate_id.into());

        // Add all versioned account child addresses as inputs
        let child_addresses = sdk
            .substate_api()
            .load_dependent_substates(&[&account.account.component_address.into()])?;
        info!(
            target: LOG_TARGET,
            "💰️ create free test coins: Loaded {} vaults for existing account: {}",
            child_addresses.len(),
            account
        );
        inputs.extend(child_addresses);
    } else {
        info!(
            target: LOG_TARGET,
            "💰️ create free test coins: Account {} is not on-chain, Will create it",
            account.account.component_address
        );
    }

    let transaction = context
        .transaction_builder()
        .await?
        .with_fee_instructions_builder(|fee_builder| {
            fee_builder
                .create_account(*account.address.account_public_key())
                .put_last_instruction_output_on_workspace("new_account")
                .call_method(XTR_FAUCET_COMPONENT_ADDRESS, "take", args![Workspace("new_account")])
                .call_method("new_account", "pay_fee", args![max_fee])
        })
        .with_inputs(inputs.into_iter().map(|input| input.into_unversioned()))
        .finish();

    let transaction = sdk.signer_api().sign(account_owner_key_id, transaction)?;

    info!(
        target: LOG_TARGET,
        "💰️ create free test coins: Submitting transaction {} for account: {}",
        transaction.calculate_id(),
        account.account,
    );

    let mut events = context.notifier().subscribe();
    // Always link the transaction to the funded account. The NewAccount kind is only attached once the
    // account is confirmed on-chain (unchanged behaviour for the account monitor).
    let mut tx_context = TransactionContext::with_accounts([*account.component_address()]);
    if account.is_confirmed_on_chain() {
        tx_context = tx_context.with_kind(TransactionContextKind::NewAccount(NewAccountData {
            address: *account.component_address(),
        }));
    }
    let tx_id = context
        .transaction_service()
        .submit_transaction_with_opts(transaction, Some(tx_context), None)
        .await?;

    // Wait for the monitor to pick up the new or updated account
    let (finalized, _) = wait_for_result_and_account(&mut events, &tx_id, account.component_address()).await?;
    if let Some(reason) = finalized.finalize.any_reject() {
        return match reason {
            RejectReason::ExecutionFailure(reason) => {
                if reason.contains("Duplicate NFT token id") {
                    return Err(faucet_already_claimed());
                }
                Err(transaction_rejected(reason))
            },
            // TODO: consensus can emit failed to lock inputs when an output fails to lock and vice versa because it
            // locks them together in some cases. so we take both as meaning already claimed
            RejectReason::FailedToLockOutputs(reason) | RejectReason::FailedToLockInputs(reason) => {
                if reason.contains("is already UP and conflicts with an existing output") {
                    return Err(faucet_already_claimed());
                }
                Err(transaction_rejected(reason))
            },
            _ => Err(transaction_rejected(reason)),
        };
    }

    info!(
        target: LOG_TARGET,
        "💰️ create free test coins: Transaction {} finalized for account: {}",
        tx_id,
        account.account,
    );

    // Refresh the account
    let account = accounts_api
        .get_account_by_address(account.component_address())
        .optional()?
        .ok_or_else(|| {
            not_found(format!(
                "Account with address '{}' not found",
                account.component_address()
            ))
        })?;

    Ok(AccountsCreateFreeTestCoinsResponse {
        account: account.account,
        transaction_id: tx_id,
        amount,
        fee: max_fee,
        result: finalized.finalize,
        address: account.address,
    })
}

#[allow(clippy::too_many_lines)]
pub async fn handle_transfer(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsTransferRequest,
) -> Result<AccountsTransferResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk().clone();

    let (account, mut inputs) = get_account_with_inputs(req.account.as_ref(), &sdk)?;
    enforce_scopes(&granted, &[Permission::Transfer(
        Crud::Create,
        Some(*account.component_address()),
    )])?;

    let account_owner_key_id = account
        .owner_key_id()
        .ok_or_else(|| invalid_params("account", Some("cannot transfer from an account without an owner key")))?;

    // get the source account component address
    let source_account_address = *account.component_address();

    // add the input for the source account vault substate
    let src_vault = sdk
        .accounts_api()
        .get_vault_by_resource(&source_account_address, &req.resource_address)?;
    let src_vault_substate = sdk.substate_api().get_substate(&src_vault.id.into())?;
    inputs.insert(src_vault_substate.substate_id.into());

    let resource_substate_address = SubstateRequirement::unversioned(src_vault.resource_address);
    inputs.insert(resource_substate_address.clone());

    let destination_account_address =
        derive_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &req.destination_public_key);
    let existing_dest_account = sdk
        .substate_api()
        .fetch_substate_from_network(&SubstateId::Component(destination_account_address), None)
        .await
        .optional()?;

    let builder = context
        .transaction_builder()
        .await?
        .create_account(req.destination_public_key);

    if let Some(ValidatorScanResult { id: address, substate }) = existing_dest_account {
        inputs.insert(address.into());

        // Figure out which vault to add as an input
        let Some(component) = substate.component() else {
            return Err(anyhow::anyhow!(
                "The destination account {} is not a component. This is unexpected.",
                destination_account_address
            ));
        };
        let indexed = component.body.to_indexed_well_known_types()?;

        let mut found_dest_vault = None;
        for vault_id in indexed.vault_ids() {
            // Local vault?
            match sdk.accounts_api().get_vault(vault_id).optional()? {
                Some(vault) => {
                    if vault.resource_address != src_vault.resource_address {
                        // Continue searching for a vault for the resource address
                        continue;
                    }
                    // Found it - we're sending to our own vault
                    found_dest_vault = Some(*vault_id);
                    break;
                },
                None => {
                    // TODO(perf): slow with lots of vaults
                    let vault = sdk
                        .substate_api()
                        .fetch_substate_from_network(&SubstateId::Vault(*vault_id), None)
                        .await
                        .optional()?;

                    let Some(vault) = vault.and_then(|scan| scan.substate.into_vault()) else {
                        warn!(
                            target: LOG_TARGET,
                            "❓️ The destination account {destination_account_address} contains a vault {vault_id} that was not found. This is unexpected.",
                        );
                        continue;
                    };

                    if *vault.resource_address() != src_vault.resource_address {
                        // Continue searching for a vault for the resource address
                        continue;
                    }

                    // Found it
                    found_dest_vault = Some(*vault_id);
                },
            }
        }

        if let Some(found) = found_dest_vault {
            inputs.insert(SubstateRequirement::unversioned(found));
        }
    }

    // build the transaction
    let max_fee = req.max_fee.max(1);

    let transaction = builder
        .with_dry_run(req.dry_run)
        .pay_fee_from_component(source_account_address, max_fee)
        .then(|builder| {
            if let Some(ref badge) = req.proof_from_badge_resource {
                // If we are creating a proof for a badge resource, we need to create the proof first
                builder
                    .call_method(source_account_address, "create_proof_for_resource", args![badge])
                    .put_last_instruction_output_on_workspace("proof")
            } else {
                builder
            }
        })
        .call_method(source_account_address, "withdraw", args![
            req.resource_address,
            req.amount
        ])
        .put_last_instruction_output_on_workspace("bucket")
        .call_method(destination_account_address, "deposit", args![Workspace("bucket")])
        .then(|builder| {
            if req.proof_from_badge_resource.is_some() {
                builder.drop_all_proofs_in_workspace()
            } else {
                builder
            }
        })
        .with_inputs(inputs.into_iter().map(|req| req.into_unversioned()))
        .finish();

    let transaction = sdk.signer_api().sign(account_owner_key_id, transaction)?;

    // If dry run we can return the result immediately
    if req.dry_run {
        let transaction_id = transaction.calculate_id();
        let _execute_result = context
            .transaction_service()
            .submit_dry_run_transaction(transaction)
            .await?;
        return Ok(AccountsTransferResponse { transaction_id });
    }

    // Otherwise submit and wait for a result. Link the source account, and the destination too if it
    // is one of the wallet's own accounts (so a self-transfer shows under both).
    let mut linked_accounts = vec![source_account_address];
    if sdk
        .accounts_api()
        .get_account_by_address(&destination_account_address)
        .optional()?
        .is_some()
    {
        linked_accounts.push(destination_account_address);
    }
    let mut events = context.notifier().subscribe();
    let tx_id = context
        .transaction_service()
        .submit_transaction_with_opts(
            transaction,
            Some(TransactionContext::with_accounts(linked_accounts)),
            None,
        )
        .await?;

    let finalized = wait_for_result(&mut events, tx_id).await?;

    if let Some(reject) = finalized.finalize.result.fee_reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.finalize.any_reject() {
        return Err(anyhow::anyhow!(
            "Fee transaction succeeded (fees charged) however the transaction failed: {}",
            reason
        ));
    }
    info!(
        target: LOG_TARGET,
        "✅ Transfer transaction {} finalized. Fee: {}",
        finalized.transaction_id,
        finalized.final_fee
    );

    Ok(AccountsTransferResponse { transaction_id: tx_id })
}

pub async fn handle_confidential_transfer(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ConfidentialTransferRequest,
) -> Result<ConfidentialTransferResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk().clone();
    let notifier = context.notifier().clone();

    if req.amount.is_negative() {
        return Err(invalid_params("amount", Some("must be positive")));
    }
    let account = get_account_or_default(req.account.as_ref(), &sdk.accounts_api())?;
    enforce_scopes(&granted, &[Permission::Transfer(
        Crud::Create,
        Some(*account.component_address()),
    )])?;

    let transaction_service = context.transaction_service().clone();
    let max_epoch = context.transaction_max_epoch().await?;

    // Spawn here is to prevent the async block from being aborted if the caller aborts the request early as this can
    // cause funds to remain locked indefinitely.
    task::spawn(async move {
        let source_account_address = *account.component_address();
        // Tag the destination too if it is one of the wallet's own accounts (resolved before the address
        // is consumed by the transfer below).
        let owned_destination = sdk
            .accounts_api()
            .get_account_by_public_key(req.destination_address.account_public_key())
            .optional()?
            .map(|a| a.account.component_address);

        let transfer = sdk
            .confidential_transfer_api()
            .transfer(ConfidentialTransferParams {
                max_epoch,
                from_account: source_account_address,
                input_selection: req.input_selection,
                amount: req.amount,
                destination_address: req.destination_address,
                resource_address: req.resource_address,
                max_fee: req.max_fee.max(1),
                output_to_revealed: req.output_to_revealed,
                proof_from_resource: req.proof_from_badge_resource,
                memo: req.output_memo,
                is_dry_run: req.dry_run,
            })
            .await?;

        if req.dry_run {
            let transaction_id = transfer.transaction.calculate_id();
            let _exec_result = transaction_service
                .submit_dry_run_transaction(transfer.transaction)
                .await?;
            return Ok(ConfidentialTransferResponse { transaction_id });
        }

        let mut linked_accounts = vec![source_account_address];
        linked_accounts.extend(owned_destination);
        let tx_id = transaction_service
            .submit_transaction_with_opts(
                transfer.transaction,
                Some(TransactionContext::with_accounts(linked_accounts)),
                None,
            )
            .await?;

        notifier.notify(TransactionSubmittedEvent {
            transaction_id: tx_id,
            context: None,
        });

        Ok(ConfidentialTransferResponse { transaction_id: tx_id })
    })
    .await?
}

#[allow(clippy::too_many_lines)]
pub async fn handle_stealth_transfer(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: StealthTransferRequest,
) -> Result<StealthTransferResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk().clone();
    let network = sdk.sdk_config().network;
    let notifier = context.notifier().clone();
    let owner_account = get_account(&req.owner_account, &sdk.accounts_api())?;
    enforce_scopes(&granted, &[Permission::Transfer(
        Crud::Create,
        Some(*owner_account.component_address()),
    )])?;
    if owner_account.owner_key_id().is_none() {
        return Err(invalid_params(
            "owner_account",
            Some("cannot transfer from an account without an owner key"),
        ));
    };

    // The sender's own address keys (account public key + view public key), used when an output opts in to
    // attaching the sender address as its memo. Network is omitted; the recipient implies it from its own wallet.
    let owner_addr = owner_account.address();
    let sender_account_key: [u8; 32] = owner_addr
        .account_public_key()
        .as_bytes()
        .try_into()
        .map_err(|_| invalid_params("owner_account", Some("invalid account public key length")))?;
    let sender_view_key: [u8; 32] = owner_addr
        .view_only_key()
        .as_bytes()
        .try_into()
        .map_err(|_| invalid_params("owner_account", Some("invalid view public key length")))?;
    let owner_fallback_pay_ref: Vec<u8> = owner_addr.pay_ref().map(|p| p.as_bytes().to_vec()).unwrap_or_default();

    // Link the source account, plus any recipients that are the wallet's own accounts. Resolved before
    // `req.transfers` is consumed below and before `owner_account` is moved into the transfer task.
    let mut linked_accounts = vec![*owner_account.component_address()];
    for transfer in &req.transfers {
        if let Some(acc) = sdk
            .accounts_api()
            .get_account_by_public_key(transfer.destination_address.account_public_key())
            .optional()?
        {
            linked_accounts.push(acc.account.component_address);
        }
    }

    let outputs = req
        .transfers
        .into_iter()
        .map(|transfer| {
            // Resolve the effective pay reference for this transfer. The explicit `pay_ref` request field wins
            // when present; an explicit empty string disables any pay-ref. When the field is unset, fall back to
            // the destination address's bech32-embedded pay-ref so non-frontend callers keep the historical
            // behaviour.
            let effective_pay_ref: Option<Vec<u8>> = match transfer.pay_ref.as_deref() {
                Some("") => None,
                Some(s) => Some(s.as_bytes().to_vec()),
                None => transfer.destination_address.pay_ref().map(|p| p.as_bytes().to_vec()),
            };

            if transfer.attach_sender_address {
                let pay_ref_bytes = effective_pay_ref
                    .as_deref()
                    .unwrap_or(owner_fallback_pay_ref.as_slice());
                let memo = Memo::new_sender_address(sender_account_key, sender_view_key, pay_ref_bytes)
                    .ok_or_else(|| invalid_params("pay_ref", Some("pay reference too long (max 64 bytes)")))?;
                return Ok(TransferOutput {
                    address: transfer.destination_address,
                    blinded_amount: transfer.blinded_output_amount,
                    revealed_amount: transfer.revealed_output_amount,
                    memo: Some(memo),
                    pay_to: transfer.pay_to,
                });
            }

            match effective_pay_ref {
                Some(pay_ref) => {
                    let memo = transfer.output_memo.unwrap_or_else(|| Memo::new_message("").unwrap());
                    if memo.as_pay_ref().is_some() {
                        warn!(
                            target: LOG_TARGET,
                            "❗️ Overwriting existing pay ref in memo for transfer to address {}",
                            transfer.destination_address
                        );
                    }

                    let memo_bytes = memo
                        .as_memo_message()
                        .map(|s| s.as_bytes())
                        .or_else(|| memo.as_memo_bytes())
                        .ok_or_else(|| invalid_params("pay_ref", Some("can only include pay ref in message memo")))?;
                    let memo = Memo::new_pay_ref_and_bytes_truncate(&pay_ref, memo_bytes)
                        .ok_or_else(|| invalid_params("pay_ref", Some("pay reference too long (max 64 bytes)")))?;

                    Ok(TransferOutput {
                        address: transfer.destination_address,
                        blinded_amount: transfer.blinded_output_amount,
                        revealed_amount: transfer.revealed_output_amount,
                        memo: Some(memo),
                        pay_to: transfer.pay_to,
                    })
                },
                None => Ok(TransferOutput {
                    address: transfer.destination_address,
                    blinded_amount: transfer.blinded_output_amount,
                    revealed_amount: transfer.revealed_output_amount,
                    memo: transfer.output_memo,
                    pay_to: transfer.pay_to,
                }),
            }
        })
        .collect::<anyhow::Result<_>>()?;

    let mut params = StealthTransferParams {
        max_epoch: context.transaction_max_epoch().await?,
        fee_params: req.fee_params,
        input_selection: req.input_selection,
        resource_address: req.resource_address,
        max_fee: req.max_fee,
        badge_usage: req.badge_usage,
        outputs,
        is_dry_run: req.dry_run,
    };
    if let Err(err) = params.validate(network) {
        return Err(invalid_params("params", Some(err)));
    }

    let transaction_service = context.transaction_service().clone();

    // Spawn here is to prevent the async block from being aborted if the caller aborts the request early as this can
    // cause funds to remain locked indefinitely.
    task::spawn(async move {
        // A dry run's estimate is only worth anything if it describes the transaction that will be
        // submitted, and the fee is an input to that: input selection targets `amount + max_fee`, so
        // the fee decides which UTXOs are spent and whether any change is left over. A change output
        // is another stealth output, and another output is another `PER_OUTPUT` of verification —
        // enough that an estimate taken at the caller's guessed fee prices a shape the real
        // submission will not have.
        //
        // Settling that costs nothing when the shape is one the static estimate prices: rebuild
        // locally until a build pays for itself, and spend a single round trip confirming it.
        // Otherwise fall back to settling over the wire — rebuild at each figure reported until
        // building at it needs no more than it, then answer with the run that named it, which is a
        // figure the caller can submit at.
        let fee_rates = fee_rates_by_network(network);
        let mut candidate: Option<StealthTransferResponse> = None;
        let mut rounds = 0usize;
        let mut static_rounds = 0usize;
        // Whether the static estimate is still trusted for this request. It is dropped for good the
        // first time a dry run disagrees with it: past that the wire rounds settle the fee alone.
        let mut settle_statically = true;

        loop {
            let max_fee_this_round = params.max_fee;
            let build = sdk
                .stealth_transfer_api()
                .transfer(owner_account.clone(), params.clone())
                .await;
            // Only an estimate names a round, and only an estimate raises the fee between builds:
            // a later round widens the selection target to `amount + max_fee`, so it can exhaust an
            // account that funded the earlier one, and the failure needs to say which fee did it.
            // The cause is interpolated rather than added as context: `anyhow::Error` renders only
            // its outermost layer under `{}`, which is what the caller and the log see.
            let (lock, transfer) = if req.dry_run {
                build.map_err(|e| {
                    anyhow!(
                        "building the transfer at a max fee of {max_fee_this_round} (fee estimate round {}): {e}",
                        rounds + 1
                    )
                })?
            } else {
                build?
            };

            let transaction = transfer.transaction;
            let main_pk = transfer.main_signer.public_key().to_byte_type();

            // Signer api which sign transaction types that require the seal signer public key
            let main_signer = sdk.signer_api().with_context(&main_pk);
            // Add additional signature if needed
            let transaction = match transfer.additional_signer.as_ref() {
                Some(s) => main_signer.sign(s.key_id, transaction)?,
                None => transaction.finish(),
            };

            // Add required UTXO spend key signatures
            let transaction = transfer
                .utxo_spend_keys
                .iter()
                .try_fold(transaction, |tx, key| main_signer.sign_with_stealth_key(key, tx))?;

            // Sign and seal the final transaction
            let transaction = sdk.signer_api().sign(transfer.main_signer.key_id, transaction)?;

            if req.dry_run {
                // Price the shape this build produced and rebuild until the price agrees with the
                // fee the build was made at. Rebuilding costs local proof generation and nothing
                // else, so the fixed point is reached without a round trip.
                //
                // Both directions matter. An estimate above the fee says the shape cannot pay for
                // itself, so the next build selects differently and dry-running this one would
                // measure a transaction that is about to change. An estimate below it says the
                // caller's `max_fee` is a ceiling rather than a price, and answering with the
                // ceiling would have them reveal — unrefundably — far more than the transfer costs.
                let mut settled_statically = false;
                if let Some(shape) = transfer.statically_priced_shape.filter(|_| settle_statically) {
                    let estimate = shape
                        .with_transaction_weight(transaction.calculate_transaction_weight().as_u64())
                        .estimate_fee(&fee_rates)
                        .saturating_add(FEE_ESTIMATE_ALLOWANCE);
                    if estimate != params.max_fee && static_rounds < MAX_STATIC_SETTLE_ROUNDS {
                        params.max_fee = estimate;
                        static_rounds += 1;
                        // No round named this fee, so an earlier round's answer no longer describes
                        // the build about to be made at it.
                        candidate = None;
                        lock.release();
                        continue;
                    }
                    // Out of static rounds without agreement, the fee still stands if it covers the
                    // estimate; if it does not, the rounds below settle it over the wire.
                    settled_statically = estimate <= params.max_fee;
                }

                // Release the lock immediately as dry run does not submit the transaction
                // TODO: maybe transfer() should not lock the outputs if it's a dry run
                lock.release();
                // A statically settled fee is the figure to report, not the lower one the run turns
                // out to need: input selection targets `amount + max_fee`, so a submission at
                // anything other than a fee some build was made at picks a different set of UTXOs
                // and prices a different transaction. The difference is the estimate's own margin,
                // single-digit microtari, and it is what buys the settlement in a single round trip.
                let settled_fee = settled_statically.then_some(params.max_fee);
                let result = transaction_service
                    .submit_dry_run_transaction_settled_at(transaction, settled_fee)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Dry run transaction failed at a max fee of {max_fee_this_round} (fee estimate round {}): \
                             {e}",
                            rounds + 1
                        )
                    })?;

                let required_fees = result.finalize.required_fees();
                // What this round's shape actually costs, without the estimate allowance that
                // `required_fees` carries on top. That allowance is the caller's margin against the
                // metering drift a wider `max_fee` causes; testing against it here would treat a fee
                // that pays in full as insufficient and spend another round chasing it.
                let charged = result.finalize.charged_fees();
                let response = StealthTransferResponse {
                    transaction_id: result.finalize.transaction_hash.into(),
                };
                rounds += 1;

                // The estimator priced under the engine — the one thing `stealth_fee_estimate.rs`
                // exists to prevent. This round's row records the fee it was built at, which the run
                // has just shown insufficient, so that row must not become the answer: drop the
                // static estimate for the rest of the request and let the wire rounds settle, as
                // they did before there was an estimate.
                if settled_statically && charged > params.max_fee {
                    warn!(
                        target: LOG_TARGET,
                        "❗️ Static stealth fee estimate came in under the charge (built at {} but cost {charged}); \
                         settling over the wire",
                        params.max_fee
                    );
                    settle_statically = false;
                    candidate = None;
                    params.max_fee = required_fees;
                    continue;
                }

                match next_fee_estimate_step(
                    params.max_fee,
                    charged,
                    required_fees,
                    candidate.is_some() || settled_statically,
                    rounds,
                ) {
                    FeeEstimateStep::Settle => {
                        // A statically settled fee was confirmed by this very round, so this round's
                        // response is the one that names it. Otherwise the answer is the round that
                        // named the fee this one was built at.
                        if settled_statically {
                            return Ok(response);
                        }
                        return Ok(candidate.expect("BUG: Settle needs a candidate or a static settle"));
                    },
                    FeeEstimateStep::GiveUp => {
                        warn!(
                            target: LOG_TARGET,
                            "Stealth transfer fee estimate did not settle in {MAX_FEE_ESTIMATE_ROUNDS} rounds (built \
                             at {} but cost {charged})",
                            params.max_fee
                        );
                        // A statically settled round either settles or is diverted above, so this
                        // response records `required_fees` — a figure above what the round cost —
                        // rather than a settled fee the run disagreed with.
                        return Ok(response);
                    },
                    FeeEstimateStep::Retry { max_fee } => {
                        params.max_fee = max_fee;
                        candidate = Some(response);
                        continue;
                    },
                }
            }

            let tx_id = transaction_service
                .submit_transaction_with_opts(
                    transaction,
                    Some(TransactionContext::with_accounts(linked_accounts)),
                    Some(lock.id()),
                )
                .await
                .map_err(|e| anyhow!("Transaction failed to submit: {e}"))?;

            // Transaction submitted, we're home free, make sure to allow the lock to persist past this call.
            // The wallet will monitor the transaction and release the lock when it's finalized.
            lock.keep_locked();

            notifier.notify(TransactionSubmittedEvent {
                transaction_id: tx_id,
                context: None,
            });

            return Ok(StealthTransferResponse { transaction_id: tx_id });
        }
    })
    .await?
}

/// How many times the static estimate may rebuild before giving up and letting the dry-run rounds
/// settle the fee instead.
///
/// Two reach the fixed point in the common case — a build at the caller's fee, then a build at what
/// that shape prices. The rest is headroom, and a bound on a fixed point that need not exist:
/// selection is branch-and-bound over the total value selected, so the price of the shape a fee
/// produces does not move monotonically with that fee, and two fees can each price to the other.
/// Such a cycle costs the whole allowance of rebuilds — tens of milliseconds of range-proof
/// generation, still well inside a single round trip — and then settles over the wire as before.
const MAX_STATIC_SETTLE_ROUNDS: usize = 4;

/// How many times a dry run may build before answering with whatever figure it reached. Three
/// settle the common case — the caller's guessed fee, the shape that fee produces, and a build at
/// the figure that shape reported, which confirms it — and the rest is headroom for a selection
/// that keeps growing as the fee rises.
const MAX_FEE_ESTIMATE_ROUNDS: usize = 5;

/// What a dry-run estimation round decides to do next.
#[derive(Debug, PartialEq, Eq)]
enum FeeEstimateStep {
    /// The fee this round was built at covers the shape it produced, so the figure that named that
    /// fee is an answer the caller can submit at.
    Settle,
    /// Build again at this fee.
    Retry { max_fee: u64 },
    /// Out of rounds; answer with what this round reached.
    GiveUp,
}

/// Decides what an estimation round leads to, given what the round was `built_at`, what the shape it
/// produced was `charged`, the `required` figure it reports, whether an earlier round named
/// `built_at`, and how many rounds have run.
///
/// A round settles only against a `verified` fee: a figure is worth reporting once a build at it has
/// been shown to cover itself. A dry-run round shows that for the figure the round before it named,
/// and the static estimate shows it without a round trip. Unverified, the fee under test is the
/// caller's guess.
fn next_fee_estimate_step(
    built_at: u64,
    charged: u64,
    required: u64,
    verified: bool,
    rounds_taken: usize,
) -> FeeEstimateStep {
    if verified && charged <= built_at {
        return FeeEstimateStep::Settle;
    }
    if rounds_taken >= MAX_FEE_ESTIMATE_ROUNDS {
        return FeeEstimateStep::GiveUp;
    }
    FeeEstimateStep::Retry { max_fee: required }
}

#[allow(clippy::too_many_lines)]
pub async fn handle_create_stealth_transfer_statement(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsCreateStealthTransferStatementRequest,
) -> Result<AccountsCreateStealthTransferStatementResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk().clone();
    if req.requests.is_empty() {
        return Err(invalid_params(
            "requests",
            Some("at least one transfer request must be provided"),
        ));
    }

    if req.requests.len() > 16 {
        return Err(invalid_params(
            "requests",
            Some("a maximum of 16 transfer requests can be processed at once"),
        ));
    }

    let mut required_signers = HashSet::new();
    let mut utxo_signers = IndexSet::new();
    let lock = sdk.locks_api().create_lock_with_timeout(Duration::from_secs(5 * 60))?;
    let mut statements = Vec::with_capacity(req.requests.len());
    for req in req.requests {
        let sender_account = get_account(&req.sender_account, &sdk.accounts_api())?;
        enforce_scopes(&granted, &[Permission::Transfer(
            Crud::Create,
            Some(*sender_account.component_address()),
        )])?;
        let Some(sender_key_id) = sender_account.owner_key_id() else {
            return Err(invalid_params(
                "owner_account",
                Some("cannot transfer from an account without an owner key"),
            ));
        };

        let resource = sdk.substate_api().fetch_resource(req.resource_address).await?;

        if !resource.resource_type().is_stealth() {
            return Err(invalid_params(
                "resource_address",
                Some(format!(
                    "Resource is not a stealth resource (type: {})",
                    resource.resource_type()
                )),
            ));
        }

        // Checked before any input is locked: a malformed or silently-discarded `pay_to` is a request error, and
        // rejecting it here avoids taking a lock this request can never use.
        for output in &req.outputs {
            output
                .validate_pay_to()
                .map_err(|reason| invalid_params("pay_to", Some(reason)))?;
        }

        let amount_to_spend = req.total_output_amount();

        let inputs = if let Some(utxo_addresses) = req.input_selection.as_specific() {
            // Exact stealth inputs named by the caller: resolve, validate and lock precisely these UTXOs,
            // preserving the requested order. They contribute no revealed amount. Naming exact UTXOs (and thus
            // learning their decrypted totals through the balance check) requires scoped stealth-UTXO read
            // authority in addition to the transfer-create scope enforced above.
            enforce_scopes(&granted, &[Permission::StealthUtxos(
                Crud::Read,
                Some(*sender_account.component_address()),
            )])?;
            let inputs = sdk
                .stealth_outputs_api()
                .lock_specific_outputs(
                    sender_account.component_address(),
                    &req.resource_address,
                    lock.id(),
                    utxo_addresses,
                )
                .map_err(map_specific_selection_error)?;
            Some(InputsToSpend {
                inputs,
                revealed: Amount::zero(),
            })
        } else {
            req.input_selection
                .as_selection()
                .map(|sel| {
                    sdk.stealth_transfer_api().lock_inputs_for_transfer(
                        lock.id(),
                        sender_account.component_address(),
                        req.resource_address,
                        amount_to_spend,
                        sel,
                    )
                })
                .transpose()?
        };

        let must_sign_with_account_key = inputs.as_ref().is_some_and(|i| i.revealed.is_positive());
        let signing_key_id = if must_sign_with_account_key {
            sender_key_id
        } else {
            sdk.key_manager_api().next_derived_key_id(KeyBranch::Nonce)?.into()
        };

        let output_revealed_amount = req.outputs.iter().map(|o| o.revealed_amount).sum();
        let outputs = req
            .outputs
            .iter()
            .filter(|o| o.blinded_amount > 0)
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        let statement = sdk
            .stealth_outputs_api()
            .generate_transfer_statement(TransferStatementParams {
                view_only_key_id: sender_account.view_only_key_id(),
                resource_address: &req.resource_address,
                resource_view_key: resource
                    .to_view_key_public_key()
                    .map_err(|e| anyhow!("Failed to decode resource public view key: {e}"))?,
                inputs: inputs.as_ref().map(|i| i.inputs.as_slice()).unwrap_or(&[]),
                input_revealed_amount: req
                    .input_selection
                    .as_from_bucket()
                    .unwrap_or(Amount::zero())
                    .checked_add(inputs.as_ref().map(|i| i.revealed).unwrap_or(Amount::zero()))
                    .ok_or_else(|| {
                        invalid_params(
                            "input_revealed_amount",
                            Some("input revealed amount overflowed or was negative"),
                        )
                    })?,
                outputs,
                output_revealed_amount,
            })
            .map_err(map_statement_construction_error)?;

        utxo_signers.extend(inputs.iter().flat_map(|i| &i.inputs).map(|i| StealthUtxoSpendKeyId {
            account_key_id: sender_key_id,
            public_nonce: i.public_nonce,
        }));

        required_signers.insert(signing_key_id);
        statements.push(statement);
    }

    // Return without unlocking the outputs
    let lock_id = lock.keep_locked();

    Ok(AccountsCreateStealthTransferStatementResponse {
        statements,
        lock_id,
        signing_keys: required_signers.into_iter().collect(),
        utxo_signers: utxo_signers.into_iter().collect(),
    })
}

/// Map a `lock_specific_outputs` failure to an RPC error. Caller-invalid selections (the SDK's `InvalidParameter`)
/// collapse to a single generic `invalid_params` that never reveals whether a named UTXO is unknown, foreign,
/// wrong-resource, or otherwise ineligible; genuine internal failures (storage, crypto) propagate unchanged as a
/// server error.
fn map_specific_selection_error(err: StealthOutputsApiError) -> anyhow::Error {
    if matches!(err, StealthOutputsApiError::InvalidParameter { .. }) {
        debug!(target: LOG_TARGET, "Rejected specific stealth input selection: {err}");
        invalid_params(
            "utxo_addresses",
            Some("one or more requested UTXOs are unavailable or ineligible for this account"),
        )
    } else {
        err.into()
    }
}

/// Map a `generate_transfer_statement` failure to an RPC error. `WalletCryptoError::InvalidArgument` names a
/// malformed field of the request — a `PayTo` condition set the engine could never admit, a negative revealed
/// amount, an unbalanced covenant partition — so it is reported as a caller error carrying that field name;
/// everything else propagates unchanged as a server error.
///
/// The variant reaches this handler by two routes: directly, from the output-authorization step, and wrapped in
/// `StealthCryptoApiError` from statement construction. Both must be recognised.
fn map_statement_construction_error(err: StealthOutputsApiError) -> anyhow::Error {
    let invalid_argument = match &err {
        StealthOutputsApiError::WalletCrypto(e) |
        StealthOutputsApiError::Crypto(StealthCryptoApiError::WalletCryptoError(e)) => Some(e),
        _ => None,
    };

    match invalid_argument {
        Some(WalletCryptoError::InvalidArgument { name, details }) => {
            debug!(target: LOG_TARGET, "Rejected stealth transfer statement: {err}");
            invalid_params(name, Some(details.clone()))
        },
        _ => err.into(),
    }
}

pub async fn handle_associate_stealth_resource(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: AccountsAssociateStealthResourceRequest,
) -> Result<AccountsAssociateStealthResourceResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    let sdk = context.wallet_sdk().clone();
    let account = get_account(&req.account, &sdk.accounts_api())?;
    enforce_scopes(&granted, &[Permission::Accounts(
        Crud::Update,
        Some(*account.component_address()),
    )])?;
    let resource = sdk.substate_api().fetch_resource(req.resource_address).await?; // validate resource exists and cache it
    if !resource.resource_type().is_stealth() {
        return Err(invalid_params(
            "resource_address",
            Some(format!(
                "Resource is not a stealth resource (type: {})",
                resource.resource_type()
            )),
        ));
    }

    context
        .account_monitor()
        .associate_resource(*account.component_address(), req.resource_address)
        .await?;

    context
        .account_monitor()
        .refresh_account_with_utxos(*account.component_address())
        .await?;

    Ok(AccountsAssociateStealthResourceResponse {})
}

#[cfg(test)]
mod balance_change_handler_tests {
    use std::str::FromStr;

    use axum_extra::headers::{Authorization, authorization::Bearer};
    use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
    use tari_engine_types::resource::Resource;
    use tari_ootle_address::Network;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_wallet_sdk::{
        WalletSdkConfig,
        cipher_seed::CipherSeedRestore,
        models::{BalanceChangeSnapshot, BalanceChangeSource, EpochBirthday, KeyBranch, KeyId},
        storage::{WalletStorageError, WalletStoreWriter, WriteableWalletStore},
    };
    use tari_ootle_wallet_sdk_services::{
        account_monitor::AccountMonitor,
        indexer_rest_api::IndexerRestApiNetworkInterface,
        notify::Notify,
        transaction_service::TransactionService,
        utxo_scanner::StealthUtxoScannerWorker,
    };
    use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
    use tari_ootle_walletd_client::permissions::Permissions;
    use tari_shutdown::Shutdown;
    use tari_template_lib_types::{
        ComponentAddress,
        Metadata,
        ResourceAddress,
        SubstateOwnerRule,
        VaultId,
        access_rules::ResourceAccessRules,
        constants::TOKEN_SYMBOL,
    };
    use tari_utilities::SafePassword;

    use super::*;
    use crate::{
        WalletSdk,
        config::{WalletDaemonAuth, WalletDaemonConfig},
        handlers::{HandlerContext, auth::create_authenticator},
    };

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn balance_changes_handler_authorizes_filters_and_paginates() {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();
        let mut sdk = WalletSdk::initialize_with_local_key_store(
            store.clone(),
            IndexerRestApiNetworkInterface::new("http://127.0.0.1:18300"),
            WalletSdkConfig {
                network: Network::LocalNet,
                override_keyring_password: Some(SafePassword::from_str("test wallet password").unwrap()),
            },
            EpochBirthday::far_future(),
        )
        .unwrap();
        sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)
            .unwrap();

        let account: ComponentAddress = "component_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a07ffffffff"
            .parse()
            .unwrap();
        let first_vault: VaultId = "vault_0000000000000000000000000000000000000000000000000000000000000001"
            .parse()
            .unwrap();
        let second_vault: VaultId = "vault_0000000000000000000000000000000000000000000000000000000000000002"
            .parse()
            .unwrap();
        let first_resource: ResourceAddress =
            "resource_0000000000000000000000000000000000000000000000000000000000000001"
                .parse()
                .unwrap();
        let second_resource: ResourceAddress =
            "resource_0000000000000000000000000000000000000000000000000000000000000002"
                .parse()
                .unwrap();
        let accounts = sdk.accounts_api();
        accounts
            .add_account(
                Some("savings"),
                &account,
                KeyId::derived(KeyBranch::ViewOnlyKey, 0),
                KeyId::derived(KeyBranch::Account, 0),
                Epoch::zero(),
                true,
                true,
            )
            .unwrap();
        for (vault, resource, symbol) in [
            (first_vault, first_resource, "ONE"),
            (second_vault, second_resource, "TWO"),
        ] {
            sdk.resources_api()
                .upsert_resource(
                    &resource,
                    &Resource::new(
                        ResourceType::Fungible,
                        SubstateOwnerRule::None,
                        ResourceAccessRules::new(),
                        Metadata::from([(TOKEN_SYMBOL, symbol)]),
                        None,
                        None,
                        2,
                        false,
                    ),
                )
                .unwrap();
            accounts
                .add_vault(
                    account,
                    vault,
                    0,
                    resource,
                    ResourceType::Fungible,
                    Some(symbol.to_string()),
                    2,
                )
                .unwrap();
        }
        accounts
            .update_vault_balance_and_record_change(
                first_vault,
                1,
                Amount::from(100u64),
                Amount::zero(),
                BalanceChangeSource::Scan,
            )
            .unwrap();
        accounts
            .update_vault_balance_and_record_change(
                first_vault,
                2,
                Amount::from(150u64),
                Amount::zero(),
                BalanceChangeSource::Recovery,
            )
            .unwrap();
        accounts
            .update_vault_balance_and_record_change(
                second_vault,
                1,
                Amount::from(200u64),
                Amount::zero(),
                BalanceChangeSource::Scan,
            )
            .unwrap();
        // Enough rows to exercise MAX_BALANCE_CHANGE_LIMIT. These are bulk fixture data - only their
        // count is asserted on - so they are inserted in one transaction. Recording them through
        // update_vault_balance_and_record_change commits each one separately, and the resulting 205
        // fsyncs dominate the test and scale with disk latency on CI.
        store
            .with_write_tx(|tx| {
                for version in 2..=206u64 {
                    tx.balance_changes_insert(
                        BalanceChangeSnapshot {
                            account_address: account,
                            vault_address: Some(second_vault),
                            vault_version: Some(version),
                            resource_address: second_resource,
                            token_symbol: Some("TWO".to_string()),
                            divisibility: 2,
                            revealed_before: Amount::from(199u64 + version),
                            revealed_after: Amount::from(200u64 + version),
                            confidential_before: Amount::zero(),
                            confidential_after: Amount::zero(),
                        },
                        BalanceChangeSource::Scan,
                    )?;
                }
                Ok::<_, WalletStorageError>(())
            })
            .unwrap();

        let notify = Notify::new(10);
        let mut shutdown = Shutdown::new();
        let (transaction_service, transaction_service_handle) =
            TransactionService::new(notify.clone(), sdk.clone(), shutdown.to_signal());
        let (utxo_worker, utxo_scanner_handle) = StealthUtxoScannerWorker::new(sdk.clone(), notify.clone()).spawn();
        let (account_monitor, account_monitor_handle) =
            AccountMonitor::new(notify.clone(), sdk.clone(), utxo_scanner_handle, shutdown.to_signal());
        let mut config = WalletDaemonConfig::default();
        config.network = Network::LocalNet;
        config.authentication = WalletDaemonAuth::None;
        let context = HandlerContext::new(
            sdk,
            notify,
            transaction_service_handle,
            account_monitor_handle,
            config.clone(),
            create_authenticator(&config, store).unwrap(),
            SafePassword::from_str("test jwt secret").unwrap(),
            shutdown.to_signal(),
        );
        let permissions = Permissions::from_str(&format!("accounts:read:{account}")).unwrap();
        let claims = context.jwt_api().generate_auth_claims(permissions).unwrap();
        let token = context.jwt_api().grant(&claims).unwrap();
        let bearer = Authorization::<Bearer>::bearer(&token).unwrap().0;

        let response = handle_get_balance_changes(&context, Some(&bearer), AccountsGetBalanceChangesRequest {
            account: "savings".into(),
            offset: 1,
            limit: 1,
            resource_address: Some(first_resource),
            transaction_id: None,
            source_type: None,
        })
        .await
        .unwrap();

        assert_eq!(response.total, 2);
        assert_eq!(response.changes.len(), 1);
        assert_eq!(response.changes[0].resource_address, first_resource);
        assert_eq!(response.changes[0].revealed_delta, "100");
        assert_eq!(response.changes[0].source, BalanceChangeSource::Scan);

        let capped_response = handle_get_balance_changes(&context, Some(&bearer), AccountsGetBalanceChangesRequest {
            account: "savings".into(),
            offset: 0,
            limit: u32::MAX,
            resource_address: None,
            transaction_id: None,
            source_type: None,
        })
        .await
        .unwrap();
        assert_eq!(capped_response.total, 208);
        assert_eq!(capped_response.changes.len(), 200);

        let missing_account_err =
            handle_get_balance_changes(&context, Some(&bearer), AccountsGetBalanceChangesRequest {
                account: "missing".into(),
                offset: 0,
                limit: 1,
                resource_address: None,
                transaction_id: None,
                source_type: None,
            })
            .await
            .unwrap_err();
        let rpc_error = missing_account_err.downcast_ref::<JsonRpcError>().unwrap();
        assert!(matches!(
            rpc_error.error_reason(),
            JsonRpcErrorReason::ApplicationError(404)
        ));

        shutdown.trigger();
        drop(account_monitor);
        drop(transaction_service);
        utxo_worker.abort();
        drop(utxo_worker.await);
    }
}

#[cfg(test)]
mod fee_estimate_step_tests {
    use super::*;

    /// A dry-run round shows a fee covers itself for the figure the round before it named; the
    /// static estimate shows the same thing without a round trip. Either way the fee is verified.
    #[test]
    fn settles_once_a_verified_fee_covers_the_shape_it_produces() {
        assert_eq!(
            next_fee_estimate_step(9_000, 9_000, 9_025, true, 2),
            FeeEstimateStep::Settle
        );
        assert_eq!(
            next_fee_estimate_step(9_000, 8_000, 8_025, true, 2),
            FeeEstimateStep::Settle
        );
    }

    /// The caller's guessed fee has nothing behind it, so a round that covers itself unverified
    /// still has to be built at the figure it reports before that figure can be answered with.
    ///
    /// This delays a cost of zero by one round rather than rejecting it: a second round is verified,
    /// and `0 <= built_at`, so it would settle. What keeps a zero out of here is
    /// `FinalizeResult::charged_fees`, which falls back to what the receipt was charged.
    #[test]
    fn does_not_settle_on_the_callers_guess() {
        assert_eq!(next_fee_estimate_step(1, 0, 9_354, false, 1), FeeEstimateStep::Retry {
            max_fee: 9_354
        });
    }

    /// A shape that costs more than the fee it was built at is not submittable, so the figure it
    /// reports becomes the next fee to try.
    #[test]
    fn retries_at_the_reported_figure_when_the_shape_costs_more() {
        assert_eq!(
            next_fee_estimate_step(9_354, 15_951, 15_976, true, 2),
            FeeEstimateStep::Retry { max_fee: 15_976 }
        );
    }

    /// The cap is a bound on rounds, not a settle condition: the estimate returned is whatever the
    /// last round reached, which is the highest, since a round only continues while the shape costs
    /// more than it was built at.
    #[test]
    fn gives_up_at_the_round_cap() {
        assert_eq!(
            next_fee_estimate_step(15_976, 15_978, 16_003, true, MAX_FEE_ESTIMATE_ROUNDS),
            FeeEstimateStep::GiveUp
        );
        // The cap does not pre-empt a settle that has already been established.
        assert_eq!(
            next_fee_estimate_step(15_976, 15_976, 16_001, true, MAX_FEE_ESTIMATE_ROUNDS),
            FeeEstimateStep::Settle
        );
    }

    /// Three builds settle the case this exists for: the caller's guess, the shape it produces, and
    /// a build at what that shape reported.
    #[test]
    fn the_cap_leaves_room_for_the_common_case() {
        const {
            assert!(MAX_FEE_ESTIMATE_ROUNDS >= 3);
        }
    }
}

#[cfg(test)]
mod create_stealth_transfer_statement_handler_tests {
    use std::str::FromStr;

    use axum_extra::headers::{Authorization, authorization::Bearer};
    use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
    use tari_crypto::{keys::SecretKey as _, ristretto::RistrettoSecretKey};
    use tari_engine_types::{
        resource::Resource,
        stealth::{MerkleTree, hashlock_digest},
    };
    use tari_ootle_address::{Network, OotleAddress, RistrettoOotleAddress};
    use tari_ootle_common_types::Epoch;
    use tari_ootle_wallet_crypto::pay_to::PayTo;
    use tari_ootle_wallet_sdk::{
        WalletSdkConfig,
        cipher_seed::CipherSeedRestore,
        models::{EpochBirthday, KeyBranch, KeyId, OutputStatus, StealthOutputModel},
    };
    use tari_ootle_wallet_sdk_services::{
        account_monitor::AccountMonitor,
        indexer_rest_api::IndexerRestApiNetworkInterface,
        notify::Notify,
        transaction_service::TransactionService,
        utxo_scanner::StealthUtxoScannerWorker,
    };
    use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
    use tari_ootle_walletd_client::{
        permissions::Permissions,
        types::{InputSelection, TransferStatementRequest},
    };
    use tari_shutdown::Shutdown;
    use tari_template_lib_types::{
        ComponentAddress,
        Metadata,
        NonFungibleAddress,
        ResourceAddress,
        SubstateOwnerRule,
        UtxoAddress,
        access_rules::{AccessRule, RequireRule, ResourceAccessRules, RestrictedAccessRule, RuleRequirement},
        constants::TOKEN_SYMBOL,
        crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
        rule,
        stealth::{AtomicCondition, BuiltinPredicate, HashAlg, SpendAuthorization, SpendCondition},
    };
    use tari_utilities::SafePassword;

    use super::*;
    use crate::{
        WalletSdk,
        config::{WalletDaemonAuth, WalletDaemonConfig},
        handlers::{
            HandlerContext,
            auth::{create_authenticator, jwt::AuthError},
        },
    };

    /// A ready-to-use handler context for `account`, with a stealth resource cached locally so `fetch_resource`
    /// resolves offline. Tests mint their own bearer with [`StatementTest::bearer`] to control which scopes are
    /// granted. The wallet's background workers are not exercised by this handler, so they are torn down during
    /// setup. `_temp` keeps the SQLite directory alive for the test.
    struct StatementTest {
        context: HandlerContext,
        account: ComponentAddress,
        stealth_resource: ResourceAddress,
        _temp: tempfile::TempDir,
    }

    impl StatementTest {
        /// A bearer token granting exactly `permissions` (a comma-separated scope string).
        fn bearer(&self, permissions: &str) -> Bearer {
            let permissions = Permissions::from_str(permissions).unwrap();
            let claims = self.context.jwt_api().generate_auth_claims(permissions).unwrap();
            let token = self.context.jwt_api().grant(&claims).unwrap();
            Authorization::<Bearer>::bearer(&token).unwrap().0
        }

        /// Both scopes the Specific path requires: transfer-create and stealth-UTXO-read for this account.
        fn transfer_and_read_scopes(&self) -> String {
            format!(
                "transfer:create:{account},stealth_utxos:read:{account}",
                account = self.account
            )
        }
    }

    async fn setup() -> StatementTest {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();
        let mut sdk = WalletSdk::initialize_with_local_key_store(
            store.clone(),
            IndexerRestApiNetworkInterface::new("http://127.0.0.1:18300"),
            WalletSdkConfig {
                network: Network::LocalNet,
                override_keyring_password: Some(SafePassword::from_str("test wallet password").unwrap()),
            },
            EpochBirthday::far_future(),
        )
        .unwrap();
        sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)
            .unwrap();

        let account: ComponentAddress = "component_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a07ffffffff"
            .parse()
            .unwrap();
        sdk.accounts_api()
            .add_account(
                Some("stealth"),
                &account,
                KeyId::derived(KeyBranch::ViewOnlyKey, 0),
                KeyId::derived(KeyBranch::Account, 0),
                Epoch::zero(),
                true,
                true,
            )
            .unwrap();

        let stealth_resource: ResourceAddress =
            "resource_0000000000000000000000000000000000000000000000000000000000000abc"
                .parse()
                .unwrap();
        sdk.resources_api()
            .upsert_resource(
                &stealth_resource,
                &Resource::new(
                    ResourceType::Stealth,
                    SubstateOwnerRule::None,
                    ResourceAccessRules::new(),
                    Metadata::from([(TOKEN_SYMBOL, "sTST")]),
                    None,
                    None,
                    0,
                    false,
                ),
            )
            .unwrap();

        let notify = Notify::new(10);
        let mut shutdown = Shutdown::new();
        let (transaction_service, transaction_service_handle) =
            TransactionService::new(notify.clone(), sdk.clone(), shutdown.to_signal());
        let (utxo_worker, utxo_scanner_handle) = StealthUtxoScannerWorker::new(sdk.clone(), notify.clone()).spawn();
        let (account_monitor, account_monitor_handle) =
            AccountMonitor::new(notify.clone(), sdk.clone(), utxo_scanner_handle, shutdown.to_signal());
        let mut config = WalletDaemonConfig::default();
        config.network = Network::LocalNet;
        config.authentication = WalletDaemonAuth::None;
        let context = HandlerContext::new(
            sdk,
            notify,
            transaction_service_handle,
            account_monitor_handle,
            config.clone(),
            create_authenticator(&config, store).unwrap(),
            SafePassword::from_str("test jwt secret").unwrap(),
            shutdown.to_signal(),
        );

        // The transaction service, account monitor and scanner are not used by this handler; shut them down now.
        shutdown.trigger();
        drop(account_monitor);
        drop(transaction_service);
        utxo_worker.abort();
        drop(utxo_worker.await);

        StatementTest {
            context,
            account,
            stealth_resource,
            _temp: temp,
        }
    }

    /// The test account's Ootle address, used as the destination for balancing outputs.
    fn account_ootle_address(test: &StatementTest) -> OotleAddress {
        let account = ComponentAddressOrName::from(test.account);
        let sender = get_account(&account, &test.context.wallet_sdk().accounts_api()).unwrap();
        sender.address().clone()
    }

    /// Mints a cryptographically valid, wallet-owned, on-chain stealth output for the test account worth `value` and
    /// inserts it. The commitment/nonce/encrypted-data/auth all come from a real output witness, so the input can be
    /// decrypted and spent through the statement flow. Returns the stored model.
    fn insert_valid_output(test: &StatementTest, value: u64) -> StealthOutputModel {
        let sdk = test.context.wallet_sdk();
        let account = ComponentAddressOrName::from(test.account);
        let sender = get_account(&account, &sdk.accounts_api()).unwrap();
        let owner_address: RistrettoOotleAddress = sender.address().try_from_byte_type().unwrap();
        let witness = sdk
            .stealth_outputs_api()
            .create_output_witness(
                &owner_address,
                value,
                &test.stealth_resource,
                None,
                None,
                PayTo::StealthPublicKey,
            )
            .unwrap();
        let model = StealthOutputModel {
            owner_account: test.account,
            resource_address: test.stealth_resource,
            commitment: witness.witness.to_commitment().to_byte_type(),
            value,
            sender_public_nonce: witness.witness.sender_public_nonce.to_byte_type(),
            view_only_key_id: sender.view_only_key_id(),
            owner_key_id: sender.owner_key_id(),
            encrypted_data: witness.witness.encrypted_data.clone(),
            tag_byte: witness.tag,
            memo: None,
            auth: witness.auth.clone(),
            minimum_value_promise: witness.witness.minimum_value_promise,
            status: OutputStatus::Unspent,
            is_burnt: false,
            is_frozen: false,
            is_on_chain: true,
            is_condition_spendable: true,
            lock_id: None,
        };
        sdk.stealth_outputs_api().add_output(&model).unwrap();
        model
    }

    /// Reads a stored output back by commitment for post-call assertions.
    fn stored_output(test: &StatementTest, commitment: &PedersenCommitmentBytes) -> StealthOutputModel {
        test.context
            .wallet_sdk()
            .stealth_outputs_api()
            .utxos_get_many(&test.stealth_resource, Some(&test.account), None)
            .unwrap()
            .into_iter()
            .find(|o| &o.commitment == commitment)
            .expect("output present in store")
    }

    fn specific_request(
        account: ComponentAddress,
        resource: ResourceAddress,
        utxo_addresses: Vec<UtxoAddress>,
    ) -> AccountsCreateStealthTransferStatementRequest {
        AccountsCreateStealthTransferStatementRequest {
            requests: vec![TransferStatementRequest {
                sender_account: account.into(),
                resource_address: resource,
                input_selection: InputSelection::Specific { utxo_addresses },
                outputs: vec![],
            }],
        }
    }

    /// Downcasts a handler error to a `JsonRpcError` and asserts it is `InvalidParams`.
    fn assert_invalid_params(err: &anyhow::Error) -> &JsonRpcError {
        let rpc = err
            .downcast_ref::<JsonRpcError>()
            .unwrap_or_else(|| panic!("expected a JsonRpcError, got: {err}"));
        assert!(
            matches!(rpc.error_reason(), JsonRpcErrorReason::InvalidParams),
            "expected InvalidParams, got: {:?}",
            rpc.error_reason()
        );
        rpc
    }

    #[tokio::test]
    async fn specific_requires_stealth_utxo_read_scope() {
        let test = setup().await;
        // Only transfer-create is granted; the specific path additionally requires stealth_utxos:read.
        let bearer = test.bearer(&format!("transfer:create:{}", test.account));
        let utxo = UtxoAddress::new(
            test.stealth_resource,
            PedersenCommitmentBytes::from_array([1u8; PedersenCommitmentBytes::length()]).into(),
        );
        let request = specific_request(test.account, test.stealth_resource, vec![utxo]);
        let err = handle_create_stealth_transfer_statement(&test.context, Some(&bearer), request)
            .await
            .expect_err("specific selection without stealth_utxos:read must be rejected");
        let auth_err = err
            .downcast_ref::<AuthError>()
            .unwrap_or_else(|| panic!("expected an AuthError, got: {err}"));
        assert!(matches!(auth_err, AuthError::InsufficientPermissions { .. }));
        assert!(
            auth_err.to_string().contains("StealthUtxos"),
            "the missing scope should be the stealth-UTXO read scope: {auth_err}"
        );
    }

    #[tokio::test]
    async fn rejects_empty_specific_inputs() {
        let test = setup().await;
        let bearer = test.bearer(&test.transfer_and_read_scopes());
        let request = specific_request(test.account, test.stealth_resource, vec![]);
        let err = handle_create_stealth_transfer_statement(&test.context, Some(&bearer), request)
            .await
            .expect_err("empty specific input list must be rejected");
        assert_invalid_params(&err);
    }

    #[tokio::test]
    async fn rejects_ineligible_specific_input_without_disclosure() {
        let test = setup().await;
        let bearer = test.bearer(&test.transfer_and_read_scopes());
        // A well-formed but unknown UTXO for the correct resource.
        let unknown = UtxoAddress::new(
            test.stealth_resource,
            PedersenCommitmentBytes::from_array([9u8; PedersenCommitmentBytes::length()]).into(),
        );
        let request = specific_request(test.account, test.stealth_resource, vec![unknown]);
        let err = handle_create_stealth_transfer_statement(&test.context, Some(&bearer), request)
            .await
            .expect_err("unknown specific input must be rejected");
        let message = assert_invalid_params(&err).to_string();
        // The wire message is generic: it must not reveal whether the commitment is unknown, foreign, or otherwise
        // ineligible.
        assert!(
            message.contains("unavailable or ineligible"),
            "not a generic message: {message}"
        );
        assert!(!message.contains("not known"), "leaks existence: {message}");
        assert!(!message.contains("does not belong"), "leaks ownership: {message}");
    }

    #[tokio::test]
    async fn specific_selection_builds_balanced_statement_in_request_order() {
        let test = setup().await;
        let bearer = test.bearer(&test.transfer_and_read_scopes());

        // Two valid wallet-owned inputs, inserted a then b.
        let output_a = insert_valid_output(&test, 100);
        let output_b = insert_valid_output(&test, 200);

        // Request them in reverse insertion order, with a single output that balances the selected inputs exactly.
        let request = AccountsCreateStealthTransferStatementRequest {
            requests: vec![TransferStatementRequest {
                sender_account: test.account.into(),
                resource_address: test.stealth_resource,
                input_selection: InputSelection::Specific {
                    utxo_addresses: vec![output_b.to_utxo_address(), output_a.to_utxo_address()],
                },
                outputs: vec![TransferOutput {
                    address: account_ootle_address(&test),
                    revealed_amount: Amount::zero(),
                    blinded_amount: 300,
                    memo: None,
                    pay_to: PayTo::StealthPublicKey,
                }],
            }],
        };

        let response = handle_create_stealth_transfer_statement(&test.context, Some(&bearer), request)
            .await
            .expect("a valid, balanced specific request should succeed");

        // Exactly one statement, whose inputs follow the caller's requested order (b then a).
        assert_eq!(response.statements.len(), 1);
        let inputs = &response.statements[0].inputs_statement.inputs;
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].commitment, output_b.commitment);
        assert_eq!(inputs[1].commitment, output_a.commitment);

        // UTXO signers are returned in the same requested order.
        assert_eq!(response.utxo_signers.len(), 2);
        assert_eq!(response.utxo_signers[0].public_nonce, output_b.sender_public_nonce);
        assert_eq!(response.utxo_signers[1].public_nonce, output_a.sender_public_nonce);

        // keep_locked was reached: both selected outputs remain LockedForSpend under the returned lock.
        for output in [&output_b, &output_a] {
            let stored = stored_output(&test, &output.commitment);
            assert!(
                matches!(stored.status, OutputStatus::LockedForSpend),
                "expected LockedForSpend, got {:?}",
                stored.status
            );
            assert_eq!(stored.lock_id, Some(response.lock_id));
        }
    }

    #[tokio::test]
    async fn unbalanced_specific_statement_releases_the_lock() {
        let test = setup().await;
        let bearer = test.bearer(&test.transfer_and_read_scopes());

        // A valid, spendable input worth 100.
        let output = insert_valid_output(&test, 100);

        // Deliberately provide an output that does not balance the selected input (100 in, 40 out): a deterministic
        // statement-construction failure that happens after the input has been locked.
        let request = AccountsCreateStealthTransferStatementRequest {
            requests: vec![TransferStatementRequest {
                sender_account: test.account.into(),
                resource_address: test.stealth_resource,
                input_selection: InputSelection::Specific {
                    utxo_addresses: vec![output.to_utxo_address()],
                },
                outputs: vec![TransferOutput {
                    address: account_ootle_address(&test),
                    revealed_amount: Amount::zero(),
                    blinded_amount: 40,
                    memo: None,
                    pay_to: PayTo::StealthPublicKey,
                }],
            }],
        };

        let err = handle_create_stealth_transfer_statement(&test.context, Some(&bearer), request)
            .await
            .expect_err("an unbalanced statement must fail after locking");
        assert!(
            err.to_string().contains("do not balance"),
            "expected a balance error, got: {err}"
        );

        // The lock guard released the selected output on failure: it is Unspent again with no lock.
        let stored = stored_output(&test, &output.commitment);
        assert!(
            matches!(stored.status, OutputStatus::Unspent),
            "expected Unspent after release, got {:?}",
            stored.status
        );
        assert_eq!(stored.lock_id, None);
    }

    /// A deterministic Ristretto public key standing in for an HTLC participant. Only the public half exists in this
    /// test: a condition tree commits public terms only, so no participant secret is needed to *create* a
    /// `PayTo::Conditions` output — a secret is required only later, to satisfy a leaf at spend time.
    fn htlc_participant_key(seed: u8) -> RistrettoPublicKeyBytes {
        let secret = RistrettoSecretKey::from_uniform_bytes(&[seed; 64]).unwrap();
        RistrettoPublicKey::from_secret_key(&secret).to_byte_type()
    }

    /// The access rule satisfied only by a proof of `public_key`, spelled out without the `rule!` macro so the
    /// condition-leaf assertions restate the rule independently of how the leaves were built.
    fn requires_public_key(public_key: RistrettoPublicKeyBytes) -> AccessRule {
        AccessRule::Restricted(RestrictedAccessRule::Require(RequireRule::Require(
            RuleRequirement::NonFungibleAddress(NonFungibleAddress::from_public_key(public_key)),
        )))
    }

    /// The two leaves of the HTLC condition tree, keyed on `refund_epoch` and the SHA-256 digest of `preimage`.
    ///
    /// Claim: hashlock AND before the refund epoch AND the claimant's key.
    /// Refund: at or after the refund epoch AND the refunder's key.
    ///
    /// The epoch bound is the same on both leaves, so exactly one path is admissible at any epoch.
    fn htlc_conditions(
        preimage: &[u8],
        refund_epoch: u64,
        claimant: RistrettoPublicKeyBytes,
        refunder: RistrettoPublicKeyBytes,
    ) -> (SpendCondition, SpendCondition) {
        let claim = SpendCondition::all([
            AtomicCondition::Builtin(BuiltinPredicate::HashLock {
                hash: hashlock_digest(HashAlg::Sha256, preimage),
                alg: HashAlg::Sha256,
            }),
            AtomicCondition::Builtin(BuiltinPredicate::BeforeEpoch(refund_epoch)),
            AtomicCondition::AccessRule(rule!(public_key(claimant))),
        ]);
        let refund = SpendCondition::all([
            AtomicCondition::Builtin(BuiltinPredicate::AfterEpoch(refund_epoch)),
            AtomicCondition::AccessRule(rule!(public_key(refunder))),
        ]);
        (claim, refund)
    }

    /// Phase 3C: the statement handler can build a fully blinded stealth output whose spend authority is a committed
    /// two-leaf HTLC condition tree (`PayTo::Conditions`), from an exactly-named `InputSelection::Specific` input set.
    ///
    /// This is the creation half of TIP-0006 script-path spending: the output commits only public terms (a SHA-256
    /// digest, two epoch bounds and two public keys), carries no key path, and needs no participant secret to build.
    #[tokio::test]
    async fn specific_selection_builds_blinded_pay_to_conditions_htlc_output() {
        let test = setup().await;
        let bearer = test.bearer(&test.transfer_and_read_scopes());

        // Two valid wallet-owned inputs, inserted a then b, funding a single HTLC output of their exact total.
        let output_a = insert_valid_output(&test, 100);
        let output_b = insert_valid_output(&test, 150);

        const REFUND_EPOCH: u64 = 4_242;
        let preimage = b"phase3c-htlc-preimage";
        let claimant = htlc_participant_key(1);
        let refunder = htlc_participant_key(2);
        let (claim_condition, refund_condition) = htlc_conditions(preimage, REFUND_EPOCH, claimant, refunder);
        let conditions = vec![claim_condition.clone(), refund_condition.clone()];

        // The committed root, computed independently of the handler from the public leaves via the canonical
        // condition-tree code in `tari_engine_types`.
        let expected_condition_root = MerkleTree::from_conditions(&conditions).unwrap().root();

        // Request the inputs in reverse insertion order (b then a) to pin the ordering guarantee, and pay the whole
        // 250 to a blinded conditions output: nothing revealed.
        let request = AccountsCreateStealthTransferStatementRequest {
            requests: vec![TransferStatementRequest {
                sender_account: test.account.into(),
                resource_address: test.stealth_resource,
                input_selection: InputSelection::Specific {
                    utxo_addresses: vec![output_b.to_utxo_address(), output_a.to_utxo_address()],
                },
                outputs: vec![TransferOutput {
                    address: account_ootle_address(&test),
                    revealed_amount: Amount::zero(),
                    blinded_amount: 250,
                    memo: None,
                    pay_to: PayTo::Conditions(conditions),
                }],
            }],
        };

        let response = handle_create_stealth_transfer_statement(&test.context, Some(&bearer), request)
            .await
            .expect("a balanced specific request paying to a condition tree should succeed");

        // Exactly the two named inputs, in the requested order, contributing nothing revealed.
        assert_eq!(response.statements.len(), 1);
        let statement = &response.statements[0];
        let inputs = &statement.inputs_statement.inputs;
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0].commitment, output_b.commitment);
        assert_eq!(inputs[1].commitment, output_a.commitment);
        assert_eq!(statement.inputs_statement.revealed_amount, Amount::zero());

        // One signer per input, in the same requested order.
        assert_eq!(response.utxo_signers.len(), 2);
        assert_eq!(response.utxo_signers[0].public_nonce, output_b.sender_public_nonce);
        assert_eq!(response.utxo_signers[1].public_nonce, output_a.sender_public_nonce);

        // A single, fully blinded output: the whole 250 is committed, none of it revealed.
        assert_eq!(statement.outputs_statement.revealed_output_amount, Amount::zero());
        let outputs = statement.stealth_outputs();
        assert_eq!(outputs.len(), 1);

        // The output is script-path only: its authorization is the independently computed condition root, and it has
        // no key path at all.
        assert_eq!(outputs[0].auth, SpendAuthorization::Script(expected_condition_root));
        assert_eq!(outputs[0].auth.condition_root(), Some(&expected_condition_root));
        assert!(
            outputs[0].auth.spend_key().is_none(),
            "a conditions output must carry no key path"
        );

        // The root commits both leaves and is order-independent: neither leaf alone reproduces it, and reversing the
        // leaf order does not change it.
        assert_ne!(
            expected_condition_root,
            MerkleTree::from_conditions([&claim_condition]).unwrap().root()
        );
        assert_ne!(
            expected_condition_root,
            MerkleTree::from_conditions([&refund_condition]).unwrap().root()
        );
        assert_eq!(
            expected_condition_root,
            MerkleTree::from_conditions([&refund_condition, &claim_condition])
                .unwrap()
                .root()
        );

        // The leaves are the HTLC conjunctions, spelled out without the `rule!` macro: claim = SHA-256 hashlock AND
        // refund-epoch deadline AND claimant key; refund = refund epoch reached AND refunder key.
        assert_eq!(claim_condition.conditions(), &[
            AtomicCondition::Builtin(BuiltinPredicate::HashLock {
                hash: hashlock_digest(HashAlg::Sha256, preimage),
                alg: HashAlg::Sha256,
            }),
            AtomicCondition::Builtin(BuiltinPredicate::BeforeEpoch(REFUND_EPOCH)),
            AtomicCondition::AccessRule(requires_public_key(claimant)),
        ]);
        assert_eq!(refund_condition.conditions(), &[
            AtomicCondition::Builtin(BuiltinPredicate::AfterEpoch(REFUND_EPOCH)),
            AtomicCondition::AccessRule(requires_public_key(refunder)),
        ]);

        // A real lock was taken and kept: both selected inputs are LockedForSpend under the returned lock id.
        assert!(
            response.lock_id > 0,
            "expected a real lock id, got {}",
            response.lock_id
        );
        for output in [&output_b, &output_a] {
            let stored = stored_output(&test, &output.commitment);
            assert!(
                matches!(stored.status, OutputStatus::LockedForSpend),
                "expected LockedForSpend, got {:?}",
                stored.status
            );
            assert_eq!(stored.lock_id, Some(response.lock_id));
        }
    }

    /// Builds a balanced single-output request paying the whole of one 100-value input to `conditions`.
    fn pay_to_conditions_request(
        test: &StatementTest,
        input: &StealthOutputModel,
        conditions: Vec<SpendCondition>,
    ) -> AccountsCreateStealthTransferStatementRequest {
        AccountsCreateStealthTransferStatementRequest {
            requests: vec![TransferStatementRequest {
                sender_account: test.account.into(),
                resource_address: test.stealth_resource,
                input_selection: InputSelection::Specific {
                    utxo_addresses: vec![input.to_utxo_address()],
                },
                outputs: vec![TransferOutput {
                    address: account_ootle_address(test),
                    revealed_amount: Amount::zero(),
                    blinded_amount: 100,
                    memo: None,
                    pay_to: PayTo::Conditions(conditions),
                }],
            }],
        }
    }

    /// Asserts a request is rejected as `InvalidParams` naming `expected_field`, and that the named input was never
    /// locked — these are all request errors caught before any input selection runs.
    async fn assert_rejected_without_locking(
        test: &StatementTest,
        input: &StealthOutputModel,
        request: AccountsCreateStealthTransferStatementRequest,
        expected_field: &str,
    ) {
        let bearer = test.bearer(&test.transfer_and_read_scopes());
        let err = handle_create_stealth_transfer_statement(&test.context, Some(&bearer), request)
            .await
            .expect_err("a malformed pay_to must be rejected");

        let message = assert_invalid_params(&err).to_string();
        assert!(
            message.contains(expected_field),
            "expected the offending field `{expected_field}` to be named, got: {message}"
        );

        let stored = stored_output(test, &input.commitment);
        assert!(
            matches!(stored.status, OutputStatus::Unspent),
            "expected Unspent, got {:?}",
            stored.status
        );
        assert_eq!(stored.lock_id, None);
    }

    /// A condition set that cannot form a tree is a malformed request, not a server fault.
    #[tokio::test]
    async fn empty_condition_set_is_rejected_as_invalid_params() {
        let test = setup().await;
        let input = insert_valid_output(&test, 100);
        let request = pay_to_conditions_request(&test, &input, vec![]);
        assert_rejected_without_locking(&test, &input, request, "conditions").await;
    }

    /// Duplicate leaves are likewise a caller error: the tree rejects them rather than silently deduplicating, so the
    /// caller learns the condition set is malformed instead of receiving a root over fewer leaves than they supplied.
    #[tokio::test]
    async fn duplicate_condition_leaves_are_rejected_as_invalid_params() {
        let test = setup().await;
        let input = insert_valid_output(&test, 100);

        let (claim_condition, _) =
            htlc_conditions(b"preimage", 4_242, htlc_participant_key(1), htlc_participant_key(2));
        let request = pay_to_conditions_request(&test, &input, vec![claim_condition.clone(), claim_condition]);

        assert_rejected_without_locking(&test, &input, request, "conditions").await;
    }

    /// An empty conjunction forms a perfectly good one-leaf tree, but the engine refuses to evaluate such a leaf, so
    /// the only committed spend path could never be taken and the funds would be unrecoverable.
    #[tokio::test]
    async fn structurally_inadmissible_leaf_is_rejected_as_invalid_params() {
        let test = setup().await;
        let input = insert_valid_output(&test, 100);
        let request = pay_to_conditions_request(&test, &input, vec![SpendCondition::all([])]);
        assert_rejected_without_locking(&test, &input, request, "conditions").await;
    }

    /// `pay_to` gates the blinded output. A revealed-only output produces no stealth output at all, so honouring the
    /// request as written is impossible: reject it rather than deposit the funds ungated.
    #[tokio::test]
    async fn gated_output_with_no_blinded_amount_is_rejected_as_invalid_params() {
        let test = setup().await;
        let input = insert_valid_output(&test, 100);

        let (claim_condition, _) =
            htlc_conditions(b"preimage", 4_242, htlc_participant_key(1), htlc_participant_key(2));
        let mut request = pay_to_conditions_request(&test, &input, vec![claim_condition]);
        request.requests[0].outputs[0].blinded_amount = 0;
        request.requests[0].outputs[0].revealed_amount = Amount::from(100u64);

        assert_rejected_without_locking(&test, &input, request, "pay_to").await;
    }
}

#[cfg(test)]
mod map_statement_construction_error_tests {
    use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};

    use super::*;

    fn invalid_argument() -> WalletCryptoError {
        WalletCryptoError::InvalidArgument {
            name: "covenant",
            details: "a covenant partition may not receive more value than it spends".to_string(),
        }
    }

    fn assert_maps_to_invalid_params(err: StealthOutputsApiError) {
        let mapped = map_statement_construction_error(err);
        let rpc = mapped
            .downcast_ref::<JsonRpcError>()
            .unwrap_or_else(|| panic!("expected a JsonRpcError, got: {mapped}"));
        assert!(
            matches!(rpc.error_reason(), JsonRpcErrorReason::InvalidParams),
            "expected InvalidParams, got: {:?}",
            rpc.error_reason()
        );
        assert!(
            rpc.to_string().contains("covenant"),
            "expected the offending field to be named, got: {rpc}"
        );
    }

    /// The output-authorization step returns the error directly.
    #[test]
    fn maps_a_direct_invalid_argument() {
        assert_maps_to_invalid_params(StealthOutputsApiError::WalletCrypto(invalid_argument()));
    }

    /// Statement construction returns the same error wrapped a layer deeper. Both routes reach this handler, so
    /// recognising only the direct one would leave the wrapped case reported as a server fault.
    #[test]
    fn maps_an_invalid_argument_wrapped_by_the_crypto_api() {
        assert_maps_to_invalid_params(StealthOutputsApiError::Crypto(invalid_argument().into()));
    }

    /// Anything that is not a malformed argument stays a server error.
    #[test]
    fn leaves_other_failures_as_server_errors() {
        let mapped = map_statement_construction_error(StealthOutputsApiError::InsufficientFunds);
        assert!(
            mapped.downcast_ref::<JsonRpcError>().is_none(),
            "expected a plain error, got a JsonRpcError: {mapped}"
        );
    }
}
