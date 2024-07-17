//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::convert::TryFrom;

use anyhow::anyhow;
use base64;
use log::*;
use rand::rngs::OsRng;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::{
    commitment::HomomorphicCommitment as Commitment,
    keys::PublicKey as _,
    ristretto::{RistrettoComSig, RistrettoPublicKey},
    tari_utilities::ByteArray,
};
use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_crypto::ConfidentialProofStatement;
use tari_dan_wallet_sdk::{
    apis::{confidential_transfer::TransferParams, jwt::JrpcPermission, key_manager, substate::ValidatorScanResult},
    models::NewAccountInfo,
    storage::WalletStore,
    DanWalletSdk,
};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_engine_types::{
    component::new_component_address_from_public_key,
    confidential::ConfidentialClaim,
    instruction::Instruction,
    substate::{Substate, SubstateId},
};
use tari_key_manager::key_manager::DerivedKey;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    constants::{XTR_FAUCET_COMPONENT_ADDRESS, XTR_FAUCET_VAULT_ADDRESS},
    models::{Amount, UnclaimedConfidentialOutputAddress},
    prelude::CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
};
use tari_transaction::{SubstateRequirement, Transaction};
use tari_wallet_daemon_client::{
    types::{
        AccountGetDefaultRequest,
        AccountGetRequest,
        AccountGetResponse,
        AccountInfo,
        AccountSetDefaultRequest,
        AccountSetDefaultResponse,
        AccountsCreateFreeTestCoinsRequest,
        AccountsCreateFreeTestCoinsResponse,
        AccountsCreateRequest,
        AccountsCreateResponse,
        AccountsGetBalancesRequest,
        AccountsGetBalancesResponse,
        AccountsInvokeRequest,
        AccountsInvokeResponse,
        AccountsListRequest,
        AccountsListResponse,
        AccountsTransferRequest,
        AccountsTransferResponse,
        BalanceEntry,
        ClaimBurnRequest,
        ClaimBurnResponse,
        ConfidentialTransferRequest,
        ConfidentialTransferResponse,
        RevealFundsRequest,
        RevealFundsResponse,
    },
    ComponentAddressOrName,
};
use tokio::task;

use super::context::HandlerContext;
use crate::{
    handlers::helpers::{
        get_account,
        get_account_or_default,
        get_account_with_inputs,
        invalid_params,
        wait_for_result,
        wait_for_result_and_account,
    },
    indexer_jrpc_impl::IndexerJsonRpcNetworkInterface,
    services::TransactionSubmittedEvent,
    DEFAULT_FEE,
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::handlers::transaction";

pub async fn handle_create(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountsCreateRequest,
) -> Result<AccountsCreateResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let key_manager_api = sdk.key_manager_api();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    if let Some(name) = req.account_name.as_ref() {
        if sdk.accounts_api().get_account_by_name(name).optional()?.is_some() {
            return Err(anyhow!("Account name '{}' already exists", name));
        }
    }

    let default_account = sdk.accounts_api().get_default()?;
    let inputs = sdk
        .substate_api()
        .locate_dependent_substates(&[default_account.address.clone()])
        .await?;

    let signing_key_index = req.key_id.unwrap_or(default_account.key_index);
    let signing_key = key_manager_api.derive_key(key_manager::TRANSACTION_BRANCH, signing_key_index)?;

    let owner_key = key_manager_api.next_key(key_manager::TRANSACTION_BRANCH)?;
    let owner_pk = PublicKey::from_secret_key(&owner_key.key);

    info!(
        target: LOG_TARGET,
        "Creating account with owner token {}. Fees are paid using account '{}' {}",
        owner_pk,
        default_account.name.as_deref().unwrap_or("<None>"),
        default_account.address
    );

    let max_fee = req.max_fee.unwrap_or(DEFAULT_FEE);
    let transaction = Transaction::builder()
        .fee_transaction_pay_from_component(default_account.address.as_component_address().unwrap(), max_fee)
        .create_account(owner_pk.clone())
        .with_inputs(inputs)
        .sign(&signing_key.key)
        .build();

    let mut events = context.notifier().subscribe();
    let tx_id = context
        .transaction_service()
        .submit_transaction_with_new_account(transaction, vec![], NewAccountInfo {
            name: req.account_name,
            key_index: owner_key.key_index,
            is_default: req.is_default,
        })
        .await?;

    let event = wait_for_result(&mut events, tx_id).await?;
    if let Some(reject) = event.finalize.result.reject() {
        return Err(anyhow!("Create account transaction rejected: {}", reject));
    }

    if let Some(reason) = event.finalize.reject() {
        return Err(anyhow!("Create account transaction failed: {}", reason));
    }

    let address = event
        .finalize
        .result
        .accept()
        .unwrap()
        .up_iter()
        .find(|(_, v)| v.version() == 0 && is_account_substate(v))
        .map(|(a, _)| a.clone())
        .ok_or_else(|| anyhow!("Finalize result did not UP any new version 0 component"))?;

    Ok(AccountsCreateResponse {
        address,
        public_key: owner_pk,
        result: event.finalize,
    })
}

