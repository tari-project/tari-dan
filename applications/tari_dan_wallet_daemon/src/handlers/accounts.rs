//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::convert::TryFrom;

use anyhow::anyhow;
use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use base64;
use log::*;
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey};
use tari_crypto::{commitment::HomomorphicCommitment as Commitment, keys::PublicKey as _, ristretto::RistrettoComSig};
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_dan_wallet_sdk::{apis::key_manager, models::VersionedSubstateAddress};
use tari_engine_types::{
    commit_result::{FinalizeResult, TransactionResult},
    confidential::ConfidentialClaim,
    instruction::Instruction,
    substate::SubstateAddress,
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, NonFungibleAddress, UnclaimedConfidentialOutputAddress},
    prelude::ResourceType,
};
use tari_transaction::Transaction;
use tari_utilities::ByteArray;
use tari_wallet_daemon_client::types::{
    AccountByNameRequest,
    AccountByNameResponse,
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
};
use tokio::sync::broadcast;

use super::context::HandlerContext;
use crate::services::{TransactionSubmittedEvent, WalletEvent};

const LOG_TARGET: &str = "tari::dan_wallet_daemon::handlers::transaction";

pub async fn handle_create(
    context: &HandlerContext,
    req: AccountsCreateRequest,
) -> Result<AccountsCreateResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();

    if let Some(name) = req.account_name.as_ref() {
        if sdk.accounts_api().get_account_by_name(name).optional()?.is_some() {
            return Err(anyhow!("Account name '{}' already exists", name));
        }
    }

    let (key_index, signing_key) = sdk
        .key_manager_api()
        .get_key_or_active(key_manager::TRANSACTION_BRANCH, req.signing_key_index)?;
    let owner_pk = sdk
        .key_manager_api()
        .get_public_key(key_manager::TRANSACTION_BRANCH, req.signing_key_index)?;
    let owner_token =
        NonFungibleAddress::from_public_key(RistrettoPublicKeyBytes::from_bytes(owner_pk.as_bytes()).unwrap());

    info!(target: LOG_TARGET, "Creating account with owner token {}", owner_pk);

    let mut builder = Transaction::builder();
    builder
        .add_instruction(Instruction::CallFunction {
            template_address: ACCOUNT_TEMPLATE_ADDRESS,
            function: "create".to_string(),
            args: args![owner_token],
        })
        .with_fee(req.fee.unwrap_or(1))
        .with_new_outputs(1)
        .sign(&signing_key.k);

    let transaction = builder.build();

    let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;

    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent { hash: tx_hash });

    let finalized = wait_for_result(&mut events, tx_hash).await?;
    let diff = finalized.result.accept().unwrap();
    let (addr, _) = diff
        .up_iter()
        .find(|(addr, _)| addr.is_component())
        .ok_or_else(|| anyhow!("Create account transaction accepted but no component address was returned"))?;

    sdk.accounts_api()
        .add_account(req.account_name.as_deref(), addr, key_index)?;

    Ok(AccountsCreateResponse {
        address: addr.clone(),
        public_key: owner_pk,
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    req: AccountsListRequest,
) -> Result<AccountsListResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let accounts = sdk.accounts_api().get_many(req.offset, req.limit)?;
    let total = sdk.accounts_api().count()?;
    let km = sdk.key_manager_api();
    let accounts = accounts
        .into_iter()
        .map(|a| {
            let key = km.derive_key(key_manager::TRANSACTION_BRANCH, a.key_index)?;
            let pk = PublicKey::from_secret_key(&key.k);
            Ok((a, pk))
        })
        .collect::<Result<_, anyhow::Error>>()?;

    Ok(AccountsListResponse { accounts, total })
}

