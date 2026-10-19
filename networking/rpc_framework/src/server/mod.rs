//  Copyright 2021, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

mod error;
pub use error::RpcServerError;

mod handle;
pub use handle::RpcServerHandle;
use handle::RpcServerRequest;

#[cfg(feature = "metrics")]
mod metrics;

mod early_close;
mod router;
mod write_timeout;

use std::{
    cmp,
    collections::HashMap,
    convert::TryFrom,
    future::Future,
    io,
    io::ErrorKind,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::{Bytes, BytesMut};
use futures::{AsyncRead, AsyncWrite, SinkExt, StreamExt, stream::FuturesUnordered};
use libp2p::{PeerId, StreamProtocol};
use libp2p_substream::{ProtocolEvent, ProtocolNotification};
use log::*;
use prost::Message;
use router::Router;
use tokio::{sync::mpsc, task::JoinHandle, time};
use tower::{Service, make::MakeService};
use tracing::{Instrument, Level, debug, error, instrument, span, trace, warn};

use super::{
    Handshake,
    RPC_MAX_FRAME_SIZE,
    body::Body,
    error::HandshakeRejectReason,
    max_response_payload_size,
    message::{Request, Response, RpcMessageFlags},
    not_found::ProtocolServiceNotFound,
    status::RpcStatus,
};
use crate::{
    body::BodyBytes,
    bounded_executor::BoundedExecutor,
    framing,
    framing::CanonicalFraming,
    message::{RpcMethod, RpcResponse},
    notify::ProtocolNotificationRx,
    proto,
    server::{
        early_close::{EarlyClose, EarlyCloseError},
        write_timeout::WriteTimeout,
    },
};

const LOG_TARGET: &str = "libp2p::rpc::server";

/// How long a refusal may take to write. A refused peer holds no session slot, so this only bounds
/// how long the task delivering the refusal lives. Refusal tasks run outside the session executor,
/// so how many are in flight at once is bounded by this timeout and the rate at which the transport
/// accepts inbound substreams, which its connection and per-connection stream limits cap.
const REJECTION_WRITE_TIMEOUT: Duration = Duration::from_secs(1);

pub trait NamedProtocolService {
    const PROTOCOL_NAME: &'static str;

    /// Default implementation that returns a pointer to the static protocol name.
    fn as_protocol_name(&self) -> &'static str {
        Self::PROTOCOL_NAME
    }
}

pub struct RpcServer {
    builder: RpcServerBuilder,
    request_tx: mpsc::Sender<RpcServerRequest>,
    request_rx: mpsc::Receiver<RpcServerRequest>,
}

impl RpcServer {
    pub fn new() -> Self {
        Self::builder().finish()
    }

    pub fn builder() -> RpcServerBuilder {
        RpcServerBuilder::new()
    }

    pub fn add_service<S>(self, service: S) -> Router<S, ProtocolServiceNotFound>
    where
        S: MakeService<
                StreamProtocol,
                Request<Bytes>,
                MakeError = RpcServerError,
                Response = Response<Body>,
                Error = RpcStatus,
            > + NamedProtocolService
            + Send
            + 'static,
        S::Future: Send + 'static,
    {
        Router::new(self, service)
    }

    pub fn get_handle(&self) -> RpcServerHandle {
        RpcServerHandle::new(self.request_tx.clone())
    }

    pub(super) async fn serve<S, TSubstream>(
        self,
        service: S,
        notifications: ProtocolNotificationRx<TSubstream>,
    ) -> Result<(), RpcServerError>
    where
        TSubstream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        S: MakeService<
                StreamProtocol,
                Request<Bytes>,
                MakeError = RpcServerError,
                Response = Response<Body>,
                Error = RpcStatus,
            > + Send
            + 'static,
        S::Service: Send + 'static,
        S::Future: Send + 'static,
        S::Service: Send + 'static,
        <S::Service as Service<Request<Bytes>>>::Future: Send + 'static,
    {
        PeerRpcServer::new(self.builder, service, notifications, self.request_rx)
            .serve()
            .await
    }
}

impl Default for RpcServer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct RpcServerBuilder {
    maximum_simultaneous_sessions: Option<usize>,
    maximum_sessions_per_client: Option<usize>,
    minimum_client_deadline: Duration,
    maximum_client_deadline: Duration,
    minimum_keepalive_interval: Duration,
    handshake_timeout: Duration,
    idle_session_timeout: Duration,
    write_timeout: Duration,
}

impl RpcServerBuilder {
    fn new() -> Self {
        Default::default()
    }

    pub fn with_maximum_simultaneous_sessions(mut self, limit: usize) -> Self {
        self.maximum_simultaneous_sessions = Some(cmp::min(limit, BoundedExecutor::max_theoretical_tasks()));
        self
    }

    pub fn with_unlimited_simultaneous_sessions(mut self) -> Self {
        self.maximum_simultaneous_sessions = None;
        self
    }

    pub fn with_maximum_sessions_per_client(mut self, limit: usize) -> Self {
        self.maximum_sessions_per_client = Some(cmp::min(limit, BoundedExecutor::max_theoretical_tasks()));
        self
    }

    pub fn with_unlimited_sessions_per_client(mut self) -> Self {
        self.maximum_sessions_per_client = None;
        self
    }

    pub fn with_minimum_client_deadline(mut self, deadline: Duration) -> Self {
        self.minimum_client_deadline = deadline;
        self
    }