pub async fn handle_set_default(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountSetDefaultRequest,
) -> Result<AccountSetDefaultResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let account = get_account(&req.account, &sdk.accounts_api())?;
    sdk.accounts_api().set_default_account(&account.address)?;
    Ok(AccountSetDefaultResponse {})
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountsListRequest,
) -> Result<AccountsListResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let accounts = sdk.accounts_api().get_many(req.offset, req.limit)?;
    let total = sdk.accounts_api().count()?;
    let km = sdk.key_manager_api();
    let accounts = accounts
        .into_iter()
        .map(|a| {
            let key = km.derive_key(key_manager::TRANSACTION_BRANCH, a.key_index)?;
            let pk = PublicKey::from_secret_key(&key.key);
            Ok(AccountInfo {
                account: a,
                public_key: pk,
            })
        })
        .collect::<Result<_, anyhow::Error>>()?;

    Ok(AccountsListResponse { accounts, total })
}

pub async fn handle_invoke(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountsInvokeRequest,
) -> Result<AccountsInvokeResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let account = get_account_or_default(req.account, &sdk.accounts_api())?;

    let signing_key = sdk
        .key_manager_api()
        .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;

    let inputs = sdk.substate_api().load_dependent_substates(&[&account.address])?;

    let inputs = inputs
        .into_iter()
        .map(|s| SubstateRequirement::new(s.substate_id.clone(), Some(s.version)));

    let account_address = account.address.as_component_address().unwrap();
    let transaction = Transaction::builder()
        .fee_transaction_pay_from_component(account_address, req.max_fee.unwrap_or(DEFAULT_FEE))
        .call_method(account_address, &req.method, req.args)
        .with_inputs(inputs)
        .sign(&signing_key.key)
        .build();

    let mut events = context.notifier().subscribe();
    let tx_id = context
        .transaction_service()
        .submit_transaction(transaction, vec![])
        .await?;

    let mut finalized = wait_for_result(&mut events, tx_id).await?;
    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reject) = finalized.finalize.reject() {
        return Err(anyhow!("Transaction rejected: {}", reject));
    }

    Ok(AccountsInvokeResponse {
        result: finalized.finalize.execution_results.pop(),
    })
}

pub async fn handle_get_balances(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountsGetBalancesRequest,
) -> Result<AccountsGetBalancesResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let account = get_account_or_default(req.account, &sdk.accounts_api())?;
    sdk.jwt_api()
        .check_auth(token, &[JrpcPermission::AccountBalance(account.clone().address)])?;
    if req.refresh {
        context
            .account_monitor()
            .refresh_account(account.address.clone())
            .await?;
    }
    let vaults = sdk.accounts_api().get_vaults_by_account(&account.address)?;

    let mut balances = Vec::with_capacity(vaults.len());
    for vault in vaults {
        balances.push(BalanceEntry {
            vault_address: vault.address,
            resource_address: vault.resource_address,
            balance: vault.revealed_balance,
            resource_type: vault.resource_type,
            confidential_balance: vault.confidential_balance,
            token_symbol: vault.token_symbol,
        })
    }

    Ok(AccountsGetBalancesResponse {
        address: account.address,
        balances,
    })
}

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountGetRequest,
) -> Result<AccountGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let account = get_account(&req.name_or_address, &sdk.accounts_api())?;
    let km = sdk.key_manager_api();
    let key = km.derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
    let public_key = PublicKey::from_secret_key(&key.key);
    Ok(AccountGetResponse { account, public_key })
}

