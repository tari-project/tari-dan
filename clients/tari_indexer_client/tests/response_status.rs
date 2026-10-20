//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

use tari_indexer_client::{
    error::IndexerRestClientError,
    rest_api_client::IndexerRestApiClient,
    types::GetUtxoUpdatesRequest,
};
use tari_ootle_common_types::{Epoch, NumPreshards, StateVersion};

/// Serves one request on a local port, answering it with `status_line` and an empty body.
fn serve_status_once(status_line: &'static str) -> IndexerRestApiClient {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let Ok((mut sock, _)) = listener.accept() else {
            return;
        };
        sock.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        // Every request in these tests is small enough to arrive in a single read.
        let read = sock.read(&mut [0u8; 8192]).unwrap();
        assert_ne!(read, 0, "client closed the connection without sending a request");
        let response = format!("HTTP/1.1 {status_line}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        sock.write_all(response.as_bytes()).unwrap();
        sock.flush().unwrap();
    });
    IndexerRestApiClient::connect(format!("http://127.0.0.1:{port}")).unwrap()
}

fn utxo_updates_request() -> GetUtxoUpdatesRequest {
    GetUtxoUpdatesRequest {
        from_epoch: Epoch::zero(),
        shard_state_versions: NumPreshards::current()
            .all_shards_iter()
            .map(|shard| (shard, StateVersion::zero()))
            .collect(),
        resource_address: "resource_0101010101010101010101010101010101010101010101010101010101010101"
            .parse()
            .unwrap(),
        unspent_only: false,
        per_shard_limit: 1000,
    }
}

async fn utxo_stream_error(status_line: &'static str) -> IndexerRestClientError {
    let client = serve_status_once(status_line);
    match client.stream_utxo_updates_protobuf(utxo_updates_request()).await {
        Ok(_) => panic!("{status_line} opened a UTXO stream"),
        Err(err) => err,
    }
}

#[tokio::test]
async fn a_redirection_status_does_not_open_the_utxo_stream() {
    for status_line in ["300 Multiple Choices", "304 Not Modified"] {
        let err = utxo_stream_error(status_line).await;
        assert!(
            matches!(err, IndexerRestClientError::UnexpectedStatus { status, .. } if status.is_redirection()),
            "{status_line}: {err}"
        );
    }
}

#[tokio::test]
async fn a_server_error_does_not_open_the_utxo_stream() {
    let err = utxo_stream_error("500 Internal Server Error").await;
    assert!(
        matches!(&err, IndexerRestClientError::ErrorResponse { source, .. } if source.status().is_some_and(|s| s.as_u16() == 500)),
        "{err}"
    );
}

#[tokio::test]
async fn a_redirection_status_is_not_parsed_as_a_json_response() {
    let client = serve_status_once("304 Not Modified");
    let err = client.get_network_info().await.unwrap_err();
    assert!(
        matches!(err, IndexerRestClientError::UnexpectedStatus { status, .. } if status.as_u16() == 304),
        "{err}"
    );
}

#[tokio::test]
async fn a_redirection_status_does_not_open_an_event_stream() {
    let client = serve_status_once("304 Not Modified");
    let Err(err) = client.sse_events().await else {
        panic!("304 opened an event stream");
    };
    assert!(
        matches!(err, IndexerRestClientError::UnexpectedStatus { status, .. } if status.as_u16() == 304),
        "{err}"
    );
}