    /// Sets the longest deadline the server honours. A client asking for a longer one is served this
    /// one, so that a single request cannot make its session wait an unbounded span for the service
    /// to produce its response or the next item of it.
    pub fn with_maximum_client_deadline(mut self, deadline: Duration) -> Self {
        self.maximum_client_deadline = deadline;
        self
    }

    /// Sets how long a session may wait for its next request before the server closes it and
    /// releases its slot. Only the gap between requests counts. While a request is served, its
    /// deadline bounds each wait for the service and the write timeout each stalled write; neither
    /// caps how long the whole request takes.
    pub fn with_idle_session_timeout(mut self, timeout: Duration) -> Self {
        self.idle_session_timeout = timeout;
        self
    }

    /// Sets how long a write to a session may go without the client taking any of it before the server closes the
    /// session and releases its slot. A client that stops reading fills the transport's flow-control window, after
    /// which nothing more can be written to it, so this bounds how long such a client holds its session whatever
    /// the server was sending. Any write that goes ahead, however small, restarts the timeout: this bounds a stall,
    /// not the time taken to serve a request, so a client that keeps taking some of a response is served for as
    /// long as the response lasts.
    pub fn with_write_timeout(mut self, timeout: Duration) -> Self {
        self.write_timeout = timeout;
        self
    }

    /// Sets how long a new session may take to send its handshake. The session holds its slot while
    /// the server waits, and a client sends its handshake as soon as it opens the substream, so this
    /// only needs to cover network latency.
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
    }

    /// Sets the shortest keepalive interval the server is willing to emit. A client asking for a
    /// shorter interval is served this one, so that a client cannot make the server produce frames
    /// at an arbitrary rate.
    pub fn with_minimum_keepalive_interval(mut self, interval: Duration) -> Self {
        self.minimum_keepalive_interval = interval;
        self
    }

    /// The keepalive interval to serve a request asking for `requested`. Never shorter than the
    /// configured minimum, so that a client cannot dictate the rate at which the server produces
    /// frames.
    fn keepalive_interval_for(&self, requested: Option<Duration>) -> Option<Duration> {
        requested.map(|interval| cmp::max(interval, self.minimum_keepalive_interval))
    }

    pub fn finish(self) -> RpcServer {
        let (request_tx, request_rx) = mpsc::channel(10);
        RpcServer {
            builder: self,
            request_tx,
            request_rx,
        }
    }
}

impl Default for RpcServerBuilder {
    fn default() -> Self {
        Self {
            maximum_simultaneous_sessions: None,
            maximum_sessions_per_client: None,
            minimum_client_deadline: Duration::from_secs(1),
            maximum_client_deadline: Duration::from_secs(10 * 60),
            minimum_keepalive_interval: crate::DEFAULT_MINIMUM_KEEPALIVE_INTERVAL,
            handshake_timeout: Duration::from_secs(5),
            idle_session_timeout: Duration::from_secs(2 * 60),
            write_timeout: Duration::from_secs(60),
        }
    }
}

pub(super) struct PeerRpcServer<TSvc, TSubstream> {
    executor: BoundedExecutor,
    config: RpcServerBuilder,
    service: TSvc,
    protocol_notifications: Option<ProtocolNotificationRx<TSubstream>>,
    request_rx: mpsc::Receiver<RpcServerRequest>,
    sessions: HashMap<PeerId, usize>,
    tasks: FuturesUnordered<JoinHandle<PeerId>>,
}