pub async fn handle_get_default(
    context: &HandlerContext,
    token: Option<String>,
    _req: AccountGetDefaultRequest,
) -> Result<AccountGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::AccountInfo])?;
    let account = get_account_or_default(None, &sdk.accounts_api())?;
    let km = sdk.key_manager_api();
    let key = km.derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
    let public_key = PublicKey::from_secret_key(&key.key);
    Ok(AccountGetResponse { account, public_key })
}

#[allow(clippy::too_many_lines)]
pub async fn handle_reveal_funds(
    context: &HandlerContext,
    token: Option<String>,
    req: RevealFundsRequest,
) -> Result<RevealFundsResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let notifier = context.notifier().clone();
    let transaction_service = context.transaction_service().clone();

    // If the caller aborts the request early, this async block would be aborted at any await point. To avoid this, we
    // spawn a task that will continue running.
    task::spawn(async move {
        let account = get_account_or_default(req.account, &sdk.accounts_api())?;

        let vault = sdk
            .accounts_api()
            .get_vault_by_resource(&account.address, &CONFIDENTIAL_TARI_RESOURCE_ADDRESS)?;

        let max_fee = req.max_fee.unwrap_or(DEFAULT_FEE);
        let amount_to_reveal = req.amount_to_reveal + if req.pay_fee_from_reveal { max_fee } else { 0.into() };

        let proof_id = sdk.confidential_outputs_api().add_proof(&vault.address)?;

        let (inputs, input_value) =
            sdk.confidential_outputs_api()
                .lock_outputs_by_amount(&vault.address, amount_to_reveal, proof_id)?;
        let input_amount = Amount::try_from(input_value)?;

        let account_key = sdk
            .key_manager_api()
            .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;

        let output_mask = sdk.key_manager_api().next_key(key_manager::TRANSACTION_BRANCH)?;
        let (_, public_nonce) = PublicKey::random_keypair(&mut OsRng);

        let remaining_confidential_amount = input_amount - amount_to_reveal;
        let encrypted_data = sdk.confidential_crypto_api().encrypt_value_and_mask(
            remaining_confidential_amount.as_u64_checked().unwrap(),
            &output_mask.key,
            &public_nonce,
            &account_key.key,
        )?;

        let output_statement = ConfidentialProofStatement {
            amount: remaining_confidential_amount,
            mask: output_mask.key,
            sender_public_nonce: public_nonce,
            minimum_value_promise: 0,
            encrypted_data,
            reveal_amount: amount_to_reveal,
            resource_view_key: None,
        };

        let inputs = sdk
            .confidential_outputs_api()
            .resolve_output_masks(inputs, key_manager::TRANSACTION_BRANCH)?;

        let reveal_proof =
            sdk.confidential_crypto_api()
                .generate_withdraw_proof(&inputs, Amount::zero(), &output_statement, None)?;

        info!(
            target: LOG_TARGET,
            "Locked {} inputs ({}) for reveal funds transaction on account: {}",
            inputs.len(),
            input_amount,
            account.address
        );

        let account_address = account.address.as_component_address().unwrap();

        let mut builder = Transaction::builder();
        if req.pay_fee_from_reveal {
            builder = builder.with_fee_instructions(vec![
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "withdraw_confidential".to_string(),
                    args: args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS, reveal_proof],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"revealed".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "deposit".to_string(),
                    args: args![Workspace("revealed".to_string())],
                },
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "pay_fee".to_string(),
                    args: args![max_fee],
                },
            ]);
        } else {
            builder = builder
                .fee_transaction_pay_from_component(account_address, max_fee)
                .call_method(account_address, "withdraw_confidential", args![
                    CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
                    reveal_proof
                ])
                .put_last_instruction_output_on_workspace("revealed")
                .call_method(account_address, "deposit", args![Workspace("revealed")]);
        }

        // Add the account component
        let account_substate = sdk.substate_api().get_substate(&account.address)?;
        // Add all versioned account child addresses as inputs
        let child_addresses = sdk.substate_api().load_dependent_substates(&[&account.address])?;
        let mut inputs = vec![account_substate.address];
        inputs.extend(child_addresses);

        let inputs = inputs
            .into_iter()
            .map(|addr| SubstateRequirement::new(addr.substate_id.clone(), Some(addr.version)));

        let transaction = builder.with_inputs(inputs).sign(&account_key.key).build();

        sdk.confidential_outputs_api()
            .proofs_set_transaction_hash(proof_id, *transaction.id())?;

        let mut events = notifier.subscribe();
        let tx_id = transaction_service.submit_transaction(transaction, vec![]).await?;

        let finalized = wait_for_result(&mut events, tx_id).await?;
        if let Some(reason) = finalized.finalize.reject() {
            return Err(anyhow::anyhow!("Transaction failed: {}", reason));
        }

        Ok(RevealFundsResponse {
            transaction_id: tx_id,
            fee: finalized.final_fee,
            result: finalized.finalize,
        })
    })
    .await?
}

