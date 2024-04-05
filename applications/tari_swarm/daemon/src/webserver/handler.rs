//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::future::Future;

use axum::async_trait;

use super::{context::HandlerContext, error::HandlerError};

#[async_trait]
pub trait JrpcHandler<'a, TReq> {
    type Response;

    async fn handle(&mut self, context: &'a HandlerContext, req: TReq) -> Result<Self::Response, HandlerError>;
}

#[async_trait]
impl<'a, F, TReq, TResp, TFut, TErr> JrpcHandler<'a, TReq> for F
where
    F: FnMut(&'a HandlerContext, TReq) -> TFut + Sync + Send,
    TFut: Future<Output = Result<TResp, TErr>> + Send,
    TReq: Send + 'static,
    TErr: Into<HandlerError>,
{
    type Response = TResp;

    async fn handle(&mut self, context: &'a HandlerContext, req: TReq) -> Result<Self::Response, HandlerError> {
        let resp = self(context, req).await.map_err(Into::into)?;
        Ok(resp)
    }
}
