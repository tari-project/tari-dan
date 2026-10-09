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

#[cfg(feature = "metrics")]
mod metrics;

use std::{
    cmp,
    convert::TryFrom,
    fmt,
    future::Future,
    marker::PhantomData,
    sync::Arc,
    time::{Duration, Instant},
};

use bytes::{Bytes, BytesMut};
use futures::{
    AsyncRead,
    AsyncWrite,
    FutureExt,
    SinkExt,
    StreamExt,
    future,
    future::{BoxFuture, Either},
    task::{Context, Poll},
};
use libp2p::{PeerId, StreamProtocol};
use log::*;
use prost::Message;
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::{
    sync::{Mutex, mpsc, oneshot, watch},
    time,
};
use tower::{Service, ServiceExt};
use tracing::{Instrument, Level, span};

use super::message::RpcMethod;
use crate::{
    Handshake,
    NamedProtocolService,
    Response,
    RpcError,
    RpcHandshakeError,
    RpcServerError,
    RpcStatus,
    body::ClientStreaming,
    framing::CanonicalFraming,
    message::{BaseRequest, RpcMessageFlags},
    proto,
};

const LOG_TARGET: &str = "libp2p::rpc::client";

/// How many keepalive intervals may pass with no frame at all before a peer that was asked for
/// keepalives is treated as gone.
const MISSED_KEEPALIVES_BEFORE_TIMEOUT: u32 = 3;

#[derive(Clone)]
pub struct RpcClient {
    connector: ClientConnector,
}

impl RpcClient {
    pub fn builder<T>(peer_id: PeerId) -> RpcClientBuilder<T>
    where T: NamedProtocolService {
        RpcClientBuilder::new(peer_id).with_protocol_id(StreamProtocol::new(T::PROTOCOL_NAME))
    }

    /// Create a new RpcClient using the given framed substream and perform the RPC handshake.
    pub async fn connect<TSubstream>(
        config: RpcClientConfig,
        peer_id: PeerId,
        framed: CanonicalFraming<TSubstream>,
        protocol_name: StreamProtocol,
    ) -> Result<Self, RpcError>
    where
        TSubstream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (request_tx, request_rx) = mpsc::channel(1);
        let shutdown = Shutdown::new();
        let shutdown_signal = shutdown.to_signal();
        let (last_request_latency_tx, last_request_latency_rx) = watch::channel(None);
        let connector = ClientConnector::new(request_tx, last_request_latency_rx, shutdown);
        let (ready_tx, ready_rx) = oneshot::channel();
        let tracing_id = tracing::Span::current().id();
        tokio::spawn({
            let span = span!(Level::TRACE, "start_rpc_worker");
            span.follows_from(tracing_id);

            RpcClientWorker::new(
                config,
                peer_id,
                request_rx,
                last_request_latency_tx,
                framed,
                ready_tx,
                protocol_name,
                shutdown_signal,
            )
            .run()
            .instrument(span)
        });
        ready_rx
            .await
            .expect("ready_rx oneshot is never dropped without a reply")?;
        Ok(Self { connector })
    }

    /// Perform a single request and single response
    pub async fn request_response<T, R, M>(&mut self, request: T, method: M) -> Result<R, RpcError>
    where
        T: prost::Message,
        R: prost::Message + Default + fmt::Debug,
        M: Into<RpcMethod>,
    {
        let req_bytes = request.encode_to_vec();
        let call = ClientCall::new(BaseRequest::new(method.into(), req_bytes.into()));

        let mut resp = self.call_inner(call).await?;
        let resp = resp.messages.recv().await.ok_or(RpcError::ServerClosedRequest)??;
        let resp = R::decode(resp.into_message())?;

        Ok(resp)
    }

    /// Perform a single request and streaming response
    pub async fn server_streaming<T, M, R>(&mut self, request: T, method: M) -> Result<ClientStreaming<R>, RpcError>
    where
        T: prost::Message,
        R: prost::Message + Default,
        M: Into<RpcMethod>,
    {
        self.server_streaming_with_options(request, method, RpcRequestOptions::default())
            .await
    }

    /// Perform a single request and streaming response, overriding the session defaults for this
    /// request only. See [`RpcRequestOptions`].
    pub async fn server_streaming_with_options<T, M, R>(
        &mut self,
        request: T,
        method: M,
        options: RpcRequestOptions,
    ) -> Result<ClientStreaming<R>, RpcError>
    where
        T: prost::Message,
        R: prost::Message + Default,
        M: Into<RpcMethod>,
    {
        let req_bytes = request.encode_to_vec();
        let call = ClientCall::new(BaseRequest::new(method.into(), req_bytes.into())).with_options(options);

        let resp = self.call_inner(call).await?;

        Ok(ClientStreaming::new(resp.messages, resp.keepalives))
    }

    /// Close the RPC session. Any subsequent calls will error.
    pub async fn close(&mut self) {
        self.connector.close().await;
    }

    pub fn is_connected(&self) -> bool {
        self.connector.is_connected()
    }

    /// Return the latency of the last request
    pub fn get_last_request_latency(&mut self) -> Option<Duration> {
        self.connector.get_last_request_latency()
    }

    /// Sends a ping and returns the latency
    pub fn ping(&mut self) -> impl Future<Output = Result<Duration, RpcError>> + '_ {
        self.connector.send_ping()
    }

    async fn call_inner(&mut self, call: ClientCall) -> Result<ResponseChannels, RpcError> {
        let svc = self.connector.ready().await?;
        let resp = svc.call(call).await?;
        Ok(resp)
    }
}

impl fmt::Debug for RpcClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RpcClient {{ inner: ... }}")
    }
}

#[derive(Debug, Clone)]
pub struct RpcClientBuilder<TClient> {
    config: RpcClientConfig,
    protocol_id: Option<StreamProtocol>,
    peer_id: PeerId,
    _client: PhantomData<TClient>,
}

impl<TClient> RpcClientBuilder<TClient> {
    pub fn new(peer_id: PeerId) -> Self {
        Self {
            config: Default::default(),
            protocol_id: None,
            _client: PhantomData,
            peer_id,
        }
    }

    /// Returns the peer ID set in this builder
    pub fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    /// The deadline to send to the peer when performing a request.
    /// If this deadline is exceeded, the server SHOULD abandon the request.
    /// The client will return a timeout error if the deadline plus the grace period is exceeded.
    ///
    /// _Note: That is the deadline is set too low, the responding peer MAY immediately reject the request.
    ///
    /// Default: 100s
    pub fn with_deadline(mut self, timeout: Duration) -> Self {
        self.config.deadline = Some(timeout);
        self
    }

    /// Sets the grace period to allow after the configured deadline before giving up and timing out.
    /// This configuration should be set to comfortably account for the latency experienced during requests.
    ///
    /// Default: 10 seconds
    pub fn with_deadline_grace_period(mut self, timeout: Duration) -> Self {
        self.config.deadline_grace_period = timeout;
        self
    }

    /// Asks the server to emit an empty keepalive frame every `interval` while a streaming response
    /// has nothing to send, so that an idle stream stays distinguishable from a dead peer. The
    /// server MAY serve a longer interval than requested, and a server that does not support
    /// keepalives simply ends an idle stream at the deadline as it otherwise would.
    ///
    /// The interval is carried on the wire in whole seconds and rounds down, with a floor of one
    /// second. A client that has asked for keepalives also rejects an ACK frame it did not ask for,
    /// so this is what enables tolerating them at all.
    ///
    /// Default: no keepalives
    pub fn with_keepalive_interval(mut self, interval: Duration) -> Self {
        self.config.keepalive_interval = Some(interval);
        self
    }

    /// The shortest keepalive interval to assume the peer may serve. A server raises a request
    /// below its own minimum to that minimum and does not report the interval it chose, so the
    /// client holds it to the longer of what it asked for and this.
    ///
    /// Default: 5 seconds, the minimum a server here serves unless configured otherwise
    pub fn with_peer_minimum_keepalive_interval(mut self, interval: Duration) -> Self {
        self.config.peer_minimum_keepalive_interval = interval;
        self
    }