impl<TSvc, TSubstream> PeerRpcServer<TSvc, TSubstream>
where
    TSubstream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    TSvc: MakeService<
            StreamProtocol,
            Request<Bytes>,
            MakeError = RpcServerError,
            Response = Response<Body>,
            Error = RpcStatus,
        > + Send
        + 'static,
    TSvc::Service: Send + 'static,
    <TSvc::Service as Service<Request<Bytes>>>::Future: Send + 'static,
    TSvc::Future: Send + 'static,
{
    fn new(
        config: RpcServerBuilder,
        service: TSvc,
        protocol_notifications: ProtocolNotificationRx<TSubstream>,
        request_rx: mpsc::Receiver<RpcServerRequest>,
    ) -> Self {
        Self {
            executor: match config.maximum_simultaneous_sessions {
                Some(usize::MAX) | None => BoundedExecutor::allow_maximum(),
                Some(num) => BoundedExecutor::new(num),
            },
            config,
            service,
            protocol_notifications: Some(protocol_notifications),
            request_rx,
            sessions: HashMap::new(),
            tasks: FuturesUnordered::new(),
        }
    }

    pub async fn serve(mut self) -> Result<(), RpcServerError> {
        let mut protocol_notifs = self
            .protocol_notifications
            .take()
            .expect("PeerRpcServer initialized without protocol_notifications");

        loop {
            tokio::select! {
                maybe_notif = protocol_notifs.recv() => {
                    match maybe_notif {
                        Some(notif) => self.handle_protocol_notification(notif).await,
                        // No more protocol notifications to come, so we're done
                        None => break,
                    }
                }

                Some(Ok(node_id)) = self.tasks.next() => {
                    self.on_session_complete(&node_id);
                },

                Some(req) = self.request_rx.recv() => {
                     self.handle_request(req).await;
                },
            }
        }

        debug!(
            target: LOG_TARGET,
            "Peer RPC server is shutting down because the protocol notification stream ended. Waiting for {} active \
             session(s) to complete",
            self.tasks.len()
        );

        // Drain in-flight sessions so that returning from serve means every per-session service instance
        // (and anything it owns, e.g. a state store handle) has been dropped. This is bounded: the
        // notification stream only ends once networking has shut down, so each session errors out at its
        // next substream read/write — which also closes any in-progress response stream — and handler
        // futures are bounded by the client deadline.
        while let Some(result) = self.tasks.next().await {
            match result {
                Ok(peer_id) => self.on_session_complete(&peer_id),
                Err(err) => {
                    error!(
                        target: LOG_TARGET,
                        "Session task panicked while shutting down: {}", err
                    );
                },
            }
        }

        debug!(target: LOG_TARGET, "Peer RPC server is shut down");

        Ok(())
    }

    async fn handle_request(&self, req: RpcServerRequest) {
        #[allow(clippy::enum_glob_use)]
        use RpcServerRequest::*;
        match req {
            GetNumActiveSessions(reply) => {
                let max_sessions = self
                    .config
                    .maximum_simultaneous_sessions
                    .unwrap_or_else(BoundedExecutor::max_theoretical_tasks);
                let num_active = max_sessions.saturating_sub(self.executor.num_available());
                let _ = reply.send(num_active);
            },
            GetNumActiveSessionsForPeer(node_id, reply) => {
                let num_active = self.sessions.get(&node_id).copied().unwrap_or(0);
                let _ = reply.send(num_active);
            },
        }
    }

    async fn handle_protocol_notification(&mut self, notification: ProtocolNotification<TSubstream>) {
        match notification.event {
            ProtocolEvent::NewInboundSubstream { peer_id, substream } => {
                debug!(
                    target: LOG_TARGET,
                    "New client connection for protocol `{}` from peer `{}`",
                    notification.protocol,
                    peer_id
                );

                let substream = WriteTimeout::new(substream, self.config.write_timeout);
                let framed = framing::canonical(substream, RPC_MAX_FRAME_SIZE);
                if let Err(err) = self.try_initiate_service(notification.protocol, peer_id, framed).await {
                    debug!(target: LOG_TARGET, "Unable to spawn RPC service: {}", err);
                }
            },
        }
    }

    fn new_session_for(&mut self, peer_id: PeerId) -> Result<usize, RpcServerError> {
        let count = self.sessions.entry(peer_id).or_insert(0);
        match self.config.maximum_sessions_per_client {
            Some(max) if max > 0 => {
                debug_assert!(*count <= max);
                if *count >= max {
                    return Err(RpcServerError::MaxSessionsPerClientReached { peer_id });
                }
            },
            Some(_) | None => {},
        }

        *count += 1;
        Ok(*count)
    }

    fn on_session_complete(&mut self, node_id: &PeerId) {
        debug!(target: LOG_TARGET, "Session complete for {}", node_id);
        if let Some(v) = self.sessions.get_mut(node_id) {
            *v -= 1;
            if *v == 0 {
                self.sessions.remove(node_id);
            }
        }
    }

    /// Admits or refuses a session without waiting on the peer: the handshake runs in the session's
    /// own task, so peers that are slow to send theirs cannot hold up admitting anyone else.
    async fn try_initiate_service(
        &mut self,
        protocol: StreamProtocol,
        peer_id: PeerId,
        framed: CanonicalFraming<WriteTimeout<TSubstream>>,
    ) -> Result<(), RpcServerError> {
        if !self.executor.can_spawn() {
            reject_session(peer_id, framed, HandshakeRejectReason::NoSessionsAvailable);
            return Err(RpcServerError::MaximumSessionsReached);
        }

        let service = match self.service.make_service(protocol.clone()).await {
            Ok(s) => s,
            Err(err) => {
                reject_session(peer_id, framed, HandshakeRejectReason::ProtocolNotSupported);
                return Err(err);
            },
        };

        match self.new_session_for(peer_id) {
            Ok(num_sessions) => {
                debug!(
                    target: LOG_TARGET,
                    "NEW SESSION for {} ({} active) ", peer_id, num_sessions
                );
            },

            Err(err) => {
                reject_session(peer_id, framed, HandshakeRejectReason::NoSessionsAvailable);
                return Err(err);
            },
        }

        let session = run_session(self.config.clone(), protocol, peer_id, service, framed);
        // The session slot taken above is released when the spawned task completes, so a session
        // that never starts must release it here.
        match self.executor.try_spawn(session) {
            Ok(handle) => {
                self.tasks.push(handle);
                Ok(())
            },
            Err(_) => {
                self.on_session_complete(&peer_id);
                Err(RpcServerError::MaximumSessionsReached)
            },
        }
    }
}

