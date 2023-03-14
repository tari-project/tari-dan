//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use bytes::Bytes;
use futures::{future::BoxFuture, stream::FuturesUnordered};
use prost::bytes;
use tari_common_types::types::FixedHash;
use tari_core::transactions::transaction_components::TemplateType;
use tari_template_lib::models::TemplateAddress;
use tokio::{sync::mpsc, task};
use tokio_stream::StreamExt;

pub struct DownloadRequest {
    pub address: TemplateAddress,
    pub template_type: TemplateType,
    pub url: String,
    pub expected_binary_hash: FixedHash,
}

pub(super) struct TemplateDownloadWorker {
    download_queue: mpsc::Receiver<DownloadRequest>,
    pending_downloads: FuturesUnordered<BoxFuture<'static, DownloadResult>>,
    completed_downloads: mpsc::Sender<DownloadResult>,
}

impl TemplateDownloadWorker {
    pub fn new(
        download_queue: mpsc::Receiver<DownloadRequest>,
        completed_downloads: mpsc::Sender<DownloadResult>,
    ) -> Self {
        Self {
            download_queue,
            pending_downloads: FuturesUnordered::new(),
            completed_downloads,
        }
    }

    pub fn spawn(self) {
        task::spawn(self.run());
    }

    pub async fn run(mut self) {
        loop {
            tokio::select! {
                maybe_req = self.download_queue.recv() => {
                    match maybe_req {
                        Some(req)  => {
                            self.pending_downloads.push(Box::pin(download(req)));
                        },
                        None => break,
                    }
                },
                Some(result) = self.pending_downloads.next() => {
                    self.completed_downloads.send(result).await.unwrap();
                }
            }
        }
    }
}

async fn download(req: DownloadRequest) -> DownloadResult {
    async fn inner(req: DownloadRequest) -> Result<Bytes, TemplateDownloadError> {
        let resp = reqwest::get(&req.url).await?;
        let bytes = resp.bytes().await?;
        Ok(bytes)
    }

    DownloadResult {
        template_address: req.address,
        template_type: req.template_type.clone(),
        expected_binary_hash: req.expected_binary_hash,
        result: inner(req).await,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TemplateDownloadError {
    #[error("Failed to download template: {0}")]
    DownloadFailed(#[from] reqwest::Error),
}

#[derive(Debug)]
pub struct DownloadResult {
    pub template_address: TemplateAddress,
    pub template_type: TemplateType,
    pub expected_binary_hash: FixedHash,
    pub result: Result<Bytes, TemplateDownloadError>,
}