    /// Set the length of time that the client will wait for a response in the RPC handshake before returning a timeout
    /// error.
    /// Default: 15 seconds
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.config.handshake_timeout = timeout;
        self
    }

    /// Set the protocol ID associated with this client. This is used for logging purposes only.
    pub fn with_protocol_id(mut self, protocol_id: StreamProtocol) -> Self {
        self.protocol_id = Some(protocol_id);
        self
    }
}

impl<TClient> RpcClientBuilder<TClient>
where TClient: From<RpcClient> + NamedProtocolService
{
    /// Negotiates and establishes a session to the peer's RPC service
    pub async fn connect<TSubstream>(self, framed: CanonicalFraming<TSubstream>) -> Result<TClient, RpcError>
    where TSubstream: AsyncRead + AsyncWrite + Unpin + Send + 'static {
        RpcClient::connect(
            self.config,
            self.peer_id,
            framed,
            self.protocol_id
                .as_ref()
                .cloned()
                .unwrap_or_else(|| StreamProtocol::new(TClient::PROTOCOL_NAME)),
        )
        .await
        .map(Into::into)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RpcClientConfig {
    pub deadline: Option<Duration>,
    pub deadline_grace_period: Duration,
    pub keepalive_interval: Option<Duration>,
    /// The shortest keepalive interval to assume the *peer* may serve, whatever this client asks
    /// for. Distinct from [`RpcServerBuilder::with_minimum_keepalive_interval`], which is what a
    /// server here will serve; this is a belief about the one at the other end.
    pub peer_minimum_keepalive_interval: Duration,
    pub handshake_timeout: Duration,
}

impl RpcClientConfig {
    /// Returns the timeout including the configured grace period
    pub fn timeout_with_grace_period(&self) -> Option<Duration> {
        self.deadline.map(|d| d + self.deadline_grace_period)
    }

    /// Returns the handshake timeout
    pub fn handshake_timeout(&self) -> Duration {
        self.handshake_timeout
    }

    /// The longest gap between frames tolerated of a peer that was asked for keepalives: several
    /// of the longest interval the peer might be serving, plus the grace period.
    ///
    /// The peer raises an interval shorter than its own minimum to that minimum and does not report
    /// the interval it settled on, so asking for a short one says nothing about how often frames
    /// actually arrive. `peer_minimum_keepalive_interval` is what the client assumes about that.
    fn keepalive_timeout(&self) -> Option<Duration> {
        self.keepalive_interval.map(|interval| {
            cmp::max(interval, self.peer_minimum_keepalive_interval) * MISSED_KEEPALIVES_BEFORE_TIMEOUT +
                self.deadline_grace_period
        })
    }
}

impl Default for RpcClientConfig {
    fn default() -> Self {
        Self {
            deadline: Some(Duration::from_secs(120)),
            deadline_grace_period: Duration::from_secs(60),
            keepalive_interval: None,
            peer_minimum_keepalive_interval: crate::DEFAULT_MINIMUM_KEEPALIVE_INTERVAL,
            handshake_timeout: Duration::from_secs(90),
        }
    }
}

/// Overrides of the session defaults that apply to a single request.
///
/// A session is shared by every caller holding the client, so a deadline long enough for a stream
/// that deliberately idles must not lengthen the ordinary requests running alongside it. Any option
/// left unset takes the value the client was built with.
#[derive(Debug, Clone, Copy, Default)]
pub struct RpcRequestOptions {
    deadline: Option<Duration>,
    keepalive_interval: Option<Duration>,
}

impl RpcRequestOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The deadline to send to the peer for this request. It bounds the gap between messages that
    /// carry the response: a stream that produces none for this long is abandoned by the peer, and
    /// the client gives up a grace period later.
    ///
    /// The deadline is carried on the wire in whole seconds and rounds down, with a floor of one
    /// second. A peer MAY reject outright a deadline shorter than the minimum it accepts.
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Asks the peer to emit an empty keepalive frame every `interval` while this response has
    /// nothing to send, so that a peer that has gone away is told apart from one that is merely
    /// idle. Keepalives prove liveness within the deadline budget rather than extending it: a
    /// response that produces no real message for the deadline still ends.
    ///
    /// The interval must be comfortably shorter than the deadline in force for the request. The
    /// peer MAY serve a longer interval than asked for, so the client waits several of them before
    /// giving up.
    ///
    /// See [`RpcClientBuilder::with_keepalive_interval`] for what the peer does with the interval.
    pub fn with_keepalive_interval(mut self, interval: Duration) -> Self {
        self.keepalive_interval = Some(interval);
        self
    }

    /// The configuration a request carrying these options is served with.
    fn apply_to(&self, config: RpcClientConfig) -> RpcClientConfig {
        RpcClientConfig {
            deadline: self.deadline.or(config.deadline),
            keepalive_interval: self.keepalive_interval.or(config.keepalive_interval),
            ..config
        }
    }
}

/// What bounds a read of the next frame, beyond the session's own configuration.
#[derive(Debug, Clone, Copy, Default)]
struct ReadBounds {
    /// The instant by which a frame carrying the response must arrive. Keepalives do not move it,
    /// so a peer emitting them cannot hold a response open past the deadline it was given.
    deadline_at: Option<Instant>,
    /// Whether the peer has begun streaming a body. It emits keepalives only from that point, so
    /// before the first frame its handler may legitimately produce nothing for the whole deadline
    /// and only the deadline may bound the wait.
    keepalives_due: bool,
}

/// What a caller receives for a request: the response messages, and word of each keepalive the
/// peer sends while it has none.
pub struct ResponseChannels {
    pub messages: mpsc::Receiver<Result<Response<Bytes>, RpcStatus>>,
    pub keepalives: watch::Receiver<u64>,
}

/// A request and the per-request overrides to send it with.
pub(crate) struct ClientCall {
    request: BaseRequest<Bytes>,
    options: RpcRequestOptions,
}

impl ClientCall {
    fn new(request: BaseRequest<Bytes>) -> Self {
        Self {
            request,
            options: RpcRequestOptions::default(),
        }
    }

    fn with_options(mut self, options: RpcRequestOptions) -> Self {
        self.options = options;
        self
    }
}

#[derive(Clone)]
pub struct ClientConnector {
    inner: mpsc::Sender<ClientRequest>,
    last_request_latency_rx: watch::Receiver<Option<Duration>>,
    shutdown: Arc<Mutex<Shutdown>>,
}

impl ClientConnector {
    pub(self) fn new(
        sender: mpsc::Sender<ClientRequest>,
        last_request_latency_rx: watch::Receiver<Option<Duration>>,
        shutdown: Shutdown,
    ) -> Self {
        Self {
            inner: sender,
            last_request_latency_rx,
            shutdown: Arc::new(Mutex::new(shutdown)),
        }
    }

    pub async fn close(&mut self) {
        let lock = self.shutdown.lock().await;
        lock.trigger();
    }

    pub fn get_last_request_latency(&mut self) -> Option<Duration> {
        *self.last_request_latency_rx.borrow()
    }

    pub async fn send_ping(&mut self) -> Result<Duration, RpcError> {
        let (reply, reply_rx) = oneshot::channel();
        self.inner
            .send(ClientRequest::SendPing(reply))
            .await
            .map_err(|_| RpcError::ClientClosed)?;

        let latency = reply_rx.await.map_err(|_| RpcError::RequestCancelled)??;
        Ok(latency)
    }

    pub fn is_connected(&self) -> bool {
        !self.inner.is_closed()
    }
}

impl fmt::Debug for ClientConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClientConnector {{ inner: ... }}")
    }
}

