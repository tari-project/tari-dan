//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::convert::{TryFrom, TryInto};

use anyhow::anyhow;
use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use base64;
use log::*;
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey};
use tari_crypto::{
    commitment::{HomomorphicCommitment as Commitment, HomomorphicCommitmentFactory},
    keys::PublicKey as _,
    ristretto::RistrettoComSig,
};
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_dan_wallet_sdk::{
    apis::{jwt::JrpcPermission, key_manager, substate::ValidatorScanResult},
    confidential::{get_commitment_factory, ConfidentialProofStatement},
    models::{ConfidentialOutputModel, OutputStatus, VersionedSubstateAddress},
    DanWalletSdk,
};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_engine_types::{
    component::new_component_address_from_parts,
    confidential::ConfidentialClaim,
    instruction::Instruction,
    substate::{Substate, SubstateAddress},
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, NonFungibleAddress, UnclaimedConfidentialOutputAddress},
    prelude::{ComponentAddress, ResourceType, CONFIDENTIAL_TARI_RESOURCE_ADDRESS},
    Hash,
};
use tari_transaction::Transaction;
use tari_utilities::ByteArray;
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
        BalanceEntry,
        ClaimBurnRequest,
        ClaimBurnResponse,
        ConfidentialTransferRequest,
        ConfidentialTransferResponse,
        RevealFundsRequest,
        RevealFundsResponse,
        TransferRequest,
        TransferResponse,
    },
    ComponentAddressOrName,
};
use tokio::{sync::broadcast, task};

