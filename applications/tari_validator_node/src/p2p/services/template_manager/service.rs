//  Copyright 2022. The Tari Project
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

use std::convert::TryFrom;

use log::*;
use tari_common_types::types::FixedHash;
use tari_core::transactions::transaction_components::TemplateType;
use tari_dan_app_utilities::template_manager::{TemplateManagerError, TemplateManagerRequest, TemplateRegistration};
use tari_dan_common_types::services::template_provider::TemplateProvider;
use tari_dan_engine::function_definitions::FlowFunctionDefinition;
use tari_dan_storage::global::{DbTemplateType, DbTemplateUpdate, TemplateStatus};
use tari_engine_types::calculate_template_binary_hash;
use tari_shutdown::ShutdownSignal;
use tari_template_lib::{models::TemplateAddress, Hash};
use tari_validator_node_client::types::{ArgDef, FunctionDef, TemplateAbi};
use tokio::{
    sync::{mpsc, mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::p2p::services::template_manager::{
    downloader::{DownloadRequest, DownloadResult},
    manager::TemplateManager,
};

const LOG_TARGET: &str = "tari::validator_node::template_manager";

pub struct TemplateManagerService {
    rx_request: Receiver<TemplateManagerRequest>,
    manager: TemplateManager,
    completed_downloads: mpsc::Receiver<DownloadResult>,
    download_queue: mpsc::Sender<DownloadRequest>,
}

impl TemplateManagerService {
    pub fn spawn(
        rx_request: Receiver<TemplateManagerRequest>,
        manager: TemplateManager,
        download_queue: mpsc::Sender<DownloadRequest>,
        completed_downloads: mpsc::Receiver<DownloadResult>,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<anyhow::Result<()>> {
        tokio::spawn(async move {
            Self {
                rx_request,
                manager,
                download_queue,
                completed_downloads,
            }
            .run(shutdown)
            .await?;
            Ok(())
        })
    }

    pub async fn run(mut self, mut shutdown: ShutdownSignal) -> Result<(), TemplateManagerError> {
        self.on_startup().await?;
        loop {
            tokio::select! {
                Some(req) = self.rx_request.recv() => self.handle_request(req).await,
                Some(download) = self.completed_downloads.recv() => {
                    if let Err(err) = self.handle_completed_download(download) {
                        error!(target: LOG_TARGET, "Error handling completed download: {}", err);
                    }
                },

                _ = shutdown.wait() => {
                    dbg!("Shutting down epoch manager");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn on_startup(&mut self) -> Result<(), TemplateManagerError> {
        let templates = self.manager.fetch_pending_templates()?;
        for template in templates {
            if template.status == TemplateStatus::Pending {
                let _ignore = self
                    .download_queue
                    .send(DownloadRequest {
                        template_type: template.template_type.into(),
                        address: Hash::try_from(template.template_address.as_slice()).unwrap(),
                        url: template.url.clone(),
                        expected_binary_hash: template.expected_hash.clone(),
                    })
                    .await;
                info!(
                    target: LOG_TARGET,
                    "⏳️️ Template {} queued for download", template.template_address
                );
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, req: TemplateManagerRequest) {
        #[allow(clippy::enum_glob_use)]
        use TemplateManagerRequest::*;
        match req {
            AddTemplate { template, reply } => {
                handle(reply, self.handle_add_template(*template).await);
            },
            GetTemplate { address, reply } => {
                handle(reply, self.manager.fetch_template(&address));
            },
            GetTemplates { limit, reply } => handle(reply, self.manager.fetch_template_metadata(limit)),
            LoadTemplateAbi { address, reply } => handle(reply, self.handle_load_template_abi(address)),
        }
    }

    fn handle_load_template_abi(&mut self, address: TemplateAddress) -> Result<TemplateAbi, TemplateManagerError> {
        let loaded = self.manager.get_template_module(&address)?;
        Ok(TemplateAbi {
            template_name: loaded.template_def().template_name.clone(),
            functions: loaded
                .template_def()
                .functions
                .iter()
                .map(|f| FunctionDef {
                    name: f.name.clone(),
                    arguments: f
                        .arguments
                        .iter()
                        .map(|a| ArgDef {
                            name: a.name.to_string(),
                            arg_type: a.arg_type.to_string(),
                        })
                        .collect(),
                    output: f.output.to_string(),
                    is_mut: f.is_mut,
                })
                .collect(),
        })
    }

    fn handle_completed_download(&mut self, download: DownloadResult) -> Result<(), TemplateManagerError> {
        match download.result {
            Ok(bytes) => {
                info!(
                    target: LOG_TARGET,
                    "✅ Template {} downloaded successfully", download.template_address
                );

                // validation of the downloaded template binary hash
                let actual_binary_hash = calculate_template_binary_hash(&bytes);
                let template_status = if actual_binary_hash.as_slice() == download.expected_binary_hash.as_slice() {
                    info!(
                        target: LOG_TARGET,
                        "✅ Template {} is active", download.template_address,
                    );
                    TemplateStatus::Active
                } else {
                    warn!(
                        target: LOG_TARGET,
                        "⚠️ Template {} hash mismatch", download.template_address
                    );
                    // TODO: For now, let's just accept this so that we can update the binary at the URL without
                    // re-registering
                    TemplateStatus::Active
                    // TemplateStatus::Invalid
                };

                let update = match download.template_type {
                    DbTemplateType::Wasm => DbTemplateUpdate {
                        compiled_code: Some(bytes.to_vec()),
                        status: Some(template_status),
                        ..Default::default()
                    },
                    DbTemplateType::Flow => {
                        // make sure it deserializes correctly
                        let mut status = TemplateStatus::Invalid;
                        match serde_json::from_slice::<FlowFunctionDefinition>(&bytes) {
                            Ok(_) => status = template_status,
                            Err(e) => {
                                warn!(
                                    target: LOG_TARGET,
                                    "⚠️ Template {} is not valid json: {}", download.template_address, e
                                );
                            },
                        };

                        DbTemplateUpdate {
                            flow_json: Some(String::from_utf8(bytes.to_vec())?),
                            status: Some(status),
                            ..Default::default()
                        }
                    },
                    DbTemplateType::Manifest => todo!(),
                };
                self.manager.update_template(download.template_address, update)?;
            },
            Err(err) => {
                warn!(target: LOG_TARGET, "🚨 Failed to download template: {}", err);
                self.manager
                    .update_template(download.template_address, DbTemplateUpdate {
                        status: Some(TemplateStatus::DownloadFailed),
                        ..Default::default()
                    })?;
            },
        }
        Ok(())
    }

    async fn handle_add_template(&mut self, template: TemplateRegistration) -> Result<(), TemplateManagerError> {
        let address = template.template_address;
        let url = template.registration.binary_url.to_string();
        let template_type = match template.registration.template_type {
            TemplateType::Wasm { .. } => DbTemplateType::Wasm,
            TemplateType::Flow => DbTemplateType::Flow,
            TemplateType::Manifest => DbTemplateType::Manifest,
        };
        let expected_binary_hash = FixedHash::try_from(template.registration.binary_sha.clone().into_vec())
            .map_err(|_| TemplateManagerError::InvalidBaseLayerTemplate)?;
        self.manager.add_template(template)?;
        // We could queue this up much later, at which point we'd update to pending
        self.manager.update_template(address, DbTemplateUpdate {
            status: Some(TemplateStatus::Pending),
            ..Default::default()
        })?;

        let _ignore = self
            .download_queue
            .send(DownloadRequest {
                template_type,
                address,
                url,
                expected_binary_hash,
            })
            .await;
        info!(target: LOG_TARGET, "⏳️️ Template {} queued for download", address);
        Ok(())
    }
}

fn handle<T>(reply: oneshot::Sender<Result<T, TemplateManagerError>>, result: Result<T, TemplateManagerError>) {
    if let Err(ref e) = result {
        error!(target: LOG_TARGET, "Request failed with error: {}", e);
    }
    if reply.send(result).is_err() {
        error!(target: LOG_TARGET, "Requester abandoned request");
    }
}