impl Service<ClientCall> for ClientConnector {
    type Error = RpcError;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = ResponseChannels;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, call: ClientCall) -> Self::Future {
        let (reply, reply_rx) = oneshot::channel();
        let inner = self.inner.clone();
        async move {
            inner
                .send(ClientRequest::SendRequest { call, reply })
                .await
                .map_err(|_| RpcError::ClientClosed)?;

            reply_rx.await.map_err(|_| RpcError::RequestCancelled)
        }
        .boxed()
    }
}

struct RpcClientWorker<TSubstream> {
    config: RpcClientConfig,
    peer_id: PeerId,
    request_rx: mpsc::Receiver<ClientRequest>,
    last_request_latency_tx: watch::Sender<Option<Duration>>,
    framed: CanonicalFraming<TSubstream>,
    // Request ids are limited to u16::MAX because varint encoding is used over the wire and the magnitude of the value
    // sent determines the byte size. A u16 will be more than enough for the purpose
    next_request_id: u16,
    ready_tx: Option<oneshot::Sender<Result<(), RpcError>>>,
    protocol_id: StreamProtocol,
    shutdown_signal: ShutdownSignal,
    /// Whether a response has yet been read on this session. Until one has, the next frame may be
    /// the server refusing the session rather than answering.
    has_read_a_response: bool,
}