#[allow(clippy::too_many_lines)]
pub async fn handle_claim_burn(
    context: &HandlerContext,
    token: Option<String>,
    req: ClaimBurnRequest,
) -> Result<ClaimBurnResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let ClaimBurnRequest {
        account,
        claim_proof,
        max_fee,
        key_id,
    } = req;

    let max_fee = max_fee.unwrap_or(DEFAULT_FEE);
    if max_fee.is_negative() {
        return Err(invalid_params("fee", Some("cannot be negative")));
    }

    let reciprocal_claim_public_key = PublicKey::from_canonical_bytes(
        &base64::decode(
            claim_proof["reciprocal_claim_public_key"]
                .as_str()
                .ok_or_else(|| invalid_params::<&str>("reciprocal_claim_public_key", None))?,
        )
        .map_err(|e| invalid_params("reciprocal_claim_public_key", Some(e)))?,
    )
    .map_err(|e| invalid_params("reciprocal_claim_public_key", Some(e)))?;
    let commitment = base64::decode(
        claim_proof["commitment"]
            .as_str()
            .ok_or_else(|| invalid_params::<&str>("commitment", None))?,
    )
    .map_err(|e| invalid_params("commitment", Some(e)))?;
    let range_proof = base64::decode(
        claim_proof["range_proof"]
            .as_str()
            .or_else(|| claim_proof["rangeproof"].as_str())
            .ok_or_else(|| invalid_params::<&str>("range_proof", None))?,
    )
    .map_err(|e| invalid_params("range_proof", Some(e)))?;

    let public_nonce = PublicKey::from_canonical_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["public_nonce"]
                .as_str()
                .ok_or_else(|| invalid_params::<&str>("ownership_proof.public_nonce", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.public_nonce", Some(e)))?,
    )
    .map_err(|e| invalid_params("ownership_proof.public_nonce", Some(e)))?;
    let u = PrivateKey::from_canonical_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["u"]
                .as_str()
                .ok_or_else(|| invalid_params::<&str>("ownership_proof.u", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.u", Some(e)))?,
    )
    .map_err(|e| invalid_params("ownership_proof.u", Some(e)))?;
    let v = PrivateKey::from_canonical_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["v"]
                .as_str()
                .ok_or_else(|| invalid_params::<&str>("ownership_proof.v", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.v", Some(e)))?,
    )
    .map_err(|e| invalid_params("ownership_proof.v", Some(e)))?;

    let mut inputs = vec![];
    let accounts_api = sdk.accounts_api();
    let (account_address, account_secret_key, new_account_name) =
        get_or_create_account(&account, &accounts_api, key_id, sdk, &mut inputs)?;

    let account_public_key = PublicKey::from_secret_key(&account_secret_key.key);

    info!(
        target: LOG_TARGET,
        "Signing claim burn with key {}. This must be the same as the claiming key used in the burn transaction.",
        account_public_key
    );

    // Add all versioned account child addresses as inputs
    // add the commitment substate id as input to the claim burn transaction
    let commitment_substate_address =
        SubstateRequirement::unversioned(UnclaimedConfidentialOutputAddress::try_from(commitment.as_slice())?);
    inputs.push(commitment_substate_address.clone());

    info!(
        target: LOG_TARGET,
        "Loaded {} inputs for claim burn transaction on account: {:?}",
        inputs.len(),
        account
    );

    // We have to unmask the commitment to allow us to reveal funds for the fee payment
    let ValidatorScanResult { substate: output, .. } = sdk
        .substate_api()
        .scan_for_substate(
            &commitment_substate_address.substate_id,
            commitment_substate_address.version,
        )
        .await?;
    let output = output.into_unclaimed_confidential_output().unwrap();
    let unmasked_output = sdk.confidential_crypto_api().unblind_output(
        &output.commitment,
        &output.encrypted_data,
        &account_secret_key.key,
        &reciprocal_claim_public_key,
    )?;

    let mask = sdk.key_manager_api().next_key(key_manager::TRANSACTION_BRANCH)?;
    let (nonce, output_public_nonce) = PublicKey::random_keypair(&mut OsRng);

    let final_amount = Amount::try_from(unmasked_output.value)? - max_fee;
    if final_amount.is_negative() {
        return Err(anyhow::anyhow!(
            "Fee ({}) is greater than the claimed output amount ({})",
            max_fee,
            unmasked_output.value
        ));
    }

    let encrypted_data = sdk.confidential_crypto_api().encrypt_value_and_mask(
        final_amount.as_u64_checked().unwrap(),
        &mask.key,
        &account_public_key,
        &nonce,
    )?;

    let output_statement = ConfidentialProofStatement {
        amount: final_amount,
        mask: mask.key,
        sender_public_nonce: output_public_nonce,
        minimum_value_promise: 0,
        encrypted_data,
        reveal_amount: max_fee,
        resource_view_key: None,
    };

    let reveal_proof = sdk.confidential_crypto_api().generate_withdraw_proof(
        &[unmasked_output],
        Amount::zero(),
        &output_statement,
        None,
    )?;

    let instructions = vec![Instruction::ClaimBurn {
        claim: Box::new(ConfidentialClaim {
            public_key: reciprocal_claim_public_key,
            output_address: commitment_substate_address
                .substate_id
                .as_unclaimed_confidential_output_address()
                .unwrap(),
            range_proof,
            proof_of_knowledge: RistrettoComSig::new(Commitment::from_public_key(&public_nonce), u, v),
            withdraw_proof: Some(reveal_proof),
        }),
    }];

    // ------------------------------
    let (tx_id, finalized) = finish_claiming(
        instructions,
        account_address,
        new_account_name,
        sdk,
        inputs,
        &account_public_key,
        max_fee,
        account_secret_key,
        &accounts_api,
        context,
    )
    .await?;

    Ok(ClaimBurnResponse {
        transaction_id: tx_id,
        fee: finalized.final_fee,
        result: finalized.finalize,
    })
}

async fn finish_claiming<T: WalletStore>(
    mut instructions: Vec<Instruction>,
    account_address: SubstateId,
    new_account_name: Option<String>,
    sdk: &DanWalletSdk<SqliteWalletStore, IndexerJsonRpcNetworkInterface>,
    mut inputs: Vec<SubstateRequirement>,
    account_public_key: &RistrettoPublicKey,
    max_fee: Amount,
    account_secret_key: DerivedKey<RistrettoPublicKey>,
    accounts_api: &tari_dan_wallet_sdk::apis::accounts::AccountsApi<'_, T>,
    context: &HandlerContext,
) -> Result<
    (
        tari_transaction::TransactionId,
        crate::services::TransactionFinalizedEvent,
    ),
    anyhow::Error,
> {
    instructions.push(Instruction::PutLastInstructionOutputOnWorkspace {
        key: b"bucket".to_vec(),
    });
    let account_component_address = account_address
        .as_component_address()
        .ok_or_else(|| anyhow!("Invalid account address"))?;
    if new_account_name.is_none() {
        // Add all versioned account child addresses as inputs unless the account is new
        let child_addresses = sdk.substate_api().load_dependent_substates(&[&account_address])?;
        inputs.extend(child_addresses.into_iter().map(Into::into));
        instructions.push(Instruction::CallMethod {
            component_address: account_component_address,
            method: "deposit".to_string(),
            args: args![Workspace("bucket")],
        });
    } else {
        instructions.push(Instruction::CreateAccount {
            owner_public_key: account_public_key.clone(),
            workspace_bucket: Some("bucket".to_string()),
        });
    }
    instructions.push(Instruction::CallMethod {
        component_address: account_component_address,
        method: "pay_fee".to_string(),
        args: args![max_fee],
    });
    let transaction = Transaction::builder()
        .with_fee_instructions(instructions)
        .with_inputs(inputs)
        .sign(&account_secret_key.key)
        .build();
    let is_first_account = accounts_api.count()? == 0;
    let mut events = context.notifier().subscribe();
    let tx_id = context
        .transaction_service()
        .submit_transaction_with_opts(
            transaction,
            vec![],
            new_account_name.map(|name| NewAccountInfo {
                name: Some(name),
                key_index: account_secret_key.key_index,
                is_default: is_first_account,
            }),
        )
        .await?;

    // Wait for the monitor to pick up the new or updated account
    let (finalized, _) = wait_for_result_and_account(&mut events, &tx_id, &account_address).await?;
    // let finalized = wait_for_result(&mut events, tx_id).await?;
    if let Some(reject) = finalized.finalize.reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.finalize.full_reject() {
        return Err(anyhow::anyhow!(
            "Fee transaction succeeded (fees charged) however the transaction failed: {}",
            reason
        ));
    }

    Ok((tx_id, finalized))
}

/// Mints free test coins into an account. If an account name is provided which does not exist, that account is created
pub async fn handle_create_free_test_coins(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountsCreateFreeTestCoinsRequest,
) -> Result<AccountsCreateFreeTestCoinsResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let AccountsCreateFreeTestCoinsRequest {
        account,
        amount,
        max_fee,
        key_id,
    } = req;

    let max_fee = max_fee.unwrap_or(DEFAULT_FEE);
    if max_fee.is_negative() {
        return Err(invalid_params("fee", Some("cannot be negative")));
    }

    let mut inputs = vec![
        SubstateRequirement::unversioned(XTR_FAUCET_COMPONENT_ADDRESS),
        SubstateRequirement::unversioned(XTR_FAUCET_VAULT_ADDRESS),
    ];
    let accounts_api = sdk.accounts_api();
    let (account_address, account_secret_key, new_account_name) =
        get_or_create_account(&account, &accounts_api, key_id, sdk, &mut inputs)?;

    let account_public_key = PublicKey::from_secret_key(&account_secret_key.key);

    let instructions = vec![Instruction::CallMethod {
        component_address: XTR_FAUCET_COMPONENT_ADDRESS,
        method: "take".to_string(),
        args: args![amount],
    }];

    // ------------------------------
    let (tx_id, finalized) = finish_claiming(
        instructions,
        account_address.clone(),
        new_account_name,
        sdk,
        inputs,
        &account_public_key,
        max_fee,
        account_secret_key,
        &accounts_api,
        context,
    )
    .await?;

    let account = accounts_api.get_account_by_address(&account_address)?;

    Ok(AccountsCreateFreeTestCoinsResponse {
        account,
        transaction_id: tx_id,
        amount,
        fee: max_fee,
        result: finalized.finalize,
        public_key: account_public_key,
    })
}

fn get_or_create_account<T: WalletStore>(
    account: &Option<ComponentAddressOrName>,
    accounts_api: &tari_dan_wallet_sdk::apis::accounts::AccountsApi<'_, T>,
    key_id: Option<u64>,
    sdk: &DanWalletSdk<SqliteWalletStore, IndexerJsonRpcNetworkInterface>,
    inputs: &mut Vec<SubstateRequirement>,
) -> Result<(SubstateId, DerivedKey<RistrettoPublicKey>, Option<String>), anyhow::Error> {
    let maybe_account = match account {
        Some(ref addr_or_name) => get_account(addr_or_name, accounts_api).optional()?,
        None => {
            let account = accounts_api
                .get_default()
                .optional()?
                .ok_or_else(|| anyhow::anyhow!("No default account found. Please set a default account."))?;

            Some(account)
        },
    };
    let (account_address, account_secret_key, new_account_name) = match maybe_account {
        Some(account) => {
            let key_index = key_id.unwrap_or(account.key_index);
            let account_secret_key = sdk
                .key_manager_api()
                .derive_key(key_manager::TRANSACTION_BRANCH, key_index)?;
            let account_substate = sdk.substate_api().get_substate(&account.address)?;
            inputs.push(account_substate.address.into());

            (account.address, account_secret_key, None)
        },
        None => {
            let name = account
                .as_ref()
                .unwrap()
                .name()
                .ok_or_else(|| anyhow!("Account name must be provided when creating a new account"))?;
            let account_secret_key = key_id
                .map(|idx| sdk.key_manager_api().derive_key(key_manager::TRANSACTION_BRANCH, idx))
                .unwrap_or_else(|| sdk.key_manager_api().next_key(key_manager::TRANSACTION_BRANCH))?;
            let account_pk = PublicKey::from_secret_key(&account_secret_key.key);

            let account_address = new_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &account_pk);

            // We have no involved substate addresses, so we need to add an output
            (account_address.into(), account_secret_key, Some(name.to_string()))
        },
    };
    Ok((account_address, account_secret_key, new_account_name))
}

