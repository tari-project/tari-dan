//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod accounts;
pub mod confidential;
mod context;
pub mod error;
pub mod keys;
pub mod nfts;
pub mod rpc;
pub mod transaction;
pub mod webrtc;

use std::{fmt::Display, future::Future};

use axum::async_trait;
pub use context::HandlerContext;
use error::HandlerError;
use tari_dan_common_types::optional::Optional;
use tari_dan_wallet_sdk::{
    apis::accounts::{AccountsApi, AccountsApiError},
    models::Account,
};
use tari_wallet_daemon_client::ComponentAddressOrName;

#[async_trait]
pub trait Handler<'a, TReq> {
    type Response;

    async fn handle(
        &mut self,
        context: &'a HandlerContext,
        token: Option<String>,
        req: TReq,
    ) -> Result<Self::Response, HandlerError>;
}

#[async_trait]
impl<'a, F, TReq, TResp, TFut, TErr> Handler<'a, TReq> for F
where
    F: FnMut(&'a HandlerContext, Option<String>, TReq) -> TFut + Sync + Send,
    TFut: Future<Output = Result<TResp, TErr>> + Send,
    TReq: Send + 'static,
    TErr: Into<HandlerError>,
{
    type Response = TResp;

    async fn handle(
        &mut self,
        context: &'a HandlerContext,
        token: Option<String>,
        req: TReq,
    ) -> Result<Self::Response, HandlerError> {
        let resp = self(context, token, req).await.map_err(Into::into)?;
        Ok(resp)
    }
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

pub(self) fn invalid_params<T: Display>(field: &str, details: Option<T>) -> anyhow::Error {
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