impl<TSubstream> RpcClientWorker<TSubstream>
where TSubstream: AsyncRead + AsyncWrite + Unpin + Send
{
    pub(self) fn new(
        config: RpcClientConfig,
        peer_id: PeerId,
        request_rx: mpsc::Receiver<ClientRequest>,
        last_request_latency_tx: watch::Sender<Option<Duration>>,
        framed: CanonicalFraming<TSubstream>,
        ready_tx: oneshot::Sender<Result<(), RpcError>>,
        protocol_id: StreamProtocol,
        shutdown_signal: ShutdownSignal,
    ) -> Self {
        Self {
            has_read_a_response: false,
            config,
            peer_id,
            request_rx,
            framed,
            next_request_id: 0,
            ready_tx: Some(ready_tx),
            last_request_latency_tx,
            protocol_id,
            shutdown_signal,
        }
    }

    fn protocol_name(&self) -> &str {
        self.protocol_id.as_ref()
    }

    async fn run(mut self) {
        debug!(
            target: LOG_TARGET,
            "Performing client handshake for '{}'",
            self.protocol_name()
        );
        let start = Instant::now();
        let mut handshake = Handshake::new(&mut self.framed).with_timeout(self.config.handshake_timeout());
        match handshake.perform_client_handshake().await {
            Ok(_) => {
                let latency = start.elapsed();
                debug!(
                    target: LOG_TARGET,
                    "RPC Session ({}) negotiation completed. Latency: {:.0?}",
                    self.protocol_name(),
                    latency
                );
                let _ = self.last_request_latency_tx.send(Some(latency));
                if let Some(r) = self.ready_tx.take() {
                    let _result = r.send(Ok(()));
                }
                #[cfg(feature = "metrics")]
                metrics::handshake_counter(&self.peer_id, &self.protocol_id).inc();
            },
            Err(err) => {
                #[cfg(feature = "metrics")]
                metrics::handshake_errors(&self.peer_id, &self.protocol_id).inc();
                if let Some(r) = self.ready_tx.take() {
                    let _result = r.send(Err(err.into()));
                }

                return;
            },
        }

        #[cfg(feature = "metrics")]
        metrics::num_sessions(&self.peer_id, &self.protocol_id).inc();
        loop {
            tokio::select! {
                // Check the futures in the order they are listed
                biased;
                _ = &mut self.shutdown_signal => {
                    break;
                },
                server_msg = self.framed.next() => {
                    match server_msg {
                        Some(Ok(msg)) => {
                            if let Err(err) = self.handle_interrupt_server_message(msg) {
                                #[cfg(feature = "metrics")]
                                metrics::handshake_errors(&self.peer_id, &self.protocol_id).inc();
                                error!(target: LOG_TARGET, "(peer={}) Unexpected error: {}. Worker is terminating.", self.peer_id, err);
                                break;
                            }
                        },
                        Some(Err(err)) => {
                            debug!(target: LOG_TARGET, "(peer={}) IO Error: {}. Worker is terminating.", self.peer_id, err);
                            break;
                        },
                        None => {
                            debug!(target: LOG_TARGET, "(peer={}) Substream closed. Worker is terminating.", self.peer_id);
                            break;
                        }
                    }
                },
                req = self.request_rx.recv() => {
                    match req {
                        Some(req) => {
                            if let Err(err) = self.handle_request(req).await {
                                #[cfg(feature = "metrics")]
                                metrics::client_errors(&self.peer_id, &self.protocol_id).inc();
                                error!(target: LOG_TARGET, "(peer={}) Unexpected error: {}. Worker is terminating.", self.peer_id, err);
                                break;
                            }
                        }
                        None => {
                            debug!(target: LOG_TARGET, "(peer={}) Request channel closed. Worker is terminating.", self.peer_id);
                            break
                        },
                    }
                }
            }
        }
        #[cfg(feature = "metrics")]
        metrics::num_sessions(&self.peer_id, &self.protocol_id).dec();

        if let Err(err) = self.framed.close().await {
            debug!(
                target: LOG_TARGET,
                "(peer: {}) IO Error when closing substream: {}",
                self.peer_id,
                err
            );
        }

        debug!(
            target: LOG_TARGET,
            "(peer: {}) RpcClientWorker ({}) terminated.",
            self.peer_id,
            self.protocol_name()
        );
    }

    fn handle_interrupt_server_message(&self, msg: BytesMut) -> Result<(), RpcError> {
        let msg = proto::RpcSessionReply::decode(&mut msg.freeze())?;
        let version = msg
            .result()
            .map_err(|e| RpcError::HandshakeError(RpcHandshakeError::Rejected(e)))?;
        debug!(target: LOG_TARGET, "Server accepted version {}", version);
        Ok(())
    }

    async fn handle_request(&mut self, req: ClientRequest) -> Result<(), RpcError> {
        use ClientRequest::{SendPing, SendRequest};
        match req {
            SendRequest { call, reply } => {
                self.do_request_response(call, reply).await?;
            },
            SendPing(reply) => {
                self.do_ping_pong(reply).await?;
            },
        }
        Ok(())
    }

    async fn do_ping_pong(&mut self, reply: oneshot::Sender<Result<Duration, RpcStatus>>) -> Result<(), RpcError> {
        let ack = proto::RpcRequest {
            flags: u32::from(RpcMessageFlags::ACK.bits()),
            deadline: self.config.deadline.map(|t| t.as_secs().max(1)).unwrap_or(0),
            ..Default::default()
        };

        let start = Instant::now();
        self.framed.send(ack.encode_to_vec().into()).await?;

        trace!(
            target: LOG_TARGET,
            "(peer={}) Ping (protocol {}) sent in {:.2?}",
            self.peer_id,
            self.protocol_name(),
            start.elapsed()
        );
        let mut reader = RpcResponseReader::new(&mut self.framed, self.config, 0);
        let resp = match reader.read_ack().await {
            Ok(resp) => resp,
            Err(RpcError::ReplyTimeout) => {
                debug!(
                    target: LOG_TARGET,
                    "(peer={}) Ping timed out after {:.0?}",
                    self.peer_id,
                    start.elapsed()
                );
                #[cfg(feature = "metrics")]
                metrics::client_timeouts(&self.peer_id, &self.protocol_id).inc();
                let _result = reply.send(Err(RpcStatus::timed_out("Response timed out")));
                return Ok(());
            },
            Err(err) => return Err(err),
        };

        let status = RpcStatus::from(&resp);
        if !status.is_ok() {
            let _result = reply.send(Err(status.clone()));
            return Err(status.into());
        }

        let resp_flags =
            RpcMessageFlags::from_bits(u8::try_from(resp.flags).map_err(|_| {
                RpcStatus::protocol_error(format!("invalid message flag: must be less than {}", u8::MAX))
            })?)
            .ok_or(RpcStatus::protocol_error(format!(
                "invalid message flag, does not match any flags ({})",
                resp.flags
            )))?;
        if !resp_flags.contains(RpcMessageFlags::ACK) {
            warn!(
                target: LOG_TARGET,
                "(peer={}) Invalid ping response {:?}",
                self.peer_id,
                resp
            );
            let _result = reply.send(Err(RpcStatus::protocol_error(format!(
                "Received invalid ping response on protocol '{}'",
                self.protocol_name()
            ))));
            return Err(RpcError::InvalidPingResponse);
        }

        let _result = reply.send(Ok(start.elapsed()));
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn do_request_response(
        &mut self,
        call: ClientCall,
        reply: oneshot::Sender<ResponseChannels>,
    ) -> Result<(), RpcError> {
        let ClientCall { request, options } = call;
        let config = options.apply_to(self.config);

        #[cfg(feature = "metrics")]
        metrics::outbound_request_bytes(&self.peer_id, &self.protocol_id).observe(request.get_ref().len() as f64);

        let request_id = self.next_request_id();
        let method = request.method.into();
        let req = proto::RpcRequest {
            request_id: u32::from(request_id),
            method,
            deadline: config.deadline.map(|t| t.as_secs().max(1)).unwrap_or(0),
            keepalive_interval: config.keepalive_interval.map(|t| t.as_secs().max(1)).unwrap_or(0),
            flags: 0,
            payload: request.message.to_vec(),
        };

        trace!(target: LOG_TARGET, "Sending request: {}", req);

        if reply.is_closed() {
            warn!(
                target: LOG_TARGET,
                "Client request was cancelled before request was sent"
            );
        }

        let (response_tx, response_rx) = mpsc::channel(5);
        let (keepalive_tx, keepalive_rx) = watch::channel(0u64);
        if let Err(mut channels) = reply.send(ResponseChannels {
            messages: response_rx,
            keepalives: keepalive_rx,
        }) {
            warn!(
                target: LOG_TARGET,
                "Client request was cancelled after request was sent. This means that you are making an RPC request \
                 and then immediately dropping the response! (protocol = {})",
                self.protocol_name(),
            );
            channels.messages.close();
            return Ok(());
        }

        #[cfg(feature = "metrics")]
        let latency = metrics::request_response_latency(&self.peer_id, &self.protocol_id);
        #[cfg(feature = "metrics")]
        let mut metrics_timer = Some(latency.start_timer());

        let mut bounds = ReadBounds {
            deadline_at: config.deadline.map(|d| Instant::now() + d),
            keepalives_due: false,
        };

        let timer = Instant::now();
        if let Err(err) = self.send_request(req).await {
            warn!(target: LOG_TARGET, "{}", err);
            #[cfg(feature = "metrics")]
            metrics::client_errors(&self.peer_id, &self.protocol_id).inc();
            let _result = response_tx.send(Err(err.into())).await;
            return Ok(());
        }
        let partial_latency = timer.elapsed();

        loop {
            if self.shutdown_signal.is_triggered() {
                debug!(
                    target: LOG_TARGET,
                    "[peer: {}, protocol: {}, req_id: {}] Client connector closed. Quitting stream \
                     early",
                    self.peer_id,
                    self.protocol_name(),
                    request_id
                );
                break;
            }

            // Check if the response receiver has been dropped while receiving messages
            let resp_result = {
                let resp_fut = self.read_response(request_id, config, bounds, &keepalive_tx);
                tokio::pin!(resp_fut);
                let closed_fut = response_tx.closed();
                tokio::pin!(closed_fut);
                match future::select(resp_fut, closed_fut).await {
                    Either::Left((r, _)) => Some(r),
                    Either::Right(_) => None,
                }
            };
            let resp_result = match resp_result {
                Some(r) => r,
                None => {
                    // The consumer has dropped the receiver before all responses are received.
                    // Closing
                    break;
                },
            };

            let resp = match resp_result {
                Ok((resp, time_to_first_msg)) => {
                    if let Some(t) = time_to_first_msg {
                        let _ = self.last_request_latency_tx.send(Some(partial_latency + t));
                    }
                    trace!(
                        target: LOG_TARGET,
                        "Received response ({} byte(s)) from request #{} (protocol = {}, method={})",
                        resp.payload.len(),
                        request_id,
                        self.protocol_name(),
                        method,
                    );

                    #[cfg(feature = "metrics")]
                    if let Some(t) = metrics_timer.take() {
                        t.observe_duration();
                    }
                    resp
                },
                Err(RpcError::ReplyTimeout) => {
                    debug!(
                        target: LOG_TARGET,
                        "Request {} (method={}) timed out (resp_closed={})", request_id, method, response_tx.is_closed()
                    );
                    #[cfg(feature = "metrics")]
                    metrics::client_timeouts(&self.peer_id, &self.protocol_id).inc();
                    if response_tx.is_closed() {
                        // The consumer has dropped the receiver before all responses are received.
                        // We have timed out on the response but since we've closed, we just exit
                    } else {
                        let _result = response_tx.send(Err(RpcStatus::timed_out("Response timed out"))).await;
                    }
                    break;
                },
                Err(RpcError::ClientClosed) => {
                    debug!(
                        target: LOG_TARGET,
                        "Request {} (method={}) was closed (read_reply)", request_id, method,
                    );
                    if !response_tx.is_closed() {
                        let _result = response_tx
                            .send(Err(RpcStatus::general(
                                "The RPC client closed before the response completed",
                            )))
                            .await;
                    }
                    self.request_rx.close();
                    break;
                },
                Err(RpcError::HandshakeError(RpcHandshakeError::Rejected(reason))) => {
                    debug!(
                        target: LOG_TARGET,
                        "(peer={}) Server refused the session: {}", self.peer_id, reason
                    );
                    let _result = response_tx
                        .send(Err(RpcStatus::handshake_denied(reason.to_string())))
                        .await;
                    break;
                },
                // The consumer must see the failure: a stream that just ends is indistinguishable from one the
                // peer chose to end.
                Err(err) => {
                    if !response_tx.is_closed() {
                        let _result = response_tx.send(Err(RpcStatus::general(err.to_string()))).await;
                    }
                    return Err(err);
                },
            };
            bounds.keepalives_due = true;
            self.has_read_a_response = true;

            match Self::convert_to_result(resp) {
                Ok(resp) => {
                    let is_finished = resp.is_finished();
                    // If the consumer drops the receiver, we can stop sending responses.
                    if response_tx.is_closed() {
                        // We have timed out on the response but since we've closed, we just exit
                        break;
                    } else {
                        let _result = response_tx.send(Ok(resp)).await;
                    }
                    // No earlier than the peer, which restarts its own budget once the write
                    // returns. Starting on receipt instead would spend a stalled consumer's time
                    // out of the budget and abandon a response the peer still considers live; the
                    // cost of starting later is a timeout late by however long the consumer held
                    // the send above.
                    bounds.deadline_at = config.deadline.map(|d| Instant::now() + d);
                    if is_finished {
                        break;
                    }
                },
                Err(err) => {
                    debug!(target: LOG_TARGET, "Remote service returned error: {}", err);
                    if !response_tx.is_closed() {
                        let _result = response_tx.send(Err(err.clone())).await;
                    }
                    if err.as_status_code().is_handshake_denied() {
                        return Err(err.into());
                    }
                    break;
                },
            }
        }

        Ok(())
    }

    async fn send_request(&mut self, req: proto::RpcRequest) -> Result<(), RpcError> {
        let payload = req.encode_to_vec();
        if payload.len() > crate::max_request_size() {
            return Err(RpcError::MaxRequestSizeExceeded {
                got: payload.len(),
                expected: crate::max_request_size(),
            });
        }
        self.framed.send(payload.into()).await?;
        Ok(())
    }

    async fn read_response(
        &mut self,
        request_id: u16,
        config: RpcClientConfig,
        bounds: ReadBounds,
        keepalive_tx: &watch::Sender<u64>,
    ) -> Result<(proto::RpcResponse, Option<Duration>), RpcError> {
        let peer_id = self.peer_id;
        let protocol_name = self.protocol_name().to_string();

        let may_be_session_rejection = !self.has_read_a_response;
        let mut reader = RpcResponseReader::new(&mut self.framed, config, request_id)
            .watching_for_session_rejection(may_be_session_rejection)
            .bounded_by(bounds)
            .reporting_keepalives_to(keepalive_tx);
        let resp = loop {
            match reader.read_response().await {
                Ok(resp) => {
                    trace!(
                        target: LOG_TARGET,
                        "(peer: {}, {}) Received body len = {}",
                        peer_id,
                        protocol_name,
                        reader.bytes_read()
                    );
                    #[cfg(feature = "metrics")]
                    metrics::inbound_response_bytes(&self.peer_id, &self.protocol_id)
                        .observe(reader.bytes_read() as f64);
                    let time_to_first_msg = reader.time_to_first_msg();
                    break (resp, time_to_first_msg);
                },
                // A frame for an earlier request was in flight when this one went out — abandoning
                // a stream can leave arbitrarily many, from more than one request back. Skipping
                // them is bounded by the deadline this read already carries, so neither how many
                // arrive nor which request they were for matters.
                Err(RpcError::ResponseIdDidNotMatchRequest { actual, .. }) => {
                    trace!(
                        target: LOG_TARGET,
                        "(peer: {}, {}) Discarding a delayed frame for request {}", peer_id, protocol_name, actual
                    );
                    continue;
                },
                Err(err) => return Err(err),
            }
        };
        Ok(resp)
    }

    fn next_request_id(&mut self) -> u16 {
        let mut next_id = self.next_request_id;
        // request_id is allowed to wrap around back to 0
        self.next_request_id = self.next_request_id.wrapping_add(1);
        // We dont want request id of zero because that is the default for varint on protobuf, so it is possible for the
        // entire message to be zero bytes (WriteZero IO error)
        if next_id == 0 {
            next_id += 1;
            self.next_request_id += 1;
        }
        next_id
    }

    fn convert_to_result(resp: proto::RpcResponse) -> Result<Response<Bytes>, RpcStatus> {
        let status = RpcStatus::from(&resp);
        if !status.is_ok() {
            return Err(status);
        }
        let flags = match resp.flags() {
            Ok(flags) => flags,
            Err(e) => return Err(RpcError::ServerError(RpcServerError::ProtocolError(e)).into()),
        };

        Ok(Response {
            flags,
            payload: resp.payload.into(),
        })
    }
}

pub enum ClientRequest {
    SendRequest {
        call: ClientCall,
        reply: oneshot::Sender<ResponseChannels>,
    },
    SendPing(oneshot::Sender<Result<Duration, RpcStatus>>),
}

struct RpcResponseReader<'a, TSubstream> {
    framed: &'a mut CanonicalFraming<TSubstream>,
    config: RpcClientConfig,
    request_id: u16,
    bounds: ReadBounds,
    /// Whether a frame may still turn out to be the session rejection reply. The handshake is
    /// one-way, so a server that refuses the session answers the first request rather than the
    /// handshake, and the refusal has to be recognised where a response was expected.
    may_be_session_rejection: bool,
    /// Told of each keepalive addressed to this request.
    keepalive_tx: Option<&'a watch::Sender<u64>>,
    bytes_read: usize,
    time_to_first_msg: Option<Duration>,
}

