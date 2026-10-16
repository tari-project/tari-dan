//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The method table the generated `ValidatorNodeRpcServer` serves, exercised over a real RPC session.

use futures::StreamExt;
use libp2p::{PeerId, StreamProtocol};
use libp2p_substream::{ProtocolEvent, ProtocolNotification};
use tari_ootle_p2p::proto::rpc as proto;
use tari_rpc_framework::{
    NamedProtocolService,
    RPC_MAX_FRAME_SIZE,
    Request,
    Response,
    RpcClient,
    RpcClientConfig,
    RpcError,
    RpcServer,
    RpcStatus,
    RpcStatusCode,
    Streaming,
    framing,
};
use tari_validator_node_rpc::rpc_service::{ValidatorNodeRpcServer, ValidatorNodeRpcService};
use tokio::{sync::mpsc, task};
use tokio_util::compat::TokioAsyncReadCompatExt;

/// Answers every method with a General error, so an UnsupportedMethod status can only come from the generated
/// dispatcher.
struct UnservedService;

fn unserved<T>() -> Result<T, RpcStatus> {
    Err(RpcStatus::general("not served by this test service"))
}

#[tari_rpc_framework::async_trait]
impl ValidatorNodeRpcService for UnservedService {
    async fn submit_transaction(
        &self,
        _: Request<proto::SubmitTransactionRequest>,
    ) -> Result<Response<proto::SubmitTransactionResponse>, RpcStatus> {
        unserved()
    }

    async fn get_substate(
        &self,
        _: Request<proto::GetSubstateRequest>,
    ) -> Result<Response<proto::GetSubstateResponse>, RpcStatus> {
        unserved()
    }

    async fn get_transaction_result(
        &self,
        _: Request<proto::GetTransactionResultRequest>,
    ) -> Result<Response<proto::GetTransactionResultResponse>, RpcStatus> {
        unserved()
    }

    async fn get_checkpoints(
        &self,
        _: Request<proto::GetCheckpointsRequest>,
    ) -> Result<Response<proto::GetCheckpointsResponse>, RpcStatus> {
        unserved()
    }

    async fn sync_state(
        &self,
        _: Request<proto::SyncStateRequest>,
    ) -> Result<Streaming<proto::SyncStateResponse>, RpcStatus> {
        unserved()
    }

    async fn get_substate_batch(
        &self,
        _: Request<proto::GetSubstatesBatchRequest>,
    ) -> Result<Streaming<proto::GetSubstatesBatchResponse>, RpcStatus> {
        unserved()
    }

    async fn get_consensus_state(
        &self,
        _: Request<proto::GetConsensusStateRequest>,
    ) -> Result<Response<proto::GetConsensusStateResponse>, RpcStatus> {
        unserved()
    }

    async fn get_high_qc(
        &self,
        _: Request<proto::GetHighQcRequest>,
    ) -> Result<Response<proto::GetHighQcResponse>, RpcStatus> {
        unserved()
    }

    async fn get_committed_block_proof(
        &self,
        _: Request<proto::GetCommittedBlockProofRequest>,
    ) -> Result<Response<proto::GetCommittedBlockProofResponse>, RpcStatus> {
        unserved()
    }
}

/// Serves `UnservedService` and returns a client session to it.
async fn connect() -> (RpcClient, task::JoinHandle<()>) {
    type Server = ValidatorNodeRpcServer<UnservedService>;

    let (notif_tx, notif_rx) = mpsc::unbounded_channel();
    let server = task::spawn(async move {
        RpcServer::builder()
            .finish()
            .add_service(Server::new(UnservedService))
            .serve(notif_rx)
            .await
            .unwrap();
    });

    let protocol = StreamProtocol::new(Server::PROTOCOL_NAME);
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    notif_tx
        .send(ProtocolNotification::new(
            protocol.clone(),
            ProtocolEvent::NewInboundSubstream {
                peer_id: PeerId::random(),
                substream: server_io.compat(),
            },
        ))
        .unwrap();
    let framed = framing::canonical(client_io.compat(), RPC_MAX_FRAME_SIZE);
    let client = RpcClient::connect(RpcClientConfig::default(), PeerId::random(), framed, protocol)
        .await
        .unwrap();
    (client, server)
}

/// Method 4 is retired: a peer that calls it gets an unsupported-method status.
#[tokio::test]
async fn method_4_is_not_served() {
    let (mut client, server) = connect().await;

    let result = client
        .server_streaming::<_, _, proto::SyncStateResponse>(proto::SyncStateRequest::default(), 4u32)
        .await;
    let status = match result {
        Ok(mut stream) => stream
            .next()
            .await
            .expect("the server closed the stream without a status")
            .expect_err("method 4 must not stream a response"),
        Err(RpcError::RequestFailed(status)) => status,
        Err(err) => panic!("expected the server to refuse the request, got {err}"),
    };
    assert_eq!(status.as_status_code(), RpcStatusCode::UnsupportedMethod, "{status}");

    server.abort();
}
