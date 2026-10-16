//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod accounts;
pub mod address_book;
pub mod auth;
pub mod burn_proofs;
pub mod confidential;
mod context;
pub mod error;
pub(crate) mod helpers;
pub mod keys;
pub mod nfts;
pub mod resources;
pub mod settings;
pub mod signing_requests;
pub mod stealth_utxos;
pub mod substates;
pub mod swap_pools;
pub mod templates;
#[cfg(test)]
pub(crate) mod test_support;
pub mod transaction;
pub mod transaction_requests;
pub mod validator;
pub mod validator_set;
pub(crate) mod value_lookup;
pub mod wallet;
pub mod web_ui;
pub mod webauthn;
pub mod webrtc;

use std::future::Future;

use async_trait::async_trait;
use axum_extra::{extract::CookieJar, headers::authorization::Bearer};
pub use context::HandlerContext;
use error::HandlerError;

// NOTE: async_trait is needed even with stabilization of async traits due to this Rust limitation: https://github.com/rust-lang/rust/issues/100013
#[async_trait]
pub trait Handler<'a, TReq> {
    type Response;

    async fn handle(
        &mut self,
        context: &'a HandlerContext,
        token: Option<&'a Bearer>,
        cookie: Option<CookieJar>,
        req: TReq,
    ) -> Result<Self::Response, HandlerError>;
}

#[async_trait]
impl<'a, F, TReq, TResp, TFut, TErr> Handler<'a, TReq> for F
where
    F: FnMut(&'a HandlerContext, Option<&'a Bearer>, TReq) -> TFut + Sync + Send,
    TFut: Future<Output = Result<TResp, TErr>> + Send,
    TReq: Send + 'static,
    TErr: Into<HandlerError>,
{
    type Response = TResp;

    async fn handle(
        &mut self,
        context: &'a HandlerContext,
        token: Option<&'a Bearer>,
        _cookie: Option<CookieJar>,
        req: TReq,
    ) -> Result<Self::Response, HandlerError> {
        let resp = self(context, token, req).await.map_err(Into::into)?;
        Ok(resp)
    }
}

// TODO: little hacky
pub struct HandlerWithCookie<F> {
    with_cookie: F,
}

impl<F> HandlerWithCookie<F> {
    pub fn new(with_cookie: F) -> Self {
        Self { with_cookie }
    }
}

#[async_trait]
impl<'a, F, TReq, TResp, TFut, TErr> Handler<'a, TReq> for HandlerWithCookie<F>
where
    F: FnMut(&'a HandlerContext, Option<&'a Bearer>, Option<CookieJar>, TReq) -> TFut + Sync + Send,
    TFut: Future<Output = Result<TResp, TErr>> + Send,
    TReq: Send + 'static,
    TErr: Into<HandlerError>,
{
    type Response = TResp;

    async fn handle(
        &mut self,
        context: &'a HandlerContext,
        token: Option<&'a Bearer>,
        cookie: Option<CookieJar>,
        req: TReq,
    ) -> Result<Self::Response, HandlerError> {
        let resp = (self.with_cookie)(context, token, cookie, req)
            .await
            .map_err(Into::into)?;
        Ok(resp)
    }
}
