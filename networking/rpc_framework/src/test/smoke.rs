//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! End-to-end tests over a real [`RpcServer`] driven across a duplex stream, one substream per
//! session, in place of a transport.

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use libp2p::{PeerId, StreamProtocol};
use libp2p_substream::{ProtocolEvent, ProtocolNotification};
use prost::Message;
use tari_shutdown::Shutdown;
use tokio::{
    io::DuplexStream,
    sync::{RwLock, mpsc},
    task,
    time,
};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

use crate::{
    Handshake,
    NamedProtocolService,
    RPC_MAX_FRAME_SIZE,
    RpcClient,
    RpcError,
    RpcServer,
    RpcServerBuilder,
    RpcStatus,
    RpcStatusCode,
    error::HandshakeRejectReason,
    framing,
    framing::CanonicalFraming,
    handshake::decode_session_rejection,
    max_response_payload_size,
    proto,
    test::greeting_service::{
        GreetingClient,
        GreetingRpc,
        GreetingServer,
        GreetingService,
        SayHelloRequest,
        SayHelloResponse,
        SlowGreetingService,
        SlowStreamRequest,
    },
};

type TestSubstream = Compat<DuplexStream>;

/// Stands in for a transport's flow-control window: a server streaming to a client that has stopped
/// reading fills this and then blocks, as it would on a real connection.
const TRANSPORT_WINDOW: usize = 64 * 1024;

/// An `RpcServer` fed substreams directly, as the connection manager would feed it substreams from a
/// transport.
struct TestRpcServer {
    notif_tx: mpsc::UnboundedSender<ProtocolNotification<TestSubstream>>,
    shutdown: Shutdown,
    handle: task::JoinHandle<()>,
}

impl TestRpcServer {
    fn spawn<T: GreetingRpc>(service: T, builder: RpcServerBuilder) -> Self {
        let (notif_tx, notif_rx) = mpsc::unbounded_channel();
        let shutdown = Shutdown::new();
        let handle = task::spawn({
            let shutdown_signal = shutdown.to_signal();
            async move {
                let fut = builder
                    .finish()
                    .add_service(GreetingServer::new(service))
                    .serve(notif_rx);

                tokio::select! {
                    biased;
                    _ = shutdown_signal => {},
                    r = fut => r.unwrap(),
                }
            }
        });

        Self {
            notif_tx,
            shutdown,
            handle,
        }
    }

    fn with_sessions<T: GreetingRpc>(service: T, num_sessions: usize) -> Self {
        Self::spawn(
            service,
            RpcServer::builder()
                .with_maximum_simultaneous_sessions(num_sessions)
                .with_minimum_client_deadline(Duration::from_secs(0)),
        )
    }

    /// Hands the server one end of a new substream as an inbound dial from `peer_id`, and returns
    /// the other end framed for a client.
    fn dial_as(&self, peer_id: PeerId, protocol: StreamProtocol, window: usize) -> CanonicalFraming<TestSubstream> {
        let (server_io, client_io) = tokio::io::duplex(window);
        self.notif_tx
            .send(ProtocolNotification::new(
                protocol,
                ProtocolEvent::NewInboundSubstream {
                    peer_id,
                    substream: server_io.compat(),
                },
            ))
            .unwrap();
        framing::canonical(client_io.compat(), RPC_MAX_FRAME_SIZE)
    }

    fn dial(&self) -> CanonicalFraming<TestSubstream> {
        self.dial_with_window(TRANSPORT_WINDOW)
    }

    fn dial_with_window(&self, window: usize) -> CanonicalFraming<TestSubstream> {
        self.dial_as(
            PeerId::random(),
            StreamProtocol::new(GreetingClient::PROTOCOL_NAME),
            window,
        )
    }

    fn dial_peer(&self, peer_id: PeerId) -> CanonicalFraming<TestSubstream> {
        self.dial_as(
            peer_id,
            StreamProtocol::new(GreetingClient::PROTOCOL_NAME),
            TRANSPORT_WINDOW,
        )
    }

    async fn shutdown(self) {
        self.shutdown.trigger();
        self.handle.await.unwrap();
    }
}

