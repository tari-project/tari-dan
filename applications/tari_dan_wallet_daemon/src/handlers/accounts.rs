//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use anyhow::anyhow;
use log::*;
use tari_common_types::types::FixedHash;
use tari_dan_common_types::{optional::Optional, NodeAddressable, ShardId};
use tari_engine_types::{
    commit_result::{FinalizeResult, TransactionResult},
    instruction::Instruction,
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{args, crypto::RistrettoPublicKeyBytes, models::NonFungibleAddress};
use tari_transaction::Transaction;
use tari_wallet_daemon_client::types::{
    AccountsCreateRequest,
    AccountsCreateResponse,
    AccountsGetBalancesRequest,
    AccountsGetBalancesResponse,
    AccountsInvokeRequest,
    AccountsInvokeResponse,
    AccountsListRequest,
    AccountsListResponse,
};
use tokio::sync::broadcast;

use super::context::HandlerContext;
use crate::{
    handlers::TRANSACTION_KEYMANAGER_BRANCH,
    services::{TransactionSubmittedEvent, WalletEvent},
};

const LOG_TARGET: &str = "tari::dan_wallet_daemon::handlers::transaction";

pub async fn handle_create(
    context: &HandlerContext,
    req: AccountsCreateRequest,
) -> Result<AccountsCreateResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();

    if let Some(name) = req.account_name.as_ref() {
        if sdk
            .accounts_api()
            .get_account_address_by_name(name)
            .optional()?
            .is_some()
        {
            return Err(anyhow!("Account name '{}' already exists", name));
        }
    }

    let (key_index, signing_key) = sdk
        .key_manager_api()
        .get_key_or_active(TRANSACTION_KEYMANAGER_BRANCH, req.signing_key_index)?;
    let owner_pk = sdk
        .key_manager_api()
        .get_public_key(TRANSACTION_KEYMANAGER_BRANCH, req.signing_key_index)?;
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

    Ok(AccountsCreateResponse { address: addr.clone() })
}

pub async fn handle_list(
    context: &HandlerContext,
    req: AccountsListRequest,
) -> Result<AccountsListResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let accounts = sdk.accounts_api().get_many(req.limit)?;
    let total = sdk.accounts_api().count()?;

    Ok(AccountsListResponse { accounts, total })
}

pub async fn handle_invoke(
    context: &HandlerContext,
    req: AccountsInvokeRequest,
) -> Result<AccountsInvokeResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let (_, signing_key) = sdk
        .key_manager_api()
        .get_key_or_active(TRANSACTION_KEYMANAGER_BRANCH, None)?;

    let account_address = sdk.accounts_api().get_account_address_by_name(&req.account_name)?;
    let inputs = sdk
        .substate_api()
        .load_dependent_substates(&[account_address.clone()])?;

    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let mut builder = Transaction::builder();
    builder
        .add_instruction(Instruction::CallMethod {
            component_address: account_address.as_component_address().unwrap(),
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
    let (_, signing_key) = sdk
        .key_manager_api()
        .get_key_or_active(TRANSACTION_KEYMANAGER_BRANCH, None)?;

    let account_address = sdk.accounts_api().get_account_address_by_name(&req.account_name)?;
    let inputs = sdk
        .substate_api()
        .load_dependent_substates(&[account_address.clone()])?;

    info!(
        target: LOG_TARGET,
        "Loaded {} inputs for account: {}",
        inputs.len(),
        account_address
    );
    for input in &inputs {
        info!(target: LOG_TARGET, "input: {}", input);
    }

    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let mut builder = Transaction::builder();
    builder
        .add_instruction(Instruction::CallMethod {
            component_address: account_address.as_component_address().unwrap(),
            method: "get_balances".to_string(),
            args: args![],
        })
        .with_fee(1)
        .with_inputs(inputs)
        .with_new_outputs(0)
        .sign(&signing_key.k);

    let transaction = builder.build();

    let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;

    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent { hash: tx_hash });

    let finalized = wait_for_result(&mut events, tx_hash).await?;
    Ok(AccountsGetBalancesResponse {
        address: account_address,
        balances: finalized.execution_results[0].decode()?,
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