/// Performs the server handshake on `framed`, then serves the session until it ends. Returns
/// `peer_id` either way, which is what releases the session's slot.
async fn run_session<TSvc, TSubstream>(
    config: RpcServerBuilder,
    protocol: StreamProtocol,
    peer_id: PeerId,
    service: TSvc,
    mut framed: CanonicalFraming<TSubstream>,
) -> PeerId
where
    TSvc: Service<Request<Bytes>, Response = Response<Body>, Error = RpcStatus>,
    TSubstream: AsyncRead + AsyncWrite + Unpin,
{
    let handshake = Handshake::new(&mut framed)
        .with_timeout(config.handshake_timeout)
        .perform_server_handshake()
        .await;
    match handshake {
        Ok(version) => {
            debug!(
                target: LOG_TARGET,
                "Server negotiated RPC v{} with client node `{}`", version, peer_id
            );
        },
        Err(err) => {
            debug!(target: LOG_TARGET, "Handshake error with peer `{}`: {}", peer_id, err);
            #[cfg(feature = "metrics")]
            metrics::handshake_error_counter(&peer_id, &protocol).inc();
            return peer_id;
        },
    }

    let service = ActivePeerRpcService::new(config, protocol, peer_id, service, framed);
    #[cfg(feature = "metrics")]
    let num_sessions = metrics::num_sessions(&peer_id, &service.protocol);
    #[cfg(feature = "metrics")]
    num_sessions.inc();
    service.start().await;
    debug!(target: LOG_TARGET, "END OF SESSION for {} ", peer_id,);
    #[cfg(feature = "metrics")]
    num_sessions.dec();

    peer_id
}

/// Refuses a session from a task of its own, so that a peer slow to take the refusal cannot hold up
/// admitting anyone else.
fn reject_session<TSubstream>(
    peer_id: PeerId,
    mut framed: CanonicalFraming<TSubstream>,
    reason: HandshakeRejectReason,
) where
    TSubstream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    debug!(
        target: LOG_TARGET,
        "Rejecting RPC session request for peer `{}` because {}", peer_id, reason
    );
    tokio::spawn(async move {
        let mut handshake = Handshake::new(&mut framed);
        match time::timeout(REJECTION_WRITE_TIMEOUT, handshake.reject_with_reason(reason)).await {
            Ok(Ok(())) => {},
            Ok(Err(err)) => {
                debug!(
                    target: LOG_TARGET,
                    "Failed to refuse session for peer `{}`: {}", peer_id, err
                );
            },
            Err(_) => {
                debug!(
                    target: LOG_TARGET,
                    "Peer `{}` did not take the session refusal within {:.0?}", peer_id, REJECTION_WRITE_TIMEOUT
                );
            },
        }
    });
}

struct ActivePeerRpcService<TSvc, TSubstream> {
    config: RpcServerBuilder,
    protocol: StreamProtocol,
    peer_id: PeerId,
    service: TSvc,
    framed: EarlyClose<CanonicalFraming<TSubstream>>,
    logging_context_string: Arc<String>,
    /// A request the client sent while a response was still streaming to it, which ended that response. It is served
    /// before anything else is read.
    next_request: Option<BytesMut>,
}