/// Connects a client over `framed`, with deadlines wide enough that a slow CI machine does not trip
/// them.
async fn connect(framed: CanonicalFraming<TestSubstream>) -> Result<GreetingClient, RpcError> {
    connect_with_deadline(framed, Duration::from_secs(5)).await
}

async fn connect_with_deadline(
    framed: CanonicalFraming<TestSubstream>,
    deadline: Duration,
) -> Result<GreetingClient, RpcError> {
    RpcClient::builder::<GreetingClient>(PeerId::random())
        .with_deadline(deadline)
        .with_deadline_grace_period(Duration::from_secs(1))
        .with_handshake_timeout(Duration::from_secs(5))
        .connect(framed)
        .await
}

/// The reason the server gave for refusing this session.
///
/// The server refuses without waiting for the client's handshake, so its refusal is the first frame
/// on the substream. How a client surfaces it depends on when it lands relative to the client's own
/// writes, which `a_refusal_in_place_of_the_first_response_is_reported` pins down.
async fn refusal_reason(mut framed: CanonicalFraming<TestSubstream>) -> String {
    let frame = time::timeout(Duration::from_secs(5), framed.next())
        .await
        .expect("the server did not refuse the session in time")
        .expect("the server closed the session without refusing it")
        .unwrap();
    decode_session_rejection(&frame)
        .expect("the server's first frame is not a refusal")
        .to_string()
}