impl<'a, TSubstream> RpcResponseReader<'a, TSubstream>
where TSubstream: AsyncRead + AsyncWrite + Unpin
{
    pub fn new(framed: &'a mut CanonicalFraming<TSubstream>, config: RpcClientConfig, request_id: u16) -> Self {
        Self {
            framed,
            config,
            request_id,
            bounds: ReadBounds::default(),
            may_be_session_rejection: false,
            keepalive_tx: None,
            bytes_read: 0,
            time_to_first_msg: None,
        }
    }

    pub fn reporting_keepalives_to(mut self, keepalive_tx: &'a watch::Sender<u64>) -> Self {
        self.keepalive_tx = Some(keepalive_tx);
        self
    }

    pub fn bounded_by(mut self, bounds: ReadBounds) -> Self {
        self.bounds = bounds;
        self
    }

    pub fn watching_for_session_rejection(mut self, may_be_session_rejection: bool) -> Self {
        self.may_be_session_rejection = may_be_session_rejection;
        self
    }

    pub fn bytes_read(&self) -> usize {
        self.bytes_read
    }

    pub fn time_to_first_msg(&self) -> Option<Duration> {
        self.time_to_first_msg
    }

    pub async fn read_response(&mut self) -> Result<proto::RpcResponse, RpcError> {
        let timer = Instant::now();
        let resp = loop {
            let resp = self.next().await?;
            if resp.is_keepalive() {
                // A keepalive carries neither payload nor stream position, so its request id is not
                // policed for the read: one left over from an abandoned request is as harmless as
                // one for this request. A peer that sends one unasked cannot gain anything by it
                // either, since only a frame carrying a response moves the deadline.
                //
                // It is also evidence that the peer has begun streaming, and the only such evidence
                // that is not id-policed, so this is a guess where the rest is not.
                self.bounds.keepalives_due = true;
                // What the consumer is told is a claim about this response - the peer is there and
                // has nothing more to send on it - so only a keepalive addressed to it counts.
                if let Some(tx) = self.keepalive_tx &&
                    self.check_response(&resp).is_ok()
                {
                    tx.send_modify(|count| *count += 1);
                }
                continue;
            }
            break resp;
        };
        self.time_to_first_msg = Some(timer.elapsed());
        self.check_response(&resp)?;
        self.bytes_read = resp.payload.len();
        trace!(
            target: LOG_TARGET,
            "Received {} bytes in {:.2?}",
            resp.payload.len(),
            self.time_to_first_msg.unwrap_or_default()
        );
        Ok(resp)
    }

    /// Reads the pong for a ping. Frames left over from an abandoned request carry the same ACK
    /// flag, so only one addressed to this request answers the ping. Skipping the others is bounded
    /// by a single timeout over the whole read, because each frame restarts the per-frame one.
    pub async fn read_ack(&mut self) -> Result<proto::RpcResponse, RpcError> {
        let timeout = self.config.timeout_with_grace_period();
        let read_pong = async {
            loop {
                let resp = self.next().await?;
                if self.check_response(&resp).is_ok() {
                    return Ok(resp);
                }
            }
        };

        match timeout {
            Some(timeout) => time::timeout(timeout, read_pong)
                .await
                .map_err(|_| RpcError::ReplyTimeout)?,
            None => read_pong.await,
        }
    }

    fn check_response(&self, resp: &proto::RpcResponse) -> Result<(), RpcError> {
        let resp_id = u16::try_from(resp.request_id)
            .map_err(|_| RpcStatus::protocol_error(format!("invalid request_id: must be less than {}", u16::MAX)))?;

        if resp_id != self.request_id {
            return Err(RpcError::ResponseIdDidNotMatchRequest {
                expected: self.request_id,
                actual: u16::try_from(resp.request_id).map_err(|_| {
                    RpcStatus::protocol_error(format!("invalid request_id: must be less than {}", u16::MAX))
                })?,
            });
        }

        Ok(())
    }

    /// How long to wait for the next frame of any kind: no later than the deadline the peer was
    /// given for the response, plus a grace period for latency, and — once the peer is streaming a
    /// body and so owes keepalives — no longer than the gap tolerated of a peer that promised them.
    fn frame_timeout(&self) -> Option<Duration> {
        let until_deadline = self
            .bounds
            .deadline_at
            .map(|at| (at + self.config.deadline_grace_period).saturating_duration_since(Instant::now()));
        // A peer owes no keepalives until it is streaming a body, so its tolerance does not apply
        // before the first frame — except where no deadline bounds the read either, since the
        // worker serves this session serially and an unbounded read hangs every caller on it.
        let until_keepalives_missed = if self.bounds.keepalives_due || until_deadline.is_none() {
            self.config.keepalive_timeout()
        } else {
            None
        };
        match (until_deadline, until_keepalives_missed) {
            (Some(deadline), Some(keepalive)) => Some(cmp::min(deadline, keepalive)),
            (deadline, keepalive) => deadline.or(keepalive),
        }
    }

    async fn next(&mut self) -> Result<proto::RpcResponse, RpcError> {
        let next_msg_fut = match self.frame_timeout() {
            Some(timeout) => Either::Left(time::timeout(timeout, self.framed.next())),
            None => Either::Right(self.framed.next().map(Ok)),
        };

        match next_msg_fut.await {
            Ok(Some(Ok(frame))) => {
                let resp = proto::RpcResponse::decode(frame.as_ref())?;
                // Request ids start at one, so a frame without one is not a response at all. On the
                // first frame of a session that means the server refused it: the handshake is
                // one-way, so a refusal arrives where the first response was expected.
                if self.may_be_session_rejection &&
                    resp.request_id == 0 &&
                    let Some(reason) = crate::handshake::decode_session_rejection(&frame)
                {
                    return Err(RpcError::HandshakeError(RpcHandshakeError::Rejected(reason)));
                }
                Ok(resp)
            },
            Ok(Some(Err(err))) => Err(err.into()),
            Ok(None) => Err(RpcError::ServerClosedRequest),
            Err(_) => Err(RpcError::ReplyTimeout),
        }
    }
}

