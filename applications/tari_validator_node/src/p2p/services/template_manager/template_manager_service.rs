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

use log::*;
use tari_core::transactions::transaction_components::CodeTemplateRegistration;
use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_shutdown::ShutdownSignal;
use tari_template_lib::models::TemplateAddress;
use tokio::{
    sync::{mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::p2p::services::template_manager::{
    manager::{Template, TemplateManager},
    TemplateManagerError,
};

const LOG_TARGET: &str = "tari::validator_node::template_manager";

pub struct TemplateManagerService {
    rx_request: Receiver<TemplateManagerRequest>,
    inner: TemplateManager,
}

#[derive(Debug)]
pub enum TemplateManagerRequest {
    AddTemplates {
        templates: Vec<CodeTemplateRegistration>,
        reply: oneshot::Sender<Result<(), TemplateManagerError>>,
    },
    GetTemplate {
        address: TemplateAddress,
        reply: oneshot::Sender<Result<Template, TemplateManagerError>>,
    },
}

impl TemplateManagerService {
    pub fn spawn(
        rx_request: Receiver<TemplateManagerRequest>,
        sqlite_db_factory: SqliteDbFactory,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<Result<(), TemplateManagerError>> {
        tokio::spawn(async move {
            TemplateManagerService {
                rx_request,
                inner: TemplateManager::new(sqlite_db_factory),
            }
            .run(shutdown)
            .await
        })
    }

    pub async fn run(&mut self, mut shutdown: ShutdownSignal) -> Result<(), TemplateManagerError> {
        loop {
            tokio::select! {
                Some(req) = self.rx_request.recv() => self.handle_request(req).await,

                _ = shutdown.wait() => {
                    dbg!("Shutting down epoch manager");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, req: TemplateManagerRequest) {
        #[allow(clippy::enum_glob_use)]
        use TemplateManagerRequest::*;
        match req {
            AddTemplates { templates, reply } => {
                handle(reply, self.inner.add_templates(templates).await);
            },
            GetTemplate { address, reply } => {
                handle(reply, self.inner.fetch_template(&address));
            },
        }
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
