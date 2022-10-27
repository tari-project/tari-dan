use std::{convert::TryInto, fmt};

use httpmock::prelude::*;
use tokio::task::{self};

pub struct MockHttpServer {
    server: MockServer,
    pub base_url: String,
}

impl MockHttpServer {
    pub async fn new(port: u64) -> Self {
        let _handle = task::spawn(async move {
            httpmock::standalone::start_standalone_server(port.try_into().unwrap(), false, None, false, 0)
                .await
                .unwrap();
        });
        let base_url = format!("localhost:{}", port);
        Self {
            base_url: base_url.clone(),
            server: MockServer::connect(&base_url),
        }
    }

    pub fn publish_file(&self, url_path: String, file_path: String) -> String {
        let _mock = self.server.mock(|when, then| {
            when.path(format!("/{}", url_path));
            then.status(200).body_from_file(file_path);
        });

        format!("{}/{}", self.server.base_url(), url_path)
    }
}

impl fmt::Debug for MockHttpServer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MockHttpServer: {}", self.base_url)
    }
}
