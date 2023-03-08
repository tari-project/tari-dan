//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod accounts;
pub mod confidential;
mod context;
pub mod error;
pub mod keys;
pub mod rpc;
pub mod transaction;

pub(crate) const TRANSACTION_KEYMANAGER_BRANCH: &str = "transactions";
pub(crate) const OUTPUT_KEYMANAGER_BRANCH: &str = "outputs";

use std::future::Future;

use axum::async_trait;
pub use context::HandlerContext;
use error::HandlerError;

#[async_trait]
pub trait Handler<'a, TReq> {
    type Response;

    async fn handle(&mut self, context: &'a HandlerContext, req: TReq) -> Result<Self::Response, HandlerError>;
}

#[async_trait]
impl<'a, F, TReq, TResp, TFut, TErr> Handler<'a, TReq> for F
where
    F: FnMut(&'a HandlerContext, TReq) -> TFut + Sync + Send,
    TFut: Future<Output = Result<TResp, TErr>> + Sync + Send,
    TReq: Send + 'static,
    TErr: Into<HandlerError>,
{
    type Response = TResp;

    async fn handle(&mut self, context: &'a HandlerContext, req: TReq) -> Result<Self::Response, HandlerError> {
        let resp = self(context, req).await.map_err(Into::into)?;
        Ok(resp)
    }
}