impl<TSvc, TSubstream> ActivePeerRpcService<TSvc, TSubstream>
where
    TSvc: Service<Request<Bytes>, Response = Response<Body>, Error = RpcStatus>,
    TSubstream: AsyncRead + AsyncWrite + Unpin,
{
    pub(self) fn new(
        config: RpcServerBuilder,
        protocol: StreamProtocol,
        node_id: PeerId,
        service: TSvc,
        framed: CanonicalFraming<TSubstream>,
    ) -> Self {
        Self {
            logging_context_string: Arc::new(format!("peer: {}, protocol: {}", node_id, protocol)),

            config,
            protocol,
            peer_id: node_id,
            service,
            framed: EarlyClose::new(framed),
            next_request: None,
        }
    }

    async fn start(mut self) {
        debug!(
            target: LOG_TARGET,
            "({}) Rpc server started.", self.logging_context_string,
        );
        if let Err(err) = self.run().await {
            #[cfg(feature = "metrics")]
            metrics::error_counter(&self.peer_id, &self.protocol, &err).inc();
            let level = match &err {
                RpcServerError::Io(e) => err_to_log_level(e),
                RpcServerError::EarlyClose(e) => e.io().map(err_to_log_level).unwrap_or(log::Level::Error),
                _ => log::Level::Error,
            };
            log!(
                target: LOG_TARGET,
                level,
                "({}) Rpc server exited with an error: {}",
                self.logging_context_string,
                err
            );
        }
    }

    async fn run(&mut self) -> Result<(), RpcServerError> {
        loop {
            let next = match self.next_request.take() {
                Some(frame) => Some(Ok(frame)),
                None => {
                    let Ok(next) = time::timeout(self.config.idle_session_timeout, self.framed.next()).await else {
                        debug!(
                            target: LOG_TARGET,
                            "({}) Closing session after {:.0?} without a request",
                            self.logging_context_string,
                            self.config.idle_session_timeout
                        );
                        break;
                    };
                    next
                },
            };
            let Some(result) = next else {
                break;
            };
            match result {
                Ok(frame) => {
                    #[cfg(feature = "metrics")]
                    metrics::inbound_requests_bytes(&self.peer_id, &self.protocol).observe(frame.len() as f64);

                    let start = Instant::now();

                    if let Err(err) = self.handle_request(frame.freeze()).await {
                        if let Err(err) = self.framed.close().await {
                            let level = err.io().map(err_to_log_level).unwrap_or(log::Level::Error);

                            log!(
                                target: LOG_TARGET,
                                level,
                                "({}) Failed to close substream after socket error: {}",
                                self.logging_context_string,
                                err,
                            );
                        }
                        let level = err.early_close_io().map(err_to_log_level).unwrap_or(log::Level::Error);
                        log!(
                            target: LOG_TARGET,
                            level,
                            "(peer: {}, protocol: {}) Failed to handle request: {}",
                            self.peer_id,
                            self.protocol_name(),
                            err
                        );
                        return Err(err);
                    }
                    let elapsed = start.elapsed();
                    debug!(
                        target: LOG_TARGET,
                        "({}) RPC request completed in {:.0?}{}",
                        self.logging_context_string,
                        elapsed,
                        if elapsed.as_secs() > 5 { " (LONG REQUEST)" } else { "" }
                    );
                },
                Err(err) => {
                    if let Err(err) = self.framed.close().await {
                        error!(
                            target: LOG_TARGET,
                            "({}) Failed to close substream after socket error: {}", self.logging_context_string, err
                        );
                    }
                    return Err(err.into());
                },
            }
        }

        self.framed.close().await?;
        Ok(())
    }

    #[instrument(name = "rpc::server::handle_req", skip(self, request), err, fields(request_size = request.len()))]
    #[allow(clippy::too_many_lines)]
    async fn handle_request(&mut self, mut request: Bytes) -> Result<(), RpcServerError> {
        let decoded_msg = proto::RpcRequest::decode(&mut request)?;

        let request_id = decoded_msg.request_id;
        let method = RpcMethod::from(decoded_msg.method);
        let deadline = cmp::min(
            Duration::from_secs(decoded_msg.deadline),
            self.config.maximum_client_deadline,
        );

        // The client side deadline MUST be greater or equal to the minimum_client_deadline
        if deadline < self.config.minimum_client_deadline {
            debug!(
                target: LOG_TARGET,
                "({}) Client has an invalid deadline. {}", self.logging_context_string, decoded_msg
            );
            // Let the client know that they have disobeyed the spec
            let status = RpcStatus::bad_request(format!(
                "Invalid deadline ({:.0?}). The deadline MUST be greater than {:.0?}.",
                self.peer_id, deadline,
            ));
            let bad_request = proto::RpcResponse {
                request_id,
                status: status.as_code(),
                flags: RpcMessageFlags::FIN.bits().into(),
                payload: status.to_details_bytes(),
            };
            #[cfg(feature = "metrics")]
            metrics::status_error_counter(&self.peer_id, &self.protocol, status.as_status_code()).inc();
            self.framed.send(bad_request.encode_to_vec().into()).await?;
            return Ok(());
        }

        let msg_flags = RpcMessageFlags::from_bits(u8::try_from(decoded_msg.flags).map_err(|_| {
            RpcServerError::ProtocolError(format!("invalid message flag: must be less than {}", u8::MAX))
        })?)
        .ok_or(RpcServerError::ProtocolError(format!(
            "invalid message flag, does not match any flags ({})",
            decoded_msg.flags
        )))?;

        if msg_flags.contains(RpcMessageFlags::FIN) {
            debug!(target: LOG_TARGET, "({}) Client sent FIN.", self.logging_context_string);
            return Ok(());
        }
        if msg_flags.contains(RpcMessageFlags::ACK) {
            debug!(
                target: LOG_TARGET,
                "({}) sending ACK response.", self.logging_context_string
            );
            let ack = proto::RpcResponse {
                request_id,
                status: RpcStatus::ok().as_code(),
                flags: RpcMessageFlags::ACK.bits().into(),
                ..Default::default()
            };
            self.framed.send(ack.encode_to_vec().into()).await?;
            return Ok(());
        }

        debug!(
            target: LOG_TARGET,
            "({}) Request: {}, Method: {}",
            self.logging_context_string,
            decoded_msg,
            method.id()
        );

        let keepalive_interval = self.config.keepalive_interval_for(decoded_msg.keepalive_interval());

        let req = Request::new(method, decoded_msg.payload.into());

        let service_call = log_timing(
            self.logging_context_string.clone(),
            request_id,
            "service call",
            self.service.call(req),
        );
        let service_result = time::timeout(deadline, service_call).await;
        let service_result = match service_result {
            Ok(v) => v,
            Err(_) => {
                warn!(
                    target: LOG_TARGET,
                    "{} RPC service was not able to complete within the deadline ({:.0?}). Request aborted",
                    self.logging_context_string,
                    deadline,
                );

                #[cfg(feature = "metrics")]
                metrics::error_counter(
                    &self.peer_id,
                    &self.protocol,
                    &RpcServerError::ServiceCallExceededDeadline,
                )
                .inc();
                return Ok(());
            },
        };

        match service_result {
            Ok(body) => {
                self.process_body(request_id, deadline, keepalive_interval, body)
                    .await?;
            },
            Err(err) => {
                debug!(
                    target: LOG_TARGET,
                    "{} Service returned an error: {}", self.logging_context_string, err
                );
                let resp = proto::RpcResponse {
                    request_id,
                    status: err.as_code(),
                    flags: RpcMessageFlags::FIN.bits().into(),
                    payload: err.to_details_bytes(),
                };

                #[cfg(feature = "metrics")]
                metrics::status_error_counter(&self.peer_id, &self.protocol, err.as_status_code()).inc();
                self.framed.send(resp.encode_to_vec().into()).await?;
            },
        }

        Ok(())
    }

    fn protocol_name(&self) -> &str {
        self.protocol.as_ref()
    }

    /// Streams `body` back to the client.
    ///
    /// `deadline` bounds the gap between messages that actually carry the response: a stream that
    /// produces nothing for that long is abandoned. When the client asked for keepalives, an empty
    /// ACK frame goes out every `keepalive_interval` while that budget is running, so an idle stream
    /// remains distinguishable from a dead peer without extending how long the server holds it.
    async fn process_body(
        &mut self,
        request_id: u32,
        deadline: Duration,
        keepalive_interval: Option<Duration>,
        body: Response<Body>,
    ) -> Result<(), RpcServerError> {
        trace!(target: LOG_TARGET, "Service call succeeded");

        #[cfg(feature = "metrics")]
        let peer_id = self.peer_id;
        #[cfg(feature = "metrics")]
        let protocol = self.protocol.clone();
        let mut stream = body
            .into_message()
            .map(|result| into_response(request_id, result))
            .map(move |mut message| {
                if message.payload.len() > max_response_payload_size() {
                    message = message.exceeded_message_size();
                }
                #[cfg(feature = "metrics")]
                if !message.status.is_ok() {
                    metrics::status_error_counter(&peer_id, &protocol, message.status).inc();
                }
                message.to_proto()
            })
            .map(|resp| Bytes::from(resp.encode_to_vec()));

        // The idle budget is measured from the last message because `now + deadline` may lie beyond what an `Instant`
        // can represent.
        let mut last_message_at = Instant::now();

        loop {
            let idle_budget = deadline.saturating_sub(last_message_at.elapsed());
            let wait = keepalive_interval.map_or(idle_budget, |interval| cmp::min(interval, idle_budget));

            let next_item = log_timing(
                self.logging_context_string.clone(),
                request_id,
                "message read",
                time::timeout(wait, stream.next()),
            );

            // The client sends nothing of its own while a response is outstanding, so whatever it does send means it is
            // done with this one. Watching for it is what ends a stream that would otherwise run for as long as the
            // session does.
            let result = tokio::select! {
                biased;
                inbound = self.framed.next() => {
                    if self.stream_continues_after_inbound(request_id, inbound)? {
                        continue;
                    }
                    break;
                },
                result = next_item => result,
            };
            match result {
                Ok(Some(msg)) => {
                    #[cfg(feature = "metrics")]
                    metrics::outbound_response_bytes(&self.peer_id, &self.protocol).observe(msg.len() as f64);
                    debug!(
                        target: LOG_TARGET,
                        "({}) Sending body len = {}",
                        self.logging_context_string,
                        msg.len()
                    );

                    if !self.send_stream_frame(request_id, msg).await? {
                        break;
                    }
                    last_message_at = Instant::now();
                },
                Ok(None) => {
                    debug!(target: LOG_TARGET, "{} Request complete", self.logging_context_string,);
                    break;
                },
                Err(_) if keepalive_interval.is_some() && last_message_at.elapsed() < deadline => {
                    trace!(
                        target: LOG_TARGET,
                        "({}) Sending keepalive for request #{}", self.logging_context_string, request_id
                    );
                    let keepalive = Bytes::from(RpcResponse::keepalive(request_id).to_proto().encode_to_vec());
                    #[cfg(feature = "metrics")]
                    metrics::outbound_response_bytes(&self.peer_id, &self.protocol).observe(keepalive.len() as f64);
                    if !self.send_stream_frame(request_id, keepalive).await? {
                        break;
                    }
                },
                Err(_) => {
                    debug!(
                        target: LOG_TARGET,
                        "({}) Failed to return result within client deadline ({:.0?})",
                        self.logging_context_string,
                        deadline
                    );

                    #[cfg(feature = "metrics")]
                    metrics::error_counter(
                        &self.peer_id,
                        &self.protocol,
                        &RpcServerError::ReadStreamExceededDeadline,
                    )
                    .inc();
                    break;
                },
            }
        } // end loop
        Ok(())
    }

    /// Sends one frame of the response to `request_id`, returning whether the response goes on. A send held up by a
    /// client that has stopped reading is cut short by the first frame the client sends, which is taken as it would
    /// be between frames. The response ends there whatever that frame is: the one being sent may never have been
    /// queued, and the response cannot go on with a gap in it.
    async fn send_stream_frame(&mut self, request_id: u32, frame: Bytes) -> Result<bool, RpcServerError> {
        match self.framed.send(frame).await {
            Ok(()) => Ok(true),
            Err(EarlyCloseError::UnexpectedMessage(inbound)) => {
                self.stream_continues_after(request_id, inbound);
                Ok(false)
            },
            Err(err) => Err(err.into()),
        }
    }

    /// Takes what the client's side of the session yielded while the response to `request_id` was streaming,
    /// returning whether the response goes on.
    fn stream_continues_after_inbound(
        &mut self,
        request_id: u32,
        inbound: Option<io::Result<BytesMut>>,
    ) -> Result<bool, RpcServerError> {
        match inbound {
            Some(Ok(frame)) => Ok(self.stream_continues_after(request_id, frame)),
            Some(Err(err)) => Err(err.into()),
            None => {
                debug!(
                    target: LOG_TARGET,
                    "({}) Client closed the session while request #{} was streaming",
                    self.logging_context_string,
                    request_id
                );
                Ok(false)
            },
        }
    }

    /// Takes a frame the client sent while the response to `request_id` was streaming, returning whether the
    /// response goes on. A FIN for `request_id` abandons it. A FIN for any other request concerns one already
    /// answered. Anything else is the client's next request, served once this response has ended.
    fn stream_continues_after(&mut self, request_id: u32, frame: BytesMut) -> bool {
        let fin = u32::from(RpcMessageFlags::FIN.bits());
        match proto::RpcRequest::decode(frame.as_ref()) {
            Ok(req) if req.flags & fin != 0 => {
                if req.request_id == request_id {
                    debug!(
                        target: LOG_TARGET,
                        "({}) Client abandoned request #{}", self.logging_context_string, request_id
                    );
                    false
                } else {
                    true
                }
            },
            _ => {
                debug!(
                    target: LOG_TARGET,
                    "({}) Client sent its next request while request #{} was streaming. Ending the stream",
                    self.logging_context_string,
                    request_id
                );
                self.next_request = Some(frame);
                false
            },
        }
    }
}

