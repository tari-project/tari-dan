//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, net::SocketAddr};

use httpmock::MockServer;
use tari_shutdown::ShutdownSignal;
use tokio::task;

use crate::utils::helpers::get_os_assigned_port;

pub struct MockHttpServer {
    server: MockServer,
    base_url: SocketAddr,
}

impl MockHttpServer {
    pub async fn connect(port: u16) -> Self {
        let base_url = format!("127.0.0.1:{}", port);
        Self {
            base_url: base_url.parse().unwrap(),
            server: MockServer::connect_async(&base_url).await,
        }
    }

    pub fn base_url(&self) -> &SocketAddr {
        &self.base_url
    }

    pub async fn publish_file(&self, url_path: String, file_path: String) -> Mock<'_> {
        let mock = self
            .server
            .mock_async(|when, then| {
                when.path(format!("/{}", url_path));
                then.status(200).body_from_file(file_path);
            })
            .await;

        let url = format!("{}/{}", self.server.base_url(), url_path);
        Mock { mock, url }
    }
}

impl fmt::Debug for MockHttpServer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MockHttpServer: {}", self.base_url)
    }
}

pub struct Mock<'a> {
    pub mock: httpmock::Mock<'a>,
    pub url: String,
}

// impl Drop for Mock<'_> {
//     fn drop(&mut self) {
//         self.mock.delete();
//     }
// }

pub async fn spawn_template_http_server(signal: ShutdownSignal) -> u16 {
    let mock_port = get_os_assigned_port();
    task::spawn(async move {
        httpmock::standalone::start_standalone_server(mock_port, false, None, false, 5, signal)
            .await
            .unwrap();
    });

    println!("Mock server started at http://localhost:{}/", mock_port);
    mock_port
}
