//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod accounts;
pub mod confidential;
mod context;
pub mod error;
pub mod keys;
pub mod rpc;
pub mod transaction;
pub mod webrtc;

use std::future::Future;

use axum::async_trait;
pub use context::HandlerContext;
use error::HandlerError;
use tari_dan_wallet_sdk::{models::Account, DanWalletSdk};
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
    TFut: Future<Output = Result<TResp, TErr>> + Sync + Send,
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

pub fn get_account<T>(account: ComponentAddressOrName, sdk: &DanWalletSdk<T>) -> Result<Account, anyhow::Error>
where T: tari_dan_wallet_sdk::storage::WalletStore {
    match account {
        ComponentAddressOrName::ComponentAddress(address) => Ok(sdk.accounts_api().get_account(&address.into())?),
        ComponentAddressOrName::Name(name) => Ok(sdk.accounts_api().get_account_by_name(&name)?),
    }
}

pub fn get_account_or_default<T>(
    account: Option<ComponentAddressOrName>,
    sdk: &DanWalletSdk<T>,
) -> Result<Account, anyhow::Error>
where
    T: tari_dan_wallet_sdk::storage::WalletStore,
{
    let result;
    if let Some(a) = account {
        result = get_account(a, sdk)?;
    } else {
        result = sdk.accounts_api().get_default()?;
    }
    Ok(result)
}