#[cfg(test)]
mod keepalive_tests {
    use tokio::io::DuplexStream;
    use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

    use super::*;
    use crate::{
        Handshake,
        RPC_MAX_FRAME_SIZE,
        RpcStatusCode,
        framing,
        message::{RpcMessageFlags, RpcResponse},
    };

    const PROTOCOL: &str = "/test/keepalive/1.0";

    async fn send_response(framed: &mut CanonicalFraming<Compat<DuplexStream>>, resp: RpcResponse) {
        framed.send(resp.to_proto().encode_to_vec().into()).await.unwrap();
    }

    async fn read_request(framed: &mut CanonicalFraming<Compat<DuplexStream>>) -> proto::RpcRequest {
        Handshake::new(framed).perform_server_handshake().await.unwrap();
        let frame = framed.next().await.unwrap().unwrap();
        proto::RpcRequest::decode(frame.freeze()).unwrap()
    }

    /// Sends a stream of exactly one message. The streaming protocol terminates with an empty FIN
    /// frame, which is not delivered to a consumer.
    async fn send_message(framed: &mut CanonicalFraming<Compat<DuplexStream>>, request_id: u32, message: Bytes) {
        send_response(framed, RpcResponse {
            request_id,
            status: RpcStatusCode::Ok,
            flags: RpcMessageFlags::empty(),
            payload: message,
        })
        .await;
        send_response(framed, RpcResponse {
            request_id,
            status: RpcStatusCode::Ok,
            flags: RpcMessageFlags::FIN,
            payload: Bytes::new(),
        })
        .await;
    }

    /// Replies to one streaming request with `num_keepalives` empty keepalive frames bearing
    /// `keepalive_id`, then a single message closing the stream.
    async fn serve_keepalives_then_message(
        substream: DuplexStream,
        num_keepalives: usize,
        keepalive_id: Option<u32>,
        message: Bytes,
    ) -> proto::RpcRequest {
        let mut framed = framing::canonical(substream.compat(), RPC_MAX_FRAME_SIZE);
        let request = read_request(&mut framed).await;

        for _ in 0..num_keepalives {
            send_response(
                &mut framed,
                RpcResponse::keepalive(keepalive_id.unwrap_or(request.request_id)),
            )
            .await;
        }
        send_message(&mut framed, request.request_id, message).await;

        request
    }