#[allow(clippy::too_many_lines)]
pub async fn handle_transfer(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountsTransferRequest,
) -> Result<AccountsTransferResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let (account, mut inputs) = get_account_with_inputs(req.account, &sdk)?;

    // get the source account component address
    let source_account_address = account
        .address
        .as_component_address()
        .ok_or_else(|| anyhow!("Invalid account address"))?;

    // add the input for the source account vault substate
    let src_vault = sdk
        .accounts_api()
        .get_vault_by_resource(&account.address, &req.resource_address)?;
    let src_vault_substate = sdk.substate_api().get_substate(&src_vault.address)?;
    inputs.push(src_vault_substate.address);

    // add the input for the resource address to be transfered
    let resource_substate = sdk
        .substate_api()
        .scan_for_substate(&SubstateId::Resource(req.resource_address), None)
        .await?;
    let resource_substate_address = SubstateRequirement::new(
        resource_substate.address.substate_id.clone(),
        Some(resource_substate.address.version),
    );
    inputs.push(resource_substate.address);

    let mut instructions = vec![];
    let mut fee_instructions = vec![];

    let destination_account_address =
        new_component_address_from_public_key(&ACCOUNT_TEMPLATE_ADDRESS, &req.destination_public_key);
    let existing_account = sdk
        .substate_api()
        .scan_for_substate(&SubstateId::Component(destination_account_address), None)
        .await
        .optional()?;

    if let Some(ValidatorScanResult { address, .. }) = existing_account {
        inputs.push(address);
    } else {
        instructions.push(Instruction::CreateAccount {
            owner_public_key: req.destination_public_key,
            workspace_bucket: None,
        });
    }

    if let Some(ref badge) = req.proof_from_badge_resource {
        instructions.extend([
            Instruction::CallMethod {
                component_address: source_account_address,
                method: "create_proof_for_resource".to_string(),
                args: args![badge],
            },
            Instruction::PutLastInstructionOutputOnWorkspace { key: b"proof".to_vec() },
        ]);
    }

    // build the transaction
    let max_fee = req.max_fee.unwrap_or(DEFAULT_FEE);
    instructions.extend([
        Instruction::CallMethod {
            component_address: source_account_address,
            method: "withdraw".to_string(),
            args: args![req.resource_address, req.amount],
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"bucket".to_vec(),
        },
        Instruction::CallMethod {
            component_address: destination_account_address,
            method: "deposit".to_string(),
            args: args![Workspace("bucket")],
        },
    ]);

    if req.proof_from_badge_resource.is_some() {
        instructions.push(Instruction::DropAllProofsInWorkspace);
    }

    fee_instructions.extend([Instruction::CallMethod {
        component_address: source_account_address,
        method: "pay_fee".to_string(),
        args: args![max_fee],
    }]);

    let account_secret_key = sdk
        .key_manager_api()
        .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;

    let transaction = Transaction::builder()
        .with_fee_instructions(fee_instructions)
        .with_instructions(instructions)
        .with_inputs(vec![resource_substate_address])
        .sign(&account_secret_key.key)
        .build();

    let required_inputs = inputs.into_iter().map(Into::into).collect();
    // If dry run we can return the result immediately
    if req.dry_run {
        let transaction_id = *transaction.id();
        let execute_result = context
            .transaction_service()
            .submit_dry_run_transaction(transaction, required_inputs)
            .await?;
        return Ok(AccountsTransferResponse {
            transaction_id,
            fee: execute_result.fee_receipt.total_fees_paid,
            fee_refunded: execute_result.fee_receipt.total_fee_payment - execute_result.fee_receipt.total_fees_paid,
            result: execute_result,
        });
    }

    // Otherwise submit and wait for a result
    let mut events = context.notifier().subscribe();
    let tx_id = context
        .transaction_service()
        .submit_transaction(transaction, required_inputs)
        .await?;

    let finalized = wait_for_result(&mut events, tx_id).await?;

    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.finalize.reject() {
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

    Ok(AccountsTransferResponse {
        transaction_id: tx_id,
        fee: finalized.final_fee,
        fee_refunded: max_fee - finalized.final_fee,
        result: finalized.finalize,
    })
}

