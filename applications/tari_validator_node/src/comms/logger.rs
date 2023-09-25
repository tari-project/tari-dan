//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::Display,
    marker::PhantomData,
    task::{Context, Poll},
};

use tari_comms::types::CommsPublicKey;
use tari_comms_logging::SqliteMessageLog;
use tari_crypto::tari_utilities::ByteArray;
use tonic::codegen::futures_core::future::BoxFuture;
use tower::{Service, ServiceExt};

use crate::comms::destination::Destination;

#[derive(Debug, Clone)]
pub struct LoggerService<TMsg> {
    logger: SqliteMessageLog,
    _msg: PhantomData<TMsg>,
}

impl<TMsg> LoggerService<TMsg> {
    pub fn new(logger: SqliteMessageLog) -> Self {
        Self {
            logger,
            _msg: PhantomData,
        }
    }
}

impl<S, TMsg> tower_layer::Layer<S> for LoggerService<TMsg>
where
    S: Service<(Destination<CommsPublicKey>, TMsg), Response = ()> + Sync + Send + Clone + 'static,
    S::Future: Send + 'static,
    TMsg: serde::Serialize + Display + Send + Sync + 'static,
{
    type Service = LoggerServiceImpl<S>;

    fn layer(&self, next_service: S) -> Self::Service {
        LoggerServiceImpl {
            next_service,
            logger: self.logger.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoggerServiceImpl<S> {
    logger: SqliteMessageLog,
    next_service: S,
}

impl<S, TMsg> Service<(Destination<CommsPublicKey>, TMsg)> for LoggerServiceImpl<S>
where
    S: Service<(Destination<CommsPublicKey>, TMsg), Response = ()> + Sync + Send + Clone + 'static,
    S::Future: Send + 'static,
    TMsg: serde::Serialize + Display + Send + Sync + 'static,
{
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<S::Response, S::Error>>;
    type Response = ();

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, (dest, msg): (Destination<CommsPublicKey>, TMsg)) -> Self::Future {
        let next_service = self.next_service.clone();
        let logger = self.logger.clone();
        Box::pin(async move {
            if logger.is_enabled() {
                match &dest {
                    Destination::Peer(pk) => {
                        logger.log_outbound_message("Peer", pk.as_bytes(), "", "", &msg);
                    },
                    Destination::Selected(pks) => {
                        for pk in pks {
                            logger.log_outbound_message("Selected", pk.as_bytes(), "", "", &msg);
                        }
                    },
                    Destination::Flood => {
                        logger.log_outbound_message("Flood", b"", "", "", &msg);
                    },
                }
            }

            let mut svc = next_service.ready_oneshot().await?;
            svc.call((dest, msg)).await
        })
    }
}