async fn log_timing<R, F: Future<Output = R>>(context_str: Arc<String>, request_id: u32, tag: &str, fut: F) -> R {
    let t = Instant::now();
    let span = span!(Level::TRACE, "rpc::internal::timing", request_id, tag);
    let ret = fut.instrument(span).await;
    let elapsed = t.elapsed();
    trace!(
        target: LOG_TARGET,
        "({}) RPC TIMING(REQ_ID={}): '{}' took {:.2}s{}",
        context_str,
        request_id,
        tag,
        elapsed.as_secs_f32(),
        if elapsed.as_secs() >= 5 { " (SLOW)" } else { "" }
    );
    ret
}

fn into_response(request_id: u32, result: Result<BodyBytes, RpcStatus>) -> RpcResponse {
    match result {
        Ok(msg) => {
            let mut flags = RpcMessageFlags::empty();
            if msg.is_finished() {
                flags |= RpcMessageFlags::FIN;
            }
            RpcResponse {
                request_id,
                status: RpcStatus::ok().as_status_code(),
                flags,
                payload: msg.into_bytes().unwrap_or_else(Bytes::new),
            }
        },
        Err(err) => {
            debug!(target: LOG_TARGET, "Body contained an error: {}", err);
            RpcResponse {
                request_id,
                status: err.as_status_code(),
                flags: RpcMessageFlags::FIN,
                payload: Bytes::from(err.to_details_bytes()),
            }
        },
    }
}

