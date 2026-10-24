//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

/// A minimal stand-in for an indexer's SSE endpoints. Every connection, whatever its path, receives an
/// `initial_event` (if any) with empty JSON data and is then held open. Each accepted connection is reported on the
/// returned channel.
pub(crate) async fn spawn_sse_server(initial_event: Option<&'static str>) -> (String, mpsc::UnboundedReceiver<()>) {
    spawn_sse_server_sending(initial_event.map(|name| format!("event: {name}\ndata: {{}}\n\n"))).await
}

/// As [`spawn_sse_server`], sending `initial_sse` (raw SSE text, if any) on each connection.
pub(crate) async fn spawn_sse_server_sending(initial_sse: Option<String>) -> (String, mpsc::UnboundedReceiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (connected_tx, connected_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut open = Vec::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n")
                .await
                .unwrap();
            if let Some(sse) = &initial_sse {
                socket.write_all(sse.as_bytes()).await.unwrap();
            }
            let _ignore = connected_tx.send(());
            open.push(socket);
        }
    });
    (url, connected_rx)
}
