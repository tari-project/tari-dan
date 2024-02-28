//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::{
    apis::accounts::{AccountsApi, AccountsApiError},
    models::{Account, VersionedSubstateId},
    DanWalletSdk,
};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;
use tari_engine_types::substate::SubstateId;
use tari_transaction::TransactionId;
use tari_wallet_daemon_client::ComponentAddressOrName;
use tokio::sync::broadcast;

use crate::{
    indexer_jrpc_impl::IndexerJsonRpcNetworkInterface,
    services::{TransactionFinalizedEvent, WalletEvent},
};

pub async fn wait_for_result(
    events: &mut broadcast::Receiver<WalletEvent>,
    transaction_id: TransactionId,
) -> Result<TransactionFinalizedEvent, anyhow::Error> {
    loop {
        let wallet_event = events.recv().await?;
        match wallet_event {
            WalletEvent::TransactionFinalized(event) if event.transaction_id == transaction_id => return Ok(event),
            WalletEvent::TransactionInvalid(event) if event.transaction_id == transaction_id => {
                return Err(anyhow::anyhow!(
                    "Transaction invalid: {} [status: {}]",
                    event
                        .finalize
                        .and_then(|finalize| finalize.reject().cloned())
                        .map(|f| f.to_string())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    event.status,
                ));
            },
            _ => {},
        }
    }
}

pub async fn wait_for_account_create_or_update(
    events: &mut broadcast::Receiver<WalletEvent>,
    account_address: &SubstateId,
) -> Result<SubstateId, anyhow::Error> {
    loop {
        let wallet_event = events.recv().await?;
        match wallet_event {
            WalletEvent::AccountCreated(event) if event.account.address == *account_address => {
                return Ok(event.account.address);
            },
            WalletEvent::AccountChanged(event) if event.account_address == *account_address => {
                return Ok(event.account_address);
            },
            _ => {},
        }
    }
}

pub fn get_account_with_inputs(
    account: Option<ComponentAddressOrName>,
    sdk: &DanWalletSdk<SqliteWalletStore, IndexerJsonRpcNetworkInterface>,
) -> Result<(Account, Vec<VersionedSubstateId>), anyhow::Error> {
    let account = get_account_or_default(account, &sdk.accounts_api())?;

    let mut inputs = vec![];

    // add the input for the source account component substate
    let account_substate = sdk.substate_api().get_substate(&account.address)?;
    inputs.push(account_substate.address);

    // Add all versioned account child addresses as inputs
    let child_addresses = sdk.substate_api().load_dependent_substates(&[&account.address])?;
    inputs.extend(child_addresses);

    Ok((account, inputs))
}

pub fn get_account<TStore>(
    account: &ComponentAddressOrName,
    accounts_api: &AccountsApi<'_, TStore>,
) -> Result<Account, AccountsApiError>
where
    TStore: tari_dan_wallet_sdk::storage::WalletStore,
{
    match account {
        ComponentAddressOrName::ComponentAddress(address) => {
            Ok(accounts_api.get_account_by_address(&(*address).into())?)
        },
        ComponentAddressOrName::Name(name) => Ok(accounts_api.get_account_by_name(name)?),
    }
}

pub fn get_account_or_default<T>(
    account: Option<ComponentAddressOrName>,
    accounts_api: &AccountsApi<'_, T>,
) -> Result<Account, anyhow::Error>
where
    T: tari_dan_wallet_sdk::storage::WalletStore,
{
    let result;
    if let Some(a) = account {
        result = get_account(&a, accounts_api)?;
    } else {
        result = accounts_api
            .get_default()
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("No default account found. Please set a default account."))?;
    }
    Ok(result)
}

pub(super) fn invalid_params<T: Display>(field: &str, details: Option<T>) -> anyhow::Error {
    axum_jrpc::error::JsonRpcError::new(
        axum_jrpc::error::JsonRpcErrorReason::InvalidParams,
        format!(
            "Invalid param '{}'{}",
            field,
            details.map(|d| format!(": {}", d)).unwrap_or_default()
        ),
        serde_json::Value::Null,
    )
    .into()
}