use super::context::HandlerContext;
use crate::{
    handlers::{get_account, get_account_or_default},
    indexer_jrpc_impl::IndexerJsonRpcNetworkInterface,
    services::{NewAccountInfo, TransactionFinalizedEvent, TransactionSubmittedEvent, WalletEvent},
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
        .locate_dependent_substates(&[&default_account.address])
        .await?;

    let signing_key_index = default_account.key_index;
    let signing_key = key_manager_api.derive_key(key_manager::TRANSACTION_BRANCH, signing_key_index)?;

    let owner_key = key_manager_api.next_key(key_manager::TRANSACTION_BRANCH)?;
    let owner_pk = PublicKey::from_secret_key(&owner_key.k);
    let owner_token =
        NonFungibleAddress::from_public_key(RistrettoPublicKeyBytes::from_bytes(owner_pk.as_bytes()).unwrap());

    info!(
        target: LOG_TARGET,
        "Creating account with owner token {}. Fees are paid using account '{}' {}",
        owner_pk,
        default_account.name,
        default_account.address
    );

    let fee = req.fee.unwrap_or(DEFAULT_FEE);
    let transaction = Transaction::builder()
        .fee_transaction_pay_from_component(default_account.address.as_component_address().unwrap(), fee)
        .call_function(*ACCOUNT_TEMPLATE_ADDRESS, "create", args![owner_token])
        .with_inputs(
            inputs
                .iter()
                .map(|addr| ShardId::from_address(&addr.address, addr.version))
                .collect(),
        )
        .sign(&signing_key.k)
        .build();

    let tx_hash = sdk.transaction_api().submit_transaction(transaction).await?;

    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent {
        hash: tx_hash,
        new_account: Some(NewAccountInfo {
            name: req.account_name,
            key_index: owner_key.key_index,
            is_default: req.is_default,
        }),
    });

    let event = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = event.finalize.result.reject() {
        return Err(anyhow!("Create account transaction rejected: {}", reject));
    }
    if let Some(reason) = event.transaction_failure {
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
            let pk = PublicKey::from_secret_key(&key.k);
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
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let account_address = account.address.as_component_address().unwrap();
    let transaction = Transaction::builder()
        .fee_transaction_pay_from_component(account_address, req.fee.unwrap_or(DEFAULT_FEE))
        .call_method(account_address, &req.method, req.args)
        .with_inputs(inputs)
        .sign(&signing_key.k)
        .build();

    let tx_hash = sdk.transaction_api().submit_transaction(transaction).await?;
    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent {
        hash: tx_hash,
        new_account: None,
    });

    let mut finalized = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reject) = finalized.transaction_failure {
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
    let outputs_api = sdk.confidential_outputs_api();

    let mut balances = Vec::with_capacity(vaults.len());
    for vault in vaults {
        let confidential_balance = if matches!(vault.resource_type, ResourceType::Confidential) {
            Amount::try_from(outputs_api.get_unspent_balance(&vault.address)?)?
        } else {
            Amount::zero()
        };

        balances.push(BalanceEntry {
            vault_address: vault.address,
            resource_address: vault.resource_address,
            balance: vault.balance,
            resource_type: vault.resource_type,
            confidential_balance,
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
    let public_key = PublicKey::from_secret_key(&key.k);
    Ok(AccountGetResponse { account, public_key })
}

pub async fn handle_get_default(
    context: &HandlerContext,
    token: Option<String>,
    _req: AccountGetDefaultRequest,
) -> Result<AccountGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let account = get_account_or_default(None, &sdk.accounts_api())?;
    let km = sdk.key_manager_api();
    let key = km.derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
    let public_key = PublicKey::from_secret_key(&key.k);
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

    task::spawn(async move {
        let mut inputs = vec![];

        let account = get_account_or_default(req.account, &sdk.accounts_api())?;
        // Add the account component
        let account_substate = sdk.substate_api().get_substate(&account.address)?;

        inputs.push(account_substate.address);

        // Add all versioned account child addresses as inputs
        let child_addresses = sdk.substate_api().load_dependent_substates(&[&account.address])?;
        inputs.extend(child_addresses);

        let vault = sdk
            .accounts_api()
            .get_vault_by_resource(&account.address, &CONFIDENTIAL_TARI_RESOURCE_ADDRESS)?;

        let fee = req.fee.unwrap_or(DEFAULT_FEE);
        let amount_to_reveal = req.amount_to_reveal + if req.pay_fee_from_reveal { fee } else { 0.into() };

        let proof_id = sdk.confidential_outputs_api().add_proof(&vault.address)?;

        let (inputs, input_value) = sdk.confidential_outputs_api().lock_outputs_by_amount(
            &vault.address,
            amount_to_reveal.as_u64_checked().unwrap(),
            proof_id,
        )?;
        let input_amount = Amount::try_from(input_value)?;

        let account_key = sdk
            .key_manager_api()
            .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
        let account_public_key = PublicKey::from_secret_key(&account_key.k);

        let (output_mask, public_nonce) = sdk
            .confidential_crypto_api()
            .derive_output_mask_for_destination(&account_public_key);

        let output_statement = ConfidentialProofStatement {
            amount: input_amount - amount_to_reveal,
            mask: output_mask,
            sender_public_nonce: public_nonce,
            minimum_value_promise: 0,
            reveal_amount: amount_to_reveal,
        };

        let inputs = sdk
            .confidential_outputs_api()
            .resolve_output_masks(inputs, key_manager::TRANSACTION_BRANCH)?;

        let reveal_proof = sdk
            .confidential_crypto_api()
            .generate_withdraw_proof(&inputs, &output_statement, None)?;

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
                    args: args![*CONFIDENTIAL_TARI_RESOURCE_ADDRESS, reveal_proof],
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
                    args: args![fee],
                },
            ]);
        } else {
            builder = builder
                .fee_transaction_pay_from_component(account_address, fee)
                .call_method(account_address, "withdraw_confidential", args![
                    *CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
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
            .map(|addr| ShardId::from_address(&addr.address, addr.version))
            .collect();

        let transaction = builder.with_inputs(inputs).sign(&account_key.k).build();

        sdk.confidential_outputs_api()
            .proofs_set_transaction_hash(proof_id, *transaction.hash())?;

        let tx_hash = sdk.transaction_api().submit_transaction(transaction).await?;

        let mut events = notifier.subscribe();
        notifier.notify(TransactionSubmittedEvent {
            hash: tx_hash,
            new_account: None,
        });

        let finalized = wait_for_result(&mut events, tx_hash).await?;
        if let Some(reason) = finalized.transaction_failure {
            return Err(anyhow::anyhow!("Transaction failed: {}", reason));
        }

        Ok(RevealFundsResponse {
            hash: tx_hash,
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
    context
        .wallet_sdk()
        .jwt_api()
        .check_auth(token, &[JrpcPermission::Admin])?;
    let ClaimBurnRequest {
        account,
        claim_proof,
        fee,
    } = req;
    let fee = fee.unwrap_or(DEFAULT_FEE);

    let reciprocal_claim_public_key = PublicKey::from_bytes(
        &base64::decode(
            claim_proof["reciprocal_claim_public_key"]
                .as_str()
                .ok_or_else(|| invalid_params("reciprocal_claim_public_key", None))?,
        )
        .map_err(|e| invalid_params("reciprocal_claim_public_key", Some(e.to_string())))?,
    )?;
    let commitment = base64::decode(
        claim_proof["commitment"]
            .as_str()
            .ok_or_else(|| invalid_params("commitment", None))?,
    )
    .map_err(|e| invalid_params("commitment", Some(e.to_string())))?;
    let range_proof = base64::decode(
        claim_proof["range_proof"]
            .as_str()
            .or_else(|| claim_proof["rangeproof"].as_str())
            .ok_or_else(|| invalid_params("range_proof", None))?,
    )
    .map_err(|e| invalid_params("range_proof", Some(e.to_string())))?;

    let public_nonce = PublicKey::from_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["public_nonce"]
                .as_str()
                .ok_or_else(|| invalid_params("ownership_proof.public_nonce", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.public_nonce", Some(e.to_string())))?,
    )?;
    let u = PrivateKey::from_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["u"]
                .as_str()
                .ok_or_else(|| invalid_params("ownership_proof.u", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.u", Some(e.to_string())))?,
    )?;
    let v = PrivateKey::from_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["v"]
                .as_str()
                .ok_or_else(|| invalid_params("ownership_proof.v", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.v", Some(e.to_string())))?,
    )?;

    let sdk = context.wallet_sdk();

    let requested_account;
    if let Some(a) = account {
        match a {
            ComponentAddressOrName::ComponentAddress(addr) => {
                requested_account = sdk.accounts_api().get_account_by_address(&addr.into())?;
            },
            ComponentAddressOrName::Name(name) => {
                requested_account = sdk.accounts_api().get_account_by_name(&name)?;
            },
        }
    } else {
        requested_account = sdk.accounts_api().get_default().optional()?.ok_or_else(|| {
            anyhow::anyhow!("No default account set. Please specify an account to use for this transaction.")
        })?;
    }
    let account = requested_account;
    let account_secret_key = sdk
        .key_manager_api()
        .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
    let account_public_key = PublicKey::from_secret_key(&account_secret_key.k);

    info!(
        target: LOG_TARGET,
        "Signing claim burn with key {}. This must be the same as the claiming key used in the burn transaction.",
        account_public_key
    );

    let mut inputs = vec![];

    // Add the account component
    let account_substate = sdk.substate_api().get_substate(&account.address)?;
    inputs.push(account_substate.address);

    // Add all versioned account child addresses as inputs
    let child_addresses = sdk.substate_api().load_dependent_substates(&[&account.address])?;
    inputs.extend(child_addresses);

    // // TODO: we assume that all inputs will be consumed and produce a new output however this is only the case when
    // the //       object is mutated
    // let outputs = inputs
    //     .iter()
    //     .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version + 1))
    //     .collect::<Vec<_>>();

    // add the commitment substate address as input to the claim burn transaction
    let commitment_substate_address = VersionedSubstateAddress {
        address: SubstateAddress::UnclaimedConfidentialOutput(UnclaimedConfidentialOutputAddress::try_from(
            commitment.as_slice(),
        )?),
        version: 0,
    };
    inputs.push(commitment_substate_address.clone());

    info!(
        target: LOG_TARGET,
        "Loaded {} inputs for claim burn transaction on account: {}",
        inputs.len(),
        account.name
    );

    // We have to unmask the commitment to allow us to reveal funds for the fee payment
    let ValidatorScanResult { substate: output, .. } = sdk
        .substate_api()
        .scan_for_substate(
            &commitment_substate_address.address,
            Some(commitment_substate_address.version),
        )
        .await?;
    let output = output.into_unclaimed_confidential_output().unwrap();
    let unmasked_output = sdk.confidential_crypto_api().unblind_output(
        &output.commitment,
        &output.encrypted_value,
        &account_secret_key.k,
        &reciprocal_claim_public_key,
    )?;

    let (mask, output_public_nonce) = sdk
        .confidential_crypto_api()
        .derive_output_mask_for_destination(&account_public_key);

    let output_statement = ConfidentialProofStatement {
        amount: Amount::try_from(unmasked_output.value)? - fee,
        mask,
        sender_public_nonce: output_public_nonce,
        minimum_value_promise: 0,
        reveal_amount: fee,
    };

    let reveal_proof =
        sdk.confidential_crypto_api()
            .generate_withdraw_proof(&[unmasked_output], &output_statement, None)?;

    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let transaction = Transaction::builder()
        .with_fee_instructions(vec![
            Instruction::ClaimBurn {
                claim: Box::new(ConfidentialClaim {
                    public_key: reciprocal_claim_public_key,
                    output_address: commitment_substate_address
                        .address
                        .as_unclaimed_confidential_output_address()
                        .unwrap(),
                    range_proof,
                    proof_of_knowledge: RistrettoComSig::new(Commitment::from_public_key(&public_nonce), u, v),
                    withdraw_proof: Some(reveal_proof),
                }),
            },
            Instruction::PutLastInstructionOutputOnWorkspace { key: b"burn".to_vec() },
            Instruction::CallMethod {
                component_address: account.address.clone().as_component_address().unwrap(),
                method: "deposit".to_string(),
                args: args![Workspace("burn")],
            },
            Instruction::CallMethod {
                component_address: account.address.clone().as_component_address().unwrap(),
                method: "pay_fee".to_string(),
                args: args![fee],
            },
        ])
        .with_inputs(inputs)
        .sign(&account_secret_key.k)
        .build();

    let tx_hash = sdk.transaction_api().submit_transaction(transaction).await?;

    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent {
        hash: tx_hash,
        new_account: None,
    });

    let finalized = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.transaction_failure {
        return Err(anyhow::anyhow!(
            "Fee transaction succeeded (fees charged) however the transaction failed: {}",
            reason
        ));
    }

    Ok(ClaimBurnResponse {
        hash: tx_hash,
        fee: finalized.final_fee,
        result: finalized.finalize,
    })
}

/// Mints free test coins into an account. If an account name is provided which does not exist, that account is created
#[allow(clippy::too_many_lines)]
pub async fn handle_create_free_test_coins(
    context: &HandlerContext,
    token: Option<String>,
    req: AccountsCreateFreeTestCoinsRequest,
) -> Result<AccountsCreateFreeTestCoinsResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let accounts_api = sdk.accounts_api();
    let mut inputs = vec![];

    // Get the account if one is specified and exists.
    let maybe_account = match req.account {
        Some(ref addr_or_name) => get_account(addr_or_name, &accounts_api).optional()?,
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
            let account_secret_key = sdk
                .key_manager_api()
                .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
            let account_substate = sdk.substate_api().get_substate(&account.address)?;
            inputs.push(account_substate.address);
            (account.address, account_secret_key, None)
        },
        None => {
            let name = req
                .account
                .as_ref()
                .unwrap()
                .name()
                .ok_or_else(|| anyhow!("Account name must be provided when creating a new account"))?;
            let account_secret_key = sdk.key_manager_api().next_key(key_manager::TRANSACTION_BRANCH)?;
            let account_pk = PublicKey::from_secret_key(&account_secret_key.k);

            let component_id = Hash::try_from(account_pk.as_bytes())?;
            let account_address = new_component_address_from_parts(&ACCOUNT_TEMPLATE_ADDRESS, &component_id);

            inputs.push(VersionedSubstateAddress {
                address: account_address.into(),
                version: 0,
            });
            (account_address.into(), account_secret_key, Some(name.to_string()))
        },
    };

    let account_public_key = PublicKey::from_secret_key(&account_secret_key.k);
    let output = sdk
        .confidential_crypto_api()
        .generate_output_for_dest(&account_public_key, req.amount)?;

    let mut instructions = vec![
        // TODO: We create double what is expected, amount confidential and amount revealed. Should let the caller
        //       specify these values separately.
        Instruction::CreateFreeTestCoins {
            revealed_amount: req.amount,
            output: Some(output),
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"free_test_coins".to_vec(),
        },
    ];

    let account_component_address = account_address
        .as_component_address()
        .ok_or_else(|| anyhow!("Invalid account address"))?;

    if new_account_name.is_none() {
        instructions.push(Instruction::CallMethod {
            component_address: account_component_address,
            method: "deposit".to_string(),
            args: args![Workspace("free_test_coins")],
        });
    } else {
        let owner_token = NonFungibleAddress::from_public_key(
            RistrettoPublicKeyBytes::from_bytes(account_public_key.as_bytes()).unwrap(),
        );
        instructions.push(Instruction::CallFunction {
            template_address: *ACCOUNT_TEMPLATE_ADDRESS,
            function: "create_with_bucket".to_string(),
            args: args![owner_token, Workspace("free_test_coins")],
        });
    }

    // Pay fees from the account
    let fee = req.fee.unwrap_or(DEFAULT_FEE);
    instructions.push(Instruction::CallMethod {
        component_address: account_component_address,
        method: "pay_fee".to_string(),
        args: args![fee],
    });

    // Add the account component
    // let account_substate = sdk.substate_api().get_substate(&account_address)?;
    // inputs.push(account_substate.address);

    // Add all versioned account child addresses as inputs unless the account is new
    if new_account_name.is_none() {
        let child_addresses = sdk.substate_api().load_dependent_substates(&[&account_address])?;
        inputs.extend(child_addresses);
    }

    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let transaction = Transaction::builder()
        .with_fee_instructions(instructions)
        .with_inputs(inputs)
        .sign(&account_secret_key.k)
        .build();

    let tx_hash = sdk.transaction_api().submit_transaction(transaction).await?;

    let is_first_account = accounts_api.count()? == 0;
    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent {
        hash: tx_hash,
        new_account: new_account_name.map(|name| NewAccountInfo {
            name: Some(name),
            key_index: account_secret_key.key_index,
            is_default: is_first_account,
        }),
    });

    let finalized = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.transaction_failure {
        return Err(anyhow::anyhow!(
            "Fee transaction succeeded (fees charged) however the transaction failed: {}",
            reason
        ));
    }

    Ok(AccountsCreateFreeTestCoinsResponse {
        hash: tx_hash,
        amount: req.amount,
        fee,
        result: finalized.finalize,
    })
}

#[allow(clippy::too_many_lines)]
pub async fn handle_transfer(
    context: &HandlerContext,
    token: Option<String>,
    req: TransferRequest,
) -> Result<TransferResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let account = get_account_or_default(req.account, &sdk.accounts_api())?;
    context
        .account_monitor()
        .refresh_account(account.address.clone())
        .await?;

    let account_secret_key = sdk
        .key_manager_api()
        .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;

    let mut instructions = vec![];
    let mut inputs = vec![];

    // get the source account component address
    let source_account_address = account
        .address
        .as_component_address()
        .ok_or_else(|| anyhow!("Invalid account address"))?;

    // add the input for the source account component substate
    let account_substate = sdk.substate_api().get_substate(&account.address)?;
    inputs.push(account_substate.address);

    // Add all versioned account child addresses as inputs
    let child_addresses = sdk.substate_api().load_dependent_substates(&[&account.address])?;
    inputs.extend(child_addresses);

    // add the input for the source account vault substate
    let src_vault = sdk
        .accounts_api()
        .get_vault_by_resource(&account.address, &req.resource_address)?;
    let src_vault_substate = sdk.substate_api().get_substate(&src_vault.address)?;
    inputs.push(src_vault_substate.address);

    // add the input for the resource address to be transfered
    let resource_substate = sdk
        .substate_api()
        .scan_for_substate(&SubstateAddress::Resource(req.resource_address), None)
        .await?;
    inputs.push(resource_substate.address);

    // get destination account information
    let destination_account_address =
        get_or_create_account_address(&sdk, &req.destination_public_key, &mut inputs, &mut instructions).await?;

    // calculate inputs shard ids
    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    // build the transaction
    let fee = req.fee.unwrap_or(DEFAULT_FEE);
    instructions.append(&mut vec![
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
        Instruction::CallMethod {
            component_address: source_account_address,
            method: "pay_fee".to_string(),
            args: args![fee],
        },
    ]);
    let transaction = Transaction::builder()
        .with_fee_instructions(instructions)
        .with_inputs(inputs)
        // potentially we can create new outputs for the destination account with its vault
        .with_new_outputs(2)
        .sign(&account_secret_key.k)
        .build();

    // send the transaction
    let tx_hash = sdk.transaction_api().submit_transaction(transaction).await?;

    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent {
        hash: tx_hash,
        new_account: None,
    });

    let finalized = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.transaction_failure {
        return Err(anyhow::anyhow!(
            "Fee transaction succeeded (fees charged) however the transaction failed: {}",
            reason
        ));
    }

    Ok(TransferResponse {
        hash: tx_hash,
        fee,
        result: finalized.finalize,
    })
}

async fn get_or_create_account_address(
    sdk: &DanWalletSdk<SqliteWalletStore, IndexerJsonRpcNetworkInterface>,
    public_key: &PublicKey,
    inputs: &mut Vec<VersionedSubstateAddress>,
    instructions: &mut Vec<Instruction>,
) -> Result<ComponentAddress, anyhow::Error> {
    // calculate the account component address from the public key
    let component_id = Hash::try_from(public_key.as_bytes())?;
    let account_address = new_component_address_from_parts(&ACCOUNT_TEMPLATE_ADDRESS, &component_id);

    let account_scan = sdk
        .substate_api()
        .scan_for_substate(&SubstateAddress::Component(account_address), None)
        .await
        .optional()?;

    match account_scan {
        Some(res) => {
            // the account already exists in the network, so we must add the substate address to the inputs
            inputs.push(res.address);
        },
        None => {
            // the account does not exists, so we must add a instruction to create it, matching the public key
            let owner_token = NonFungibleAddress::from_public_key(
                RistrettoPublicKeyBytes::from_bytes(public_key.as_bytes()).unwrap(),
            );
            instructions.insert(0, Instruction::CallFunction {
                template_address: *ACCOUNT_TEMPLATE_ADDRESS,
                function: "create".to_string(),
                args: args![owner_token],
            });
        },
    };

    Ok(account_address)
}

#[allow(clippy::too_many_lines)]
pub async fn handle_confidential_transfer(
    context: &HandlerContext,
    token: Option<String>,
    req: ConfidentialTransferRequest,
) -> Result<ConfidentialTransferResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    let notifier = context.notifier().clone();

    task::spawn(async move {
        let outputs_api = sdk.confidential_outputs_api();
        let crypto_api = sdk.confidential_crypto_api();
        let accounts_api = sdk.accounts_api();
        let substate_api = sdk.substate_api();
        let key_manager_api = sdk.key_manager_api();

        let mut instructions = vec![];

        // -------------------------------- Load up known substates -------------------------------- //
        let account = get_account_or_default(req.account, &accounts_api)?;
        let account_secret = key_manager_api.derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
        let source_component_address = account
            .address
            .as_component_address()
            .ok_or_else(|| anyhow!("Invalid component address for source address"))?;

        let account_substate = substate_api.get_substate(&account.address)?;
        let src_vault = accounts_api.get_vault_by_resource(&account.address, &CONFIDENTIAL_TARI_RESOURCE_ADDRESS)?;
        let src_vault_substate = substate_api.get_substate(&src_vault.address)?;

        // get destination account information
        let mut destination_account_inputs = vec![];
        let destination_account_address = get_or_create_account_address(
            &sdk,
            &req.destination_public_key,
            &mut destination_account_inputs,
            &mut instructions,
        )
        .await?;

        // -------------------------------- Lock outputs for spending -------------------------------- //
        let total_amount = req.fee.unwrap_or(DEFAULT_FEE) + req.amount;
        let proof_id = outputs_api.add_proof(&src_vault.address)?;
        let (confidential_inputs, total_input_value) =
            outputs_api.lock_outputs_by_amount(&src_vault.address, total_amount.as_u64_checked().unwrap(), proof_id)?;

        let (output_mask, public_nonce) = crypto_api.derive_output_mask_for_destination(&req.destination_public_key);

        let output_statement = ConfidentialProofStatement {
            amount: req.amount,
            mask: output_mask,
            sender_public_nonce: public_nonce,
            minimum_value_promise: 0,
            reveal_amount: Amount::zero(),
        };

        let change_amount = total_input_value - req.amount.as_u64_checked().unwrap();
        let maybe_change_statement = if change_amount > 0 {
            let account_pk = PublicKey::from_secret_key(&account_secret.k);
            let (change_mask, public_nonce) = crypto_api.derive_output_mask_for_destination(&account_pk);

            outputs_api.add_output(ConfidentialOutputModel {
                account_address: account.address,
                vault_address: src_vault.address,
                commitment: get_commitment_factory().commit_value(&change_mask, change_amount),
                value: change_amount,
                sender_public_nonce: Some(public_nonce.clone()),
                secret_key_index: account.key_index,
                public_asset_tag: None,
                status: OutputStatus::LockedUnconfirmed,
                locked_by_proof: Some(proof_id),
            })?;

            Some(ConfidentialProofStatement {
                amount: change_amount.try_into()?,
                mask: change_mask,
                sender_public_nonce: public_nonce,
                minimum_value_promise: 0,
                reveal_amount: Amount::zero(),
            })
        } else {
            None
        };

        let confidential_inputs =
            outputs_api.resolve_output_masks(confidential_inputs, key_manager::TRANSACTION_BRANCH)?;

        let proof = crypto_api.generate_withdraw_proof(
            &confidential_inputs,
            &output_statement,
            maybe_change_statement.as_ref(),
        )?;

        // destination account inputs
        let mut shard_inputs: Vec<ShardId> = destination_account_inputs
            .into_iter()
            .map(|s| ShardId::from_address(&s.address, s.version))
            .collect();

        // Source account input
        shard_inputs.push(ShardId::from_address(
            &account_substate.address.address,
            account_substate.address.version,
        ));
        shard_inputs.push(ShardId::from_address(
            &src_vault_substate.address.address,
            src_vault_substate.address.version,
        ));

        instructions.append(&mut vec![
            Instruction::CallMethod {
                component_address: source_component_address,
                method: "withdraw_confidential".to_string(),
                args: args![req.resource_address, proof],
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

        let transaction = Transaction::builder()
            .fee_transaction_pay_from_component(source_component_address, req.fee.unwrap_or(DEFAULT_FEE))
            .with_instructions(instructions)
            .with_inputs(shard_inputs)
            .sign(&account_secret.k)
            .build();

        outputs_api.proofs_set_transaction_hash(proof_id, *transaction.hash())?;

        let tx_hash = sdk.transaction_api().submit_transaction(transaction).await?;

        let mut events = notifier.subscribe();
        notifier.notify(TransactionSubmittedEvent {
            hash: tx_hash,
            new_account: None,
        });

        let finalized = wait_for_result(&mut events, tx_hash).await?;
        if let Some(reject) = finalized.finalize.result.reject() {
            return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
        }
        if let Some(reason) = finalized.transaction_failure {
            return Err(anyhow::anyhow!(
                "Fee transaction succeeded (fees charged) however the transaction failed: {}",
                reason
            ));
        }

        Ok(ConfidentialTransferResponse {
            hash: tx_hash,
            fee: finalized.final_fee,
            result: finalized.finalize,
        })
    })
    .await?
}

async fn wait_for_result(
    events: &mut broadcast::Receiver<WalletEvent>,
    tx_hash: FixedHash,
) -> Result<TransactionFinalizedEvent, anyhow::Error> {
    loop {
        let wallet_event = events.recv().await?;
        match wallet_event {
            WalletEvent::TransactionFinalized(event) if event.hash == tx_hash => return Ok(event),
            _ => {},
        }
    }
}

fn invalid_params(field: &str, details: Option<String>) -> JsonRpcError {
    JsonRpcError::new(
        JsonRpcErrorReason::InvalidParams,
        format!(
            "Invalid param '{}'{}",
            field,
            details.map(|d| format!(": {}", d)).unwrap_or_default()
        ),
        serde_json::Value::Null,
    )
}

fn is_account_substate(substate: &Substate) -> bool {
    substate
        .substate_value()
        .component()
        .filter(|c| c.template_address == *ACCOUNT_TEMPLATE_ADDRESS)
        .is_some()
}