    #[tokio::test]
    async fn a_stream_cut_off_mid_way_ends_with_an_error() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let request = read_request(&mut framed).await;
            send_response(&mut framed, RpcResponse {
                request_id: request.request_id,
                status: RpcStatusCode::Ok,
                flags: RpcMessageFlags::empty(),
                payload: reply.encode_to_vec().into(),
            })
            .await;
        });

        let mut rpc_client = RpcClient::connect(
            RpcClientConfig::default(),
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let stream = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap();
        server.await.unwrap();
        let received = stream.collect::<Vec<_>>().await;

        assert_eq!(received.len(), 2, "{received:?}");
        assert_eq!(received[0].as_ref().unwrap().supported_versions, vec![7]);
        assert!(received[1].is_err(), "{received:?}");
    }

    #[tokio::test]
    async fn keepalive_frames_are_not_delivered_as_stream_messages() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let server = tokio::spawn(serve_keepalives_then_message(
            server,
            3,
            None,
            reply.encode_to_vec().into(),
        ));

        let config = RpcClientConfig {
            keepalive_interval: Some(Duration::from_secs(11)),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let stream = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap();
        let received = stream.collect::<Vec<_>>().await;

        let request = server.await.unwrap();
        assert_eq!(request.keepalive_interval, 11);
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].as_ref().unwrap().supported_versions, vec![7]);
    }

    #[tokio::test]
    async fn keepalives_for_this_request_are_reported_to_the_consumer_and_others_are_not() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let message: Bytes = reply.encode_to_vec().into();
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let request = read_request(&mut framed).await;
            for _ in 0..3 {
                send_response(&mut framed, RpcResponse::keepalive(request.request_id)).await;
            }
            // Left over from a request this client abandoned: not this response's liveness.
            send_response(&mut framed, RpcResponse::keepalive(9999)).await;
            send_message(&mut framed, request.request_id, message).await;
            request
        });

        let config = RpcClientConfig {
            keepalive_interval: Some(Duration::from_secs(5)),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let stream = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap();
        let mut keepalives = stream.keepalives();
        let received = stream.collect::<Vec<_>>().await;
        server.await.unwrap();

        assert_eq!(received.len(), 1);
        assert_eq!(*keepalives.borrow_and_update(), 3);
        // The response is over, so its sender is gone and nothing more will be reported.
        assert!(keepalives.changed().await.is_err());
    }

    #[tokio::test]
    async fn keepalives_left_over_from_an_abandoned_request_do_not_end_the_session() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let message: Bytes = reply.encode_to_vec().into();
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            // The first request is answered after one more keepalive than MAX_ALLOWED_IGNORED, all
            // of them bearing an id the client is no longer waiting on.
            let first = read_request(&mut framed).await;
            for _ in 0..21 {
                send_response(&mut framed, RpcResponse::keepalive(9999)).await;
            }
            send_message(&mut framed, first.request_id, message.clone()).await;

            // A second request only gets served if the session survived those frames.
            let frame = framed.next().await.unwrap().unwrap();
            let second = proto::RpcRequest::decode(frame.freeze()).unwrap();
            send_message(&mut framed, second.request_id, message).await;
            first
        });

        let config = RpcClientConfig {
            keepalive_interval: Some(Duration::from_secs(5)),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let stream = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap();
        let received = stream.collect::<Vec<_>>().await;

        assert_eq!(received.len(), 1);
        assert_eq!(received[0].as_ref().unwrap().supported_versions, vec![7]);

        let second = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        server.await.unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].as_ref().unwrap().supported_versions, vec![7]);
    }

    #[tokio::test]
    async fn a_keepalive_that_was_never_asked_for_is_discarded() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let server = tokio::spawn(serve_keepalives_then_message(
            server,
            1,
            None,
            reply.encode_to_vec().into(),
        ));

        let mut rpc_client = RpcClient::connect(
            RpcClientConfig::default(),
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let stream = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap();
        let received = stream.collect::<Vec<_>>().await;

        let request = server.await.unwrap();
        assert_eq!(request.keepalive_interval, 0);
        // A peer that sends one unasked gains nothing by it, since only a frame carrying a response
        // moves the deadline. Refusing the session over it would cost every caller sharing it.
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].as_ref().unwrap().supported_versions, vec![7]);
    }

    #[tokio::test]
    async fn request_options_override_the_session_defaults_on_the_wire() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let server = tokio::spawn(serve_keepalives_then_message(
            server,
            2,
            None,
            reply.encode_to_vec().into(),
        ));

        // The session asks for no keepalives and carries the default deadline.
        let mut rpc_client = RpcClient::connect(
            RpcClientConfig::default(),
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let stream = rpc_client
            .server_streaming_with_options::<_, _, proto::RpcSession>(
                proto::RpcSession::default(),
                1u32,
                RpcRequestOptions::new()
                    .with_deadline(Duration::from_secs(600))
                    .with_keepalive_interval(Duration::from_secs(30)),
            )
            .await
            .unwrap();
        let received = stream.collect::<Vec<_>>().await;

        let request = server.await.unwrap();
        assert_eq!(request.deadline, 600);
        assert_eq!(request.keepalive_interval, 30);
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].as_ref().unwrap().supported_versions, vec![7]);
    }

    #[tokio::test]
    async fn a_request_without_options_keeps_the_session_defaults() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let server = tokio::spawn(serve_keepalives_then_message(
            server,
            0,
            None,
            reply.encode_to_vec().into(),
        ));

        let config = RpcClientConfig {
            deadline: Some(Duration::from_secs(45)),
            keepalive_interval: Some(Duration::from_secs(11)),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let stream = rpc_client
            .server_streaming_with_options::<_, _, proto::RpcSession>(
                proto::RpcSession::default(),
                1u32,
                RpcRequestOptions::new(),
            )
            .await
            .unwrap();
        stream.collect::<Vec<_>>().await;

        let request = server.await.unwrap();
        assert_eq!(request.deadline, 45);
        assert_eq!(request.keepalive_interval, 11);
    }

    #[tokio::test]
    async fn a_stray_keepalive_does_not_disturb_a_later_request() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let message: Bytes = reply.encode_to_vec().into();
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let first = read_request(&mut framed).await;
            send_message(&mut framed, first.request_id, message.clone()).await;

            // A frame from the first request is still in flight behind the second.
            let frame = framed.next().await.unwrap().unwrap();
            let second = proto::RpcRequest::decode(frame.freeze()).unwrap();
            send_response(&mut framed, RpcResponse::keepalive(first.request_id)).await;
            send_message(&mut framed, second.request_id, message).await;
            (first, second)
        });

        let mut rpc_client = RpcClient::connect(
            RpcClientConfig::default(),
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        rpc_client
            .server_streaming_with_options::<_, _, proto::RpcSession>(
                proto::RpcSession::default(),
                1u32,
                RpcRequestOptions::new().with_keepalive_interval(Duration::from_secs(5)),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        let second = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        let (first_request, second_request) = server.await.unwrap();
        assert_eq!(first_request.keepalive_interval, 5);
        assert_eq!(second_request.keepalive_interval, 0);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].as_ref().unwrap().supported_versions, vec![7]);
    }

    /// Streams keepalives every `every` for as long as the client keeps the session open, and never
    /// a message carrying a response.
    async fn serve_only_keepalives(substream: DuplexStream, every: Duration) {
        let mut framed = framing::canonical(substream.compat(), RPC_MAX_FRAME_SIZE);
        let request = read_request(&mut framed).await;
        loop {
            time::sleep(every).await;
            if send_response_checked(&mut framed, RpcResponse::keepalive(request.request_id))
                .await
                .is_err()
            {
                break;
            }
        }
    }

    async fn send_response_checked(
        framed: &mut CanonicalFraming<Compat<DuplexStream>>,
        resp: RpcResponse,
    ) -> Result<(), std::io::Error> {
        framed.send(resp.to_proto().encode_to_vec().into()).await
    }

    #[tokio::test]
    async fn keepalives_do_not_extend_the_deadline_they_prove_liveness_within() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        // Keepalives arrive far more often than the deadline, and the gap tolerated of a peer that
        // was asked for them is far longer, so only the deadline can end this stream.
        let server = tokio::spawn(serve_only_keepalives(server, Duration::from_millis(50)));

        let config = RpcClientConfig {
            deadline: Some(Duration::from_secs(1)),
            deadline_grace_period: Duration::from_millis(200),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let started = Instant::now();
        let received = time::timeout(
            Duration::from_secs(10),
            rpc_client
                .server_streaming_with_options::<_, _, proto::RpcSession>(
                    proto::RpcSession::default(),
                    1u32,
                    RpcRequestOptions::new().with_keepalive_interval(Duration::from_secs(2)),
                )
                .await
                .unwrap()
                .collect::<Vec<_>>(),
        )
        .await
        .expect("keepalives held the response open past its deadline");
        let elapsed = started.elapsed();

        server.abort();
        assert_eq!(
            received[0].as_ref().unwrap_err().as_status_code(),
            RpcStatusCode::Timeout
        );
        assert!(
            elapsed < Duration::from_secs(3),
            "expected the deadline to end the stream, took {elapsed:.2?}"
        );
    }

    #[tokio::test]
    async fn a_peer_that_stops_sending_the_keepalives_it_promised_is_given_up_on() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        // The peer starts streaming keepalives, then goes silent without closing the substream.
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let request = read_request(&mut framed).await;
            for _ in 0..2 {
                send_response(&mut framed, RpcResponse::keepalive(request.request_id)).await;
                time::sleep(Duration::from_millis(100)).await;
            }
            future::pending::<()>().await;
        });

        let config = RpcClientConfig {
            deadline: Some(Duration::from_secs(60)),
            deadline_grace_period: Duration::from_millis(300),
            peer_minimum_keepalive_interval: Duration::from_millis(100),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let started = Instant::now();
        let received = time::timeout(
            Duration::from_secs(10),
            rpc_client
                .server_streaming_with_options::<_, _, proto::RpcSession>(
                    proto::RpcSession::default(),
                    1u32,
                    RpcRequestOptions::new().with_keepalive_interval(Duration::from_millis(100)),
                )
                .await
                .unwrap()
                .collect::<Vec<_>>(),
        )
        .await
        .expect("a silent peer held the response open for the whole deadline");
        let elapsed = started.elapsed();

        server.abort();
        assert_eq!(
            received[0].as_ref().unwrap_err().as_status_code(),
            RpcStatusCode::Timeout
        );
        // Three missed intervals plus the grace period, well inside the 60s deadline.
        assert!(
            elapsed < Duration::from_secs(3),
            "expected the missed keepalives to end the stream, took {elapsed:.2?}"
        );
    }

    #[tokio::test]
    async fn a_peer_yet_to_send_a_first_frame_is_bounded_only_by_its_deadline() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let message: Bytes = reply.encode_to_vec().into();
        // A peer runs the handler to completion before it streams anything, and emits no keepalives
        // while it does, so silence up to the deadline is legitimate here.
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let request = read_request(&mut framed).await;
            time::sleep(Duration::from_secs(1)).await;
            send_message(&mut framed, request.request_id, message).await;
        });

        let config = RpcClientConfig {
            deadline: Some(Duration::from_secs(10)),
            deadline_grace_period: Duration::from_millis(200),
            peer_minimum_keepalive_interval: Duration::from_millis(100),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        // The tolerated gap once keepalives are due is 500ms, far short of the peer's first frame.
        let received = rpc_client
            .server_streaming_with_options::<_, _, proto::RpcSession>(
                proto::RpcSession::default(),
                1u32,
                RpcRequestOptions::new().with_keepalive_interval(Duration::from_millis(100)),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        server.await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].as_ref().unwrap().supported_versions, vec![7]);
    }

    #[tokio::test]
    async fn a_frame_from_more_than_one_request_ago_is_discarded() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let message: Bytes = reply.encode_to_vec().into();
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let first = read_request(&mut framed).await;
            send_message(&mut framed, first.request_id, message.clone()).await;

            let frame = framed.next().await.unwrap().unwrap();
            let second = proto::RpcRequest::decode(frame.freeze()).unwrap();
            send_message(&mut framed, second.request_id, message.clone()).await;

            // A frame for the first request arrives while the third is outstanding, two ids back.
            let frame = framed.next().await.unwrap().unwrap();
            let third = proto::RpcRequest::decode(frame.freeze()).unwrap();
            send_response(&mut framed, RpcResponse {
                request_id: first.request_id,
                status: RpcStatusCode::Ok,
                flags: RpcMessageFlags::empty(),
                payload: message.clone(),
            })
            .await;
            send_message(&mut framed, third.request_id, message).await;
        });

        let mut rpc_client = RpcClient::connect(
            RpcClientConfig::default(),
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        for _ in 0..2 {
            rpc_client
                .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
                .await
                .unwrap()
                .collect::<Vec<_>>()
                .await;
        }

        let received = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        server.await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].as_ref().unwrap().supported_versions, vec![7]);
    }

    #[tokio::test]
    async fn a_read_is_bounded_even_when_the_session_carries_no_deadline() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        // The peer answers the handshake and then never sends anything at all.
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            read_request(&mut framed).await;
            future::pending::<()>().await;
        });

        let config = RpcClientConfig {
            deadline: None,
            deadline_grace_period: Duration::from_millis(200),
            peer_minimum_keepalive_interval: Duration::from_millis(100),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let received = time::timeout(
            Duration::from_secs(5),
            rpc_client
                .server_streaming_with_options::<_, _, proto::RpcSession>(
                    proto::RpcSession::default(),
                    1u32,
                    RpcRequestOptions::new().with_keepalive_interval(Duration::from_millis(100)),
                )
                .await
                .unwrap()
                .collect::<Vec<_>>(),
        )
        .await
        .expect("a session without a deadline left the read unbounded");

        server.abort();
        assert_eq!(
            received[0].as_ref().unwrap_err().as_status_code(),
            RpcStatusCode::Timeout
        );
    }

    #[tokio::test]
    async fn a_stray_frame_is_not_taken_as_the_peer_having_begun_streaming() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let message: Bytes = reply.encode_to_vec().into();
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let request = read_request(&mut framed).await;
            // A frame for the request before this one, which the client discards on its id. The
            // first request id is 1, and the leniency for a delayed response covers its
            // predecessor.
            send_response(&mut framed, RpcResponse {
                request_id: request.request_id - 1,
                status: RpcStatusCode::Ok,
                flags: RpcMessageFlags::empty(),
                payload: Bytes::new(),
            })
            .await;
            time::sleep(Duration::from_secs(1)).await;
            send_message(&mut framed, request.request_id, message).await;
        });

        let config = RpcClientConfig {
            deadline: Some(Duration::from_secs(10)),
            deadline_grace_period: Duration::from_millis(200),
            peer_minimum_keepalive_interval: Duration::from_millis(100),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        // The discarded frame is not a keepalive, so it says nothing about the peer streaming and
        // must not tighten the wait to the 500ms tolerance.
        let received = rpc_client
            .server_streaming_with_options::<_, _, proto::RpcSession>(
                proto::RpcSession::default(),
                1u32,
                RpcRequestOptions::new().with_keepalive_interval(Duration::from_millis(100)),
            )
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;

        server.await.unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].as_ref().unwrap().supported_versions, vec![7]);
    }

    #[tokio::test]
    async fn a_slow_consumer_does_not_spend_the_next_message_s_deadline() {
        const NUM_MESSAGES: usize = 6;
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let reply = proto::RpcSession {
            supported_versions: vec![7],
        };
        let message: Bytes = reply.encode_to_vec().into();
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let request = read_request(&mut framed).await;
            for _ in 0..NUM_MESSAGES {
                send_response(&mut framed, RpcResponse {
                    request_id: request.request_id,
                    status: RpcStatusCode::Ok,
                    flags: RpcMessageFlags::empty(),
                    payload: message.clone(),
                })
                .await;
            }
            // Quiet for longer than the grace period but well inside the deadline, which the peer
            // only starts counting once it has sent the message above.
            time::sleep(Duration::from_millis(1500)).await;
            send_response(&mut framed, RpcResponse {
                request_id: request.request_id,
                status: RpcStatusCode::Ok,
                flags: RpcMessageFlags::FIN,
                payload: Bytes::new(),
            })
            .await;
        });

        let config = RpcClientConfig {
            deadline: Some(Duration::from_secs(1)),
            deadline_grace_period: Duration::from_millis(200),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let mut stream = rpc_client
            .server_streaming::<_, _, proto::RpcSession>(proto::RpcSession::default(), 1u32)
            .await
            .unwrap();

        // Read nothing for long enough to fill the response channel and block the worker mid-send.
        time::sleep(Duration::from_secs(1)).await;
        let mut received = Vec::new();
        while let Some(item) = stream.next().await {
            received.push(item);
        }

        server.await.unwrap();
        assert_eq!(received.len(), NUM_MESSAGES);
        assert!(
            received.iter().all(Result::is_ok),
            "a stalled consumer cost the stream its deadline: {received:?}"
        );
    }

    #[test]
    fn the_keepalive_tolerance_covers_a_peer_serving_its_own_minimum() {
        let config = RpcClientConfig {
            keepalive_interval: Some(Duration::from_secs(1)),
            deadline_grace_period: Duration::from_secs(2),
            ..Default::default()
        };
        // Asking for 1s tolerates a peer that raised it to the 5s default minimum.
        assert_eq!(config.keepalive_timeout(), Some(Duration::from_secs(17)));

        let config = RpcClientConfig {
            keepalive_interval: Some(Duration::from_secs(30)),
            ..config
        };
        assert_eq!(config.keepalive_timeout(), Some(Duration::from_secs(92)));

        assert_eq!(
            RpcClientConfig {
                keepalive_interval: None,
                ..config
            }
            .keepalive_timeout(),
            None
        );
    }

    #[tokio::test]
    async fn a_peer_that_never_answers_a_ping_cannot_hold_it_open() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            read_request(&mut framed).await;
            // Frames the ping must skip, arriving faster than the read timeout they each restart.
            loop {
                send_response(&mut framed, RpcResponse::keepalive(9999)).await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });

        let config = RpcClientConfig {
            deadline: Some(Duration::from_millis(100)),
            deadline_grace_period: Duration::from_millis(100),
            ..Default::default()
        };
        let mut rpc_client = RpcClient::connect(
            config,
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let result = tokio::time::timeout(Duration::from_secs(5), rpc_client.ping())
            .await
            .expect("ping was held open by frames it was skipping");
        assert!(result.is_err());

        server.abort();
    }

    #[tokio::test]
    async fn a_stale_keepalive_is_not_mistaken_for_a_pong() {
        let (server, client) = tokio::io::duplex(RPC_MAX_FRAME_SIZE);
        let server = tokio::spawn(async move {
            let mut framed = framing::canonical(server.compat(), RPC_MAX_FRAME_SIZE);
            let ping = read_request(&mut framed).await;
            send_response(&mut framed, RpcResponse::keepalive(9999)).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            send_response(&mut framed, RpcResponse::keepalive(ping.request_id)).await;
        });

        let mut rpc_client = RpcClient::connect(
            RpcClientConfig::default(),
            PeerId::random(),
            framing::canonical(client.compat(), RPC_MAX_FRAME_SIZE),
            StreamProtocol::new(PROTOCOL),
        )
        .await
        .unwrap();

        let latency = rpc_client.ping().await.unwrap();

        server.await.unwrap();
        assert!(
            latency >= Duration::from_millis(150),
            "ping was answered by the stale keepalive ({:.0?})",
            latency
        );
    }
}