pub async fn handle_confidential_transfer(
    context: &HandlerContext,
    token: Option<String>,
    req: ConfidentialTransferRequest,
) -> Result<ConfidentialTransferResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let notifier = context.notifier().clone();

    if req.amount.is_negative() {
        return Err(invalid_params("amount", Some("must be positive")));
    }
    let transaction_service = context.transaction_service().clone();

    task::spawn(async move {
        let account = get_account_or_default(req.account, &sdk.accounts_api())?;

        let transfer = sdk
            .confidential_transfer_api()
            .transfer(TransferParams {
                from_account: account.address.as_component_address().unwrap(),
                input_selection: req.input_selection,
                amount: req.amount,
                destination_public_key: req.destination_public_key,
                resource_address: req.resource_address,
                max_fee: req.max_fee.unwrap_or(DEFAULT_FEE),
                output_to_revealed: req.output_to_revealed,
                proof_from_resource: req.proof_from_badge_resource,
                is_dry_run: req.dry_run,
            })
            .await?;

        if req.dry_run {
            let transaction_id = *transfer.transaction.id();
            let finalize = transaction_service
                .submit_dry_run_transaction(
                    transfer.transaction,
                    transfer.inputs.into_iter().map(Into::into).collect(),
                )
                .await?;
            return Ok(ConfidentialTransferResponse {
                transaction_id,
                fee: finalize.fee_receipt.total_fees_paid,
                result: finalize,
            });
        }

        let mut events = notifier.subscribe();
        let tx_id = transaction_service
            .submit_transaction(
                transfer.transaction,
                transfer.inputs.into_iter().map(Into::into).collect(),
            )
            .await?;

        notifier.notify(TransactionSubmittedEvent {
            transaction_id: tx_id,
            new_account: None,
        });

        let finalized = wait_for_result(&mut events, tx_id).await?;
        if let Some(reject) = finalized.finalize.result.reject() {
            return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
        }
        if let Some(reason) = finalized.finalize.reject() {
            return Err(anyhow::anyhow!(
                "Fee transaction succeeded (fees charged) however the transaction failed: {}",
                reason
            ));
        }

        Ok(ConfidentialTransferResponse {
            transaction_id: tx_id,
            fee: finalized.final_fee,
            result: finalized.finalize,
        })
    })
    .await?
}

fn is_account_substate(substate: &Substate) -> bool {
    substate
        .substate_value()
        .component()
        .filter(|c| c.template_address == ACCOUNT_TEMPLATE_ADDRESS)
        .is_some()
}
