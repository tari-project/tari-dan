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
use tari_dan_app_utilities::template_manager::{
    Template, TemplateManagerError, TemplateManagerRequest, TemplateMetadata, TemplateRegistration,
};
use tari_shutdown::ShutdownSignal;
use tari_template_lib::models::TemplateAddress;
use tari_validator_node_client::types::TemplateAbi;
use tokio::{
    sync::{mpsc::Receiver, oneshot},
    task::JoinHandle,
};

const LOG_TARGET: &str = "tari::indexer::template_manager";

pub struct TemplateManagerService {
    rx_request: Receiver<TemplateManagerRequest>,
}

impl TemplateManagerService {
    pub fn spawn(
        rx_request: Receiver<TemplateManagerRequest>,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<anyhow::Result<()>> {
        tokio::spawn(async move {
            Self { rx_request }.run(shutdown).await?;
            Ok(())
        })
    }

    pub async fn run(mut self, mut shutdown: ShutdownSignal) -> Result<(), TemplateManagerError> {
        loop {
            tokio::select! {
                Some(req) = self.rx_request.recv() => self.handle_request(req).await,
                _ = shutdown.wait() => {
                    dbg!("Shutting down template manager");
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
            AddTemplate { template, reply } => {
                handle(reply, self.handle_add_template(*template).await);
            },
            GetTemplate { address, reply } => {
                handle(reply, self.fetch_template(&address));
            },
            GetTemplates { limit, reply } => handle(reply, self.fetch_template_metadata(limit)),
            LoadTemplateAbi { address, reply } => handle(reply, self.handle_load_template_abi(address)),
        }
    }

    async fn handle_add_template(&mut self, _template: TemplateRegistration) -> Result<(), TemplateManagerError> {
        Ok(())
    }

    fn fetch_template(&self, address: &TemplateAddress) -> Result<Template, TemplateManagerError> {
        Err(TemplateManagerError::TemplateNotFound { address: *address })
    }

    fn fetch_template_metadata(&self, _limit: usize) -> Result<Vec<TemplateMetadata>, TemplateManagerError> {
        Ok(vec![])
    }

    fn handle_load_template_abi(&mut self, address: TemplateAddress) -> Result<TemplateAbi, TemplateManagerError> {
        Err(TemplateManagerError::TemplateNotFound { address })
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