#[tokio::test]
async fn request_response_errors_and_streaming() {
    let server = TestRpcServer::with_sessions(GreetingService::default(), 1);
    let mut client = connect(server.dial()).await.unwrap();

    // Latency is available "for free" as part of the connect protocol
    assert!(client.get_last_request_latency().is_some());

    let resp = client
        .say_hello(SayHelloRequest {
            name: "Yathvan".to_string(),
            language: 1,
        })
        .await
        .unwrap();
    assert_eq!(resp.greeting, "Jambo Yathvan");

    let resp = client.get_greetings(4).await.unwrap();
    let greetings = resp.map(|r| r.unwrap()).collect::<Vec<_>>().await;
    assert_eq!(greetings, ["Sawubona", "Jambo", "Bonjour", "Hello"]);

    let err = client.return_error().await.unwrap_err();
    let RpcError::RequestFailed(status) = err else {
        panic!("expected a request failure, got {err:?}");
    };
    assert_eq!(status.as_status_code(), RpcStatusCode::NotImplemented);
    assert_eq!(status.details(), "I haven't gotten to this yet :(");

    let stream = client.streaming_error("Gurglesplurb".to_string()).await.unwrap();
    let status = stream
        // StreamExt::collect has a Default trait bound which Result<_, _> cannot satisfy, so the
        // results are collected into a Vec first.
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<String, _>>()
        .unwrap_err();
    assert_eq!(status.as_status_code(), RpcStatusCode::BadRequest);
    assert_eq!(status.details(), "What does 'Gurglesplurb' mean?");

    let stream = client.streaming_error2().await.unwrap();
    let results = stream.collect::<Vec<_>>().await;
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].as_ref().unwrap(), "This is ok");

    let second_reply = results[1].as_ref().unwrap_err();
    assert_eq!(second_reply.as_status_code(), RpcStatusCode::BadRequest);
    assert_eq!(second_reply.details(), "This is a problem");

    client.close().await;

    let err = client.say_hello(SayHelloRequest::default()).await.unwrap_err();
    assert!(
        // Closing the request stream races the send on it, so either answer is correct.
        matches!(err, RpcError::ClientClosed | RpcError::RequestCancelled),
        "unexpected error {err:?}"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn concurrent_requests() {
    let server = TestRpcServer::with_sessions(GreetingService::default(), 1);
    let mut client = connect(server.dial()).await.unwrap();

    let mut cloned_client = client.clone();
    let spawned1 = task::spawn(async move {
        cloned_client
            .say_hello(SayHelloRequest {
                name: "Madeupington".to_string(),
                language: 2,
            })
            .await
            .unwrap()
    });
    let mut cloned_client = client.clone();
    let spawned2 = task::spawn(async move {
        let resp = cloned_client.get_greetings(5).await.unwrap().collect::<Vec<_>>().await;
        resp.into_iter().map(Result::unwrap).collect::<Vec<_>>()
    });
    let resp = client
        .say_hello(SayHelloRequest {
            name: "Yathvan".to_string(),
            language: 1,
        })
        .await
        .unwrap();
    assert_eq!(resp.greeting, "Jambo Yathvan");

    assert_eq!(spawned1.await.unwrap().greeting, "Bonjour Madeupington");
    assert_eq!(spawned2.await.unwrap(), GreetingService::DEFAULT_GREETINGS[..5]);
}

#[tokio::test]
async fn response_too_big() {
    let server = TestRpcServer::with_sessions(GreetingService::new(&[]), 1);
    let mut client = connect(server.dial_with_window(RPC_MAX_FRAME_SIZE)).await.unwrap();

    // The response overhead means a payload of exactly the frame size is always too large.
    let err = client
        .reply_with_msg_of_size(max_response_payload_size() as u64 + 1)
        .await
        .unwrap_err();
    let RpcError::RequestFailed(status) = err else {
        panic!("expected a request failure, got {err:?}");
    };
    assert_eq!(status.as_status_code(), RpcStatusCode::MalformedResponse);

    // The exact frame size boundary works, and the session survives the rejection above.
    let reply = client
        .reply_with_msg_of_size(max_response_payload_size() as u64 - 9)
        .await
        .unwrap();
    assert_eq!(reply.len(), max_response_payload_size() - 9);
}

#[tokio::test]
async fn ping_latency() {
    let server = TestRpcServer::with_sessions(GreetingService::new(&[]), 1);
    let mut client = connect(server.dial()).await.unwrap();

    let latency = client.ping().await.unwrap();
    // Typically well under a millisecond over a duplex stream; the bound is wide for slow CI.
    assert!(latency.as_secs() < 5);
}

#[tokio::test]
async fn timeout() {
    let delay = Arc::new(RwLock::new(Duration::from_secs(10)));
    let server = TestRpcServer::with_sessions(SlowGreetingService::new(delay.clone()), 1);
    let mut client = connect_with_deadline(server.dial(), Duration::from_secs(1))
        .await
        .unwrap();

    let err = client.say_hello(SayHelloRequest::default()).await.unwrap_err();
    let RpcError::RequestFailed(status) = err else {
        panic!("expected a request failure, got {err:?}");
    };
    assert_eq!(status.as_status_code(), RpcStatusCode::Timeout);

    *delay.write().await = Duration::from_secs(0);

    // The server abandons the request at the deadline and waits for the next one rather than
    // ending the session, so the next request is answered normally.
    let resp = client.say_hello(SayHelloRequest::default()).await.unwrap();
    assert_eq!(resp.greeting, "took a while to load");
}

#[tokio::test]
async fn stream_still_works_after_cancel() {
    let service_impl = GreetingService::default();
    let server = TestRpcServer::with_sessions(service_impl.clone(), 1);
    let mut client = connect(server.dial()).await.unwrap();

    // Ask for a stream and immediately throw away the receiver.
    client
        .slow_stream(SlowStreamRequest {
            num_items: 100,
            item_size: 100,
            delay_ms: 10,
        })
        .await
        .unwrap();
    assert_eq!(service_impl.call_count(), 1);

    let resp = client
        .slow_stream(SlowStreamRequest {
            num_items: 100,
            item_size: 100,
            delay_ms: 10,
        })
        .await
        .unwrap();

    resp.collect::<Vec<_>>().await.into_iter().for_each(|r| {
        r.unwrap();
    });
}

#[tokio::test]
async fn a_session_recovers_from_an_abandoned_stream() {
    let service_impl = GreetingService::default();
    let server = TestRpcServer::with_sessions(service_impl.clone(), 1);
    let mut client = connect(server.dial()).await.unwrap();

    // Abandoned after one item, leaving the rest of the stream to arrive against a request that is
    // no longer waiting for it.
    let mut resp = client
        .slow_stream(SlowStreamRequest {
            num_items: 50,
            item_size: 100,
            delay_ms: 1,
        })
        .await
        .unwrap();
    let _buffer = resp.next().await.unwrap().unwrap();
    drop(resp);

    // The next request has to read past every frame the abandoned one left behind before its own
    // response, and there are more of them than a session may discard by any fixed count.
    let resp = client.get_greetings(4).await.unwrap();
    let greetings = resp.map(|r| r.unwrap()).collect::<Vec<_>>().await;
    assert_eq!(greetings, ["Sawubona", "Jambo", "Bonjour", "Hello"]);

    // And the session is still good for another after that.
    let resp = client
        .say_hello(SayHelloRequest {
            name: "Yathvan".to_string(),
            language: 1,
        })
        .await
        .unwrap();
    assert_eq!(resp.greeting, "Jambo Yathvan");
}

/// A stream that never ends on its own, as a followed stream does not.
fn endless_stream() -> SlowStreamRequest {
    SlowStreamRequest {
        num_items: u32::MAX,
        item_size: 10,
        delay_ms: 10,
    }
}

async fn until_no_stream_is_live(service: &GreetingService) {
    time::timeout(Duration::from_secs(5), async {
        while service.live_streams() > 0 {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the server went on producing a stream nobody was reading");
}

#[tokio::test]
async fn abandoning_a_stream_stops_the_server_producing_it() {
    let service_impl = GreetingService::default();
    let server = TestRpcServer::with_sessions(service_impl.clone(), 1);
    let mut client = connect(server.dial()).await.unwrap();

    let mut resp = client.slow_stream(endless_stream()).await.unwrap();
    resp.next().await.unwrap().unwrap();
    assert_eq!(service_impl.live_streams(), 1);
    drop(resp);

    // Nothing else is asked of the session: the abandonment alone has to reach the server.
    until_no_stream_is_live(&service_impl).await;
}

#[tokio::test]
async fn a_request_after_an_abandoned_endless_stream_is_answered() {
    let service_impl = GreetingService::default();
    let server = TestRpcServer::with_sessions(service_impl.clone(), 1);
    let mut client = connect(server.dial()).await.unwrap();

    let mut resp = client.slow_stream(endless_stream()).await.unwrap();
    resp.next().await.unwrap().unwrap();
    drop(resp);

    let resp = client
        .say_hello(SayHelloRequest {
            name: "Yathvan".to_string(),
            language: 1,
        })
        .await
        .unwrap();
    assert_eq!(resp.greeting, "Jambo Yathvan");
}

const SAY_HELLO_METHOD: u32 = 1;
const SLOW_STREAM_METHOD: u32 = 7;

fn raw_request(request_id: u32, method: u32, payload: Vec<u8>) -> Bytes {
    proto::RpcRequest {
        request_id,
        method,
        deadline: 5,
        payload,
        ..Default::default()
    }
    .encode_to_vec()
    .into()
}

/// Reads past whatever is left of earlier requests to the response to `request_id`.
async fn raw_response(framed: &mut CanonicalFraming<TestSubstream>, request_id: u32) -> proto::RpcResponse {
    time::timeout(Duration::from_secs(5), async {
        loop {
            let resp = proto::RpcResponse::decode(framed.next().await.unwrap().unwrap()).unwrap();
            if resp.request_id == request_id {
                break resp;
            }
        }
    })
    .await
    .expect("the server did not answer a request sent while it was streaming")
}

/// Starts an endless stream on a session opened with no client of the framework's own, which sends only what each
/// test makes it send.
async fn raw_session_streaming(server: &TestRpcServer, window: usize) -> CanonicalFraming<TestSubstream> {
    let mut framed = server.dial_with_window(window);
    Handshake::new(&mut framed).perform_client_handshake().await.unwrap();
    framed
        .send(raw_request(1, SLOW_STREAM_METHOD, endless_stream().encode_to_vec()))
        .await
        .unwrap();
    framed
}

fn assert_greeted(resp: &proto::RpcResponse) {
    assert_eq!(resp.status, RpcStatus::ok().as_code());
    assert_eq!(
        SayHelloResponse::decode(resp.payload.as_slice()).unwrap().greeting,
        "Jambo Yathvan"
    );
}

fn hello() -> Vec<u8> {
    SayHelloRequest {
        name: "Yathvan".to_string(),
        language: 1,
    }
    .encode_to_vec()
}

#[tokio::test]
async fn a_request_sent_while_a_stream_runs_ends_the_stream_and_is_answered() {
    let service_impl = GreetingService::default();
    let server = TestRpcServer::with_sessions(service_impl.clone(), 1);
    let mut framed = raw_session_streaming(&server, TRANSPORT_WINDOW).await;
    let first = proto::RpcResponse::decode(framed.next().await.unwrap().unwrap()).unwrap();
    assert_eq!(first.request_id, 1);

    // The next request goes out with the stream still running and no FIN sent for it.
    framed.send(raw_request(2, SAY_HELLO_METHOD, hello())).await.unwrap();

    assert_greeted(&raw_response(&mut framed, 2).await);
    until_no_stream_is_live(&service_impl).await;
}

#[tokio::test]
async fn a_request_sent_while_the_server_is_held_up_writing_a_stream_is_answered() {
    let service_impl = GreetingService::default();
    let server = TestRpcServer::with_sessions(service_impl.clone(), 1);
    // A window smaller than one frame of the stream, read by nobody, holds the server up mid-write.
    let mut framed = raw_session_streaming(&server, 4).await;
    time::sleep(Duration::from_millis(100)).await;

    framed.send(raw_request(2, SAY_HELLO_METHOD, hello())).await.unwrap();

    assert_greeted(&raw_response(&mut framed, 2).await);
    until_no_stream_is_live(&service_impl).await;
}

#[tokio::test]
async fn a_session_beyond_the_server_s_limit_is_refused() {
    // A server with no session slots refuses every session it is offered.
    let server = TestRpcServer::with_sessions(GreetingService::new(&[]), 0);
    assert_eq!(
        refusal_reason(server.dial()).await,
        HandshakeRejectReason::NoSessionsAvailable.to_string()
    );
}

#[tokio::test]
async fn a_refusal_in_place_of_the_first_response_is_reported() {
    let (server_io, client_io) = tokio::io::duplex(TRANSPORT_WINDOW);
    let mut server_framed = framing::canonical(server_io.compat(), RPC_MAX_FRAME_SIZE);
    let mut client = connect(framing::canonical(client_io.compat(), RPC_MAX_FRAME_SIZE))
        .await
        .unwrap();

    // The refusal goes out only once the request has been read, so it arrives where the client is
    // waiting for the response.
    let server = task::spawn(async move {
        let _handshake = server_framed.next().await.unwrap().unwrap();
        let _request = server_framed.next().await.unwrap().unwrap();
        Handshake::new(&mut server_framed)
            .reject_with_reason(HandshakeRejectReason::NoSessionsAvailable)
            .await
            .unwrap();
    });

    let err = client.say_hello(SayHelloRequest::default()).await.unwrap_err();
    let RpcError::RequestFailed(status) = err else {
        panic!("expected the session to be refused, got {err:?}");
    };
    assert_eq!(status.as_status_code(), RpcStatusCode::HandshakeDenied);
    assert_eq!(status.details(), HandshakeRejectReason::NoSessionsAvailable.to_string());
    server.await.unwrap();
}

#[tokio::test]
async fn a_session_for_an_unknown_protocol_is_refused() {
    let server = TestRpcServer::with_sessions(GreetingService::new(&[]), 1);
    let framed = server.dial_as(
        PeerId::random(),
        StreamProtocol::new("/test/this-is-junk/1.0"),
        TRANSPORT_WINDOW,
    );
    assert_eq!(
        refusal_reason(framed).await,
        HandshakeRejectReason::ProtocolNotSupported.to_string()
    );
}

#[tokio::test]
async fn max_global_sessions() {
    let server = TestRpcServer::spawn(
        GreetingService::default(),
        RpcServer::builder().with_maximum_simultaneous_sessions(1),
    );

    let mut first = connect(server.dial()).await.unwrap();
    first.say_hello(SayHelloRequest::default()).await.unwrap();

    assert_eq!(
        refusal_reason(server.dial()).await,
        HandshakeRejectReason::NoSessionsAvailable.to_string()
    );

    // Closing the first session frees its slot.
    first.close().await;
    wait_for_session(&server).await;
}

#[tokio::test]
async fn an_idle_session_is_closed_and_its_slot_released() {
    let idle_timeout = Duration::from_secs(2);
    let server = TestRpcServer::spawn(
        GreetingService::default(),
        RpcServer::builder()
            .with_maximum_simultaneous_sessions(2)
            .with_idle_session_timeout(idle_timeout),
    );

    // Completed handshakes that then send nothing, holding every slot.
    let mut idle = Vec::new();
    for _ in 0..2 {
        let mut client = connect(server.dial()).await.unwrap();
        client.say_hello(SayHelloRequest::default()).await.unwrap();
        idle.push(client);
    }

    assert_eq!(
        refusal_reason(server.dial()).await,
        HandshakeRejectReason::NoSessionsAvailable.to_string()
    );

    time::sleep(idle_timeout).await;
    wait_for_session(&server).await;
}

#[tokio::test]
async fn max_per_client_sessions() {
    let server = TestRpcServer::spawn(
        GreetingService::default(),
        RpcServer::builder()
            .with_maximum_simultaneous_sessions(3)
            .with_maximum_sessions_per_client(1),
    );
    let peer_id = PeerId::random();

    let mut first = connect(server.dial_peer(peer_id)).await.unwrap();
    first.say_hello(SayHelloRequest::default()).await.unwrap();

    assert_eq!(
        refusal_reason(server.dial_peer(peer_id)).await,
        HandshakeRejectReason::NoSessionsAvailable.to_string()
    );

    // Another peer is unaffected by the per-client limit.
    let mut other = connect(server.dial_peer(PeerId::random())).await.unwrap();
    other.say_hello(SayHelloRequest::default()).await.unwrap();
}

#[tokio::test]
async fn failed_handshakes_do_not_count_against_the_per_client_limit() {
    let server = TestRpcServer::spawn(
        GreetingService::default(),
        RpcServer::builder().with_maximum_sessions_per_client(2),
    );
    let peer_id = PeerId::random();

    // Each substream closes before the client sends its handshake.
    for _ in 0..3 {
        drop(server.dial_peer(peer_id));
    }

    wait_for_session_from(&server, peer_id).await;
}

#[tokio::test]
async fn a_substream_that_never_sends_its_handshake_does_not_hold_up_other_sessions() {
    let server = TestRpcServer::spawn(GreetingService::default(), RpcServer::builder());

    // Held open for the whole test without a handshake ever being written to them.
    let _silent = (0..3).map(|_| server.dial()).collect::<Vec<_>>();

    let admitted = time::timeout(Duration::from_secs(2), async {
        let mut client = connect(server.dial()).await.unwrap();
        client.say_hello(SayHelloRequest::default()).await.unwrap();
    })
    .await;
    assert!(
        admitted.is_ok(),
        "a session behind substreams awaiting their handshake was not served"
    );
}

#[tokio::test]
async fn a_substream_that_never_sends_its_handshake_releases_its_slot_at_the_handshake_timeout() {
    let handshake_timeout = Duration::from_millis(500);
    let server = TestRpcServer::spawn(
        GreetingService::default(),
        RpcServer::builder()
            .with_maximum_simultaneous_sessions(1)
            .with_handshake_timeout(handshake_timeout),
    );

    let _silent = server.dial();
    assert_eq!(
        refusal_reason(server.dial()).await,
        HandshakeRejectReason::NoSessionsAvailable.to_string()
    );

    time::sleep(handshake_timeout).await;
    wait_for_session(&server).await;
}

async fn wait_for_session(server: &TestRpcServer) -> GreetingClient {
    wait_for_session_from(server, PeerId::random()).await
}

/// Opens sessions from `peer_id` until one is accepted. A slot is released when the server's task
/// for that session finishes, which trails the client closing its end.
async fn wait_for_session_from(server: &TestRpcServer, peer_id: PeerId) -> GreetingClient {
    for _ in 0..50 {
        if let Ok(mut client) = connect(server.dial_peer(peer_id)).await &&
            client.say_hello(SayHelloRequest::default()).await.is_ok()
        {
            return client;
        }
        time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the server never released the closed session's slot");
}