pub async fn handle_invoke(
    context: &HandlerContext,
    req: AccountsInvokeRequest,
) -> Result<AccountsInvokeResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let (_, signing_key) = sdk
        .key_manager_api()
        .get_key_or_active(key_manager::TRANSACTION_BRANCH, None)?;

    let account = sdk.accounts_api().get_account_by_name(&req.account_name)?;
    let inputs = sdk
        .substate_api()
        .load_dependent_substates(&[account.address.clone()])?;

    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let mut builder = Transaction::builder();
    builder
        .add_instruction(Instruction::CallMethod {
            component_address: account.address.as_component_address().unwrap(),
            method: req.method,
            args: req.args,
        })
        .with_fee(1)
        .with_inputs(inputs)
        .with_new_outputs(0)
        .sign(&signing_key.k);

    let transaction = builder.build();

    let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;
    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent { hash: tx_hash });

    let mut finalized = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = finalized.result.reject() {
        return Err(anyhow!("Transaction rejected: {}", reject));
    }

    Ok(AccountsInvokeResponse {
        result: finalized.execution_results.pop(),
    })
}

pub async fn handle_get_balances(
    context: &HandlerContext,
    req: AccountsGetBalancesRequest,
) -> Result<AccountsGetBalancesResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let account = sdk.accounts_api().get_account_by_name(&req.account_name)?;
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

pub async fn handle_get_by_name(
    context: &HandlerContext,
    req: AccountByNameRequest,
) -> Result<AccountByNameResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let account = sdk.accounts_api().get_account_by_name(&req.name)?;
    Ok(AccountByNameResponse { account })
}

#[allow(clippy::too_many_lines)]
pub async fn handle_claim_burn(
    context: &HandlerContext,
    req: ClaimBurnRequest,
) -> Result<ClaimBurnResponse, anyhow::Error> {
    let ClaimBurnRequest {
        account,
        claim_proof,
        fee,
    } = req;

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
    let (_, signing_key) = sdk.key_manager_api().get_active_key(key_manager::TRANSACTION_BRANCH)?;

    info!(
        target: LOG_TARGET,
        "Signing claim burn with key {}. This must be the same as the claiming key used in the burn transaction.",
        PublicKey::from_secret_key(&signing_key.k)
    );

    let mut inputs = vec![];

    // Add the account component
    let account_substate = sdk.substate_api().get_substate(&account.into())?;
    inputs.push(account_substate.address);

    // Add all versioned account child addresses as inputs
    let child_addresses = sdk.substate_api().load_dependent_substates(&[account.into()])?;
    inputs.extend(child_addresses);

    // TODO: we assume that all inputs will be consumed and produce a new output however this is only the case when the
    //       object is mutated
    let outputs = inputs
        .iter()
        .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version + 1))
        .collect::<Vec<_>>();

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
        account
    );

    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let instructions = vec![
        Instruction::ClaimBurn {
            claim: Box::new(ConfidentialClaim {
                public_key: reciprocal_claim_public_key,
                output_address: UnclaimedConfidentialOutputAddress::try_from(commitment.as_slice())?,
                range_proof,
                proof_of_knowledge: RistrettoComSig::new(Commitment::from_public_key(&public_nonce), u, v),
            }),
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: b"burn".to_vec() },
        Instruction::CallMethod {
            component_address: account,
            method: String::from("deposit"),
            args: args![Variable("burn")],
        },
    ];

    let mut transaction_builder = Transaction::builder();
    transaction_builder
        .with_instructions(instructions)
        .with_fee(fee)
        .with_inputs(inputs)
        .with_outputs(outputs)
        // transaction should have one output, corresponding to the same shard
        // as the account substate address
        // TODO: on a second claim burn, we shouldn't have any new outputs being created.
        .with_new_outputs(1)
        .sign(&signing_key.k);

    let transaction = transaction_builder.build();

    let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;

    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent { hash: tx_hash });

    let finalized = wait_for_result(&mut events, tx_hash).await?;

    Ok(ClaimBurnResponse {
        hash: tx_hash,
        result: finalized,
    })
}

async fn wait_for_result(
    events: &mut broadcast::Receiver<WalletEvent>,
    tx_hash: FixedHash,
) -> Result<FinalizeResult, anyhow::Error> {
    loop {
        let event = events.recv().await?;
        match event {
            WalletEvent::TransactionFinalized(finalized) if finalized.hash == tx_hash => {
                match finalized.result.result {
                    TransactionResult::Accept(_) => {
                        return Ok(finalized.result);
                    },
                    TransactionResult::Reject(reject) => {
                        return Err(anyhow!("Transaction rejected: {}", reject));
                    },
                }
            },
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
