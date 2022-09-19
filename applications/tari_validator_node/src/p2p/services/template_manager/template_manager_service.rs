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
use tari_dan_storage_sqlite::SqliteDbFactory;
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::p2p::services::template_manager::{
    template_manager::{TemplateManager, TemplateMetadata},
    TemplateManagerError,
};
const LOG_TARGET: &str = "tari::validator_node::template_manager";

pub struct TemplateManagerService {
    rx_request: Receiver<(
        TemplateManagerRequest,
        oneshot::Sender<Result<TemplateManagerResponse, TemplateManagerError>>,
    )>,
    inner: TemplateManager,
}

#[derive(Debug, Clone)]
pub enum TemplateManagerRequest {
    AddTemplates { templates: Vec<TemplateMetadata> },
}

pub enum TemplateManagerResponse {
    AddTemplates,
}

impl TemplateManagerService {
    pub fn spawn(
        rx_request: Receiver<(
            TemplateManagerRequest,
            oneshot::Sender<Result<TemplateManagerResponse, TemplateManagerError>>,
        )>,
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
                Some((req, reply)) = self.rx_request.recv() => {
                    let _ignore = reply.send(self.handle_request(req).await.map_err(|e|
                    {error!(target: LOG_TARGET, "Error handling request:  {}", &e); e})).map_err(|_|
                        error!(target: LOG_TARGET, "Error sending response on template manager")
                        );

                },
                _ = shutdown.wait() => {
                    dbg!("Shutting down epoch manager");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_request(
        &mut self,
        req: TemplateManagerRequest,
    ) -> Result<TemplateManagerResponse, TemplateManagerError> {
        match req {
            TemplateManagerRequest::AddTemplates { templates } => {
                self.inner.add_templates(templates).await?;

                Ok(TemplateManagerResponse::AddTemplates)
            },
        }
    }
}