fn err_to_log_level(err: &io::Error) -> log::Level {
    match err.kind() {
        ErrorKind::ConnectionReset |
        ErrorKind::ConnectionAborted |
        ErrorKind::BrokenPipe |
        ErrorKind::WriteZero |
        ErrorKind::TimedOut |
        ErrorKind::UnexpectedEof => log::Level::Debug,
        _ => log::Level::Error,
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::DuplexStream;
    use tokio_util::compat::Compat;

    use super::*;

    #[tokio::test]
    async fn serve_drains_in_flight_sessions_before_returning() {
        let (notify_tx, notify_rx) = mpsc::unbounded_channel::<ProtocolNotification<Compat<DuplexStream>>>();
        let (_request_tx, request_rx) = mpsc::channel(1);
        let server = PeerRpcServer::new(RpcServerBuilder::new(), ProtocolServiceNotFound, notify_rx, request_rx);

        // Each real session task owns a per-session service instance (and anything it captured, e.g. a state
        // store handle). Returning from serve must guarantee those are dropped.
        let session_resource = Arc::new(());
        for _ in 0..3 {
            let resource = session_resource.clone();
            server.tasks.push(tokio::spawn(async move {
                time::sleep(Duration::from_millis(50)).await;
                drop(resource);
                PeerId::random()
            }));
        }

        // Closing the notification channel signals serve to shut down.
        drop(notify_tx);
        server.serve().await.unwrap();

        assert_eq!(Arc::strong_count(&session_resource), 1);
    }
}

#[cfg(test)]
mod keepalive_tests {
    use tokio::io::DuplexStream;
    use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

    use super::*;
    use crate::not_found::NeverService;

    type TestSubstream = Compat<DuplexStream>;

    fn framed_pair() -> (CanonicalFraming<TestSubstream>, CanonicalFraming<TestSubstream>) {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        (
            framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
        )
    }

    fn session(framed: CanonicalFraming<TestSubstream>) -> ActivePeerRpcService<NeverService, TestSubstream> {
        ActivePeerRpcService::new(
            RpcServerBuilder::default(),
            StreamProtocol::new("/test/keepalive/1.0"),
            PeerId::random(),
            NeverService,
            framed,
        )
    }

    /// A response body fed by the test, which stalls for as long as the test holds the sender
    /// without sending.
    fn body_fed_by(rx: mpsc::Receiver<Result<Bytes, RpcStatus>>) -> Response<Body> {
        Response::new(Body::streaming(futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        })))
    }

    async fn collect_responses(framed: &mut CanonicalFraming<TestSubstream>) -> Vec<proto::RpcResponse> {
        let mut responses = Vec::new();
        while let Some(frame) = framed.next().await {
            responses.push(proto::RpcResponse::decode(frame.unwrap().freeze()).unwrap());
        }
        responses
    }

    #[tokio::test]
    async fn keepalives_hold_an_idle_stream_open_until_its_deadline() {
        let (server_framed, mut client_framed) = framed_pair();
        let (tx, rx) = mpsc::channel(1);
        tx.send(Ok(Bytes::from_static(b"first"))).await.unwrap();

        let mut service = session(server_framed);
        let started = Instant::now();
        let server = tokio::spawn(async move {
            service
                .process_body(
                    1,
                    Duration::from_millis(400),
                    Some(Duration::from_millis(50)),
                    body_fed_by(rx),
                )
                .await
                .unwrap();
        });

        let responses = collect_responses(&mut client_framed).await;
        server.await.unwrap();
        // The stream is only abandoned once the deadline is up, not at the first idle interval.
        assert!(started.elapsed() >= Duration::from_millis(400));

        let (keepalives, messages): (Vec<_>, Vec<_>) = responses.iter().partition(|r| r.is_keepalive());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload, b"first");
        assert!(
            keepalives.len() >= 3,
            "expected several keepalives, got {}",
            keepalives.len()
        );
        assert!(keepalives.iter().all(|r| r.payload.is_empty() && r.request_id == 1));

        drop(tx);
    }

    #[tokio::test]
    async fn an_idle_stream_ends_at_its_deadline_when_no_keepalives_are_asked_for() {
        let (server_framed, mut client_framed) = framed_pair();
        let (tx, rx) = mpsc::channel(1);
        tx.send(Ok(Bytes::from_static(b"first"))).await.unwrap();

        let mut service = session(server_framed);
        let server = tokio::spawn(async move {
            service
                .process_body(1, Duration::from_millis(150), None, body_fed_by(rx))
                .await
                .unwrap();
        });

        let responses = collect_responses(&mut client_framed).await;
        server.await.unwrap();

        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].payload, b"first");

        drop(tx);
    }

    #[tokio::test]
    async fn a_message_renews_the_deadline() {
        let (server_framed, mut client_framed) = framed_pair();
        let (tx, rx) = mpsc::channel(1);

        let mut service = session(server_framed);
        let server = tokio::spawn(async move {
            service
                .process_body(
                    1,
                    Duration::from_millis(600),
                    Some(Duration::from_millis(50)),
                    body_fed_by(rx),
                )
                .await
                .unwrap();
        });

        let feeder = tokio::spawn(async move {
            for _ in 0..4 {
                time::sleep(Duration::from_millis(100)).await;
                tx.send(Ok(Bytes::from_static(b"tick"))).await.unwrap();
            }
            // Holding the sender past the last message lets the deadline expire and end the stream.
            time::sleep(Duration::from_millis(900)).await;
        });

        let responses = collect_responses(&mut client_framed).await;
        server.await.unwrap();
        feeder.await.unwrap();

        assert_eq!(responses.iter().filter(|r| !r.is_keepalive()).count(), 4);
    }

    fn answering_session(
        config: RpcServerBuilder,
        framed: CanonicalFraming<TestSubstream>,
    ) -> ActivePeerRpcService<impl Service<Request<Bytes>, Response = Response<Body>, Error = RpcStatus>, TestSubstream>
    {
        let service = tower::service_fn(|_: Request<Bytes>| {
            futures::future::ready(Ok::<_, RpcStatus>(Response::new(Body::single(Bytes::from_static(
                b"pong",
            )))))
        });
        ActivePeerRpcService::new(
            config,
            StreamProtocol::new("/test/deadline/1.0"),
            PeerId::random(),
            service,
            framed,
        )
    }

    async fn serve_one_request_with_deadline(config: RpcServerBuilder, deadline_secs: u64) -> proto::RpcResponse {
        let (server_framed, mut client_framed) = framed_pair();
        let mut service = answering_session(config, server_framed);
        let request = proto::RpcRequest {
            request_id: 1,
            method: 1,
            deadline: deadline_secs,
            ..Default::default()
        };
        service.handle_request(request.encode_to_vec().into()).await.unwrap();
        drop(service);

        let responses = collect_responses(&mut client_framed).await;
        assert_eq!(responses.len(), 1, "expected a single response, got {responses:?}");
        responses.into_iter().next().unwrap()
    }

    #[tokio::test]
    async fn the_longest_deadline_a_client_can_ask_for_is_served() {
        let response = serve_one_request_with_deadline(RpcServerBuilder::default(), u64::MAX).await;
        assert_eq!(response.status, RpcStatus::ok().as_code());
        assert_eq!(response.payload, b"pong");
    }

    #[tokio::test]
    async fn the_longest_deadline_a_client_can_ask_for_is_served_without_a_ceiling() {
        let config = RpcServerBuilder::default().with_maximum_client_deadline(Duration::MAX);
        for deadline_secs in [1u64 << 63, u64::MAX] {
            let response = serve_one_request_with_deadline(config.clone(), deadline_secs).await;
            assert_eq!(response.status, RpcStatus::ok().as_code());
            assert_eq!(response.payload, b"pong");
        }
    }

    #[test]
    fn a_keepalive_interval_below_the_minimum_is_raised_to_it() {
        let config = RpcServerBuilder::new().with_minimum_keepalive_interval(Duration::from_secs(10));
        assert_eq!(config.keepalive_interval_for(None), None);
        assert_eq!(
            config.keepalive_interval_for(Some(Duration::from_secs(1))),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            config.keepalive_interval_for(Some(Duration::from_secs(30))),
            Some(Duration::from_secs(30))
        );
    }
}
