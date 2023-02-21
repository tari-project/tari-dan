//  Copyright 2023. The Tari Project
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

use tari_common_types::types::FixedHash;
use tari_core::transactions::transaction_components::CodeTemplateRegistration;
use tari_template_lib::models::TemplateAddress;
use tari_validator_node_client::types::TemplateAbi;
use tokio::sync::{mpsc, oneshot};

use super::{types::TemplateManagerRequest, Template, TemplateManagerError, TemplateMetadata};

#[derive(Debug, Clone)]
pub struct TemplateManagerHandle {
    request_tx: mpsc::Sender<TemplateManagerRequest>,
}

impl TemplateManagerHandle {
    pub fn new(request_tx: mpsc::Sender<TemplateManagerRequest>) -> Self {
        Self { request_tx }
    }

    pub async fn add_template(&self, template: TemplateRegistration) -> Result<(), TemplateManagerError> {
        let (tx, rx) = oneshot::channel();
        self.request_tx
            .send(TemplateManagerRequest::AddTemplate {
                template: Box::new(template),
                reply: tx,
            })
            .await
            .map_err(|_| TemplateManagerError::SendError)?;
        rx.await.map_err(|_| TemplateManagerError::SendError)?
    }

    pub async fn get_template(&self, address: TemplateAddress) -> Result<Template, TemplateManagerError> {
        let (tx, rx) = oneshot::channel();
        self.request_tx
            .send(TemplateManagerRequest::GetTemplate { address, reply: tx })
            .await
            .map_err(|_| TemplateManagerError::SendError)?;
        rx.await.map_err(|_| TemplateManagerError::SendError)?
    }

    pub async fn load_template_abi(&self, address: TemplateAddress) -> Result<TemplateAbi, TemplateManagerError> {
        let (tx, rx) = oneshot::channel();
        self.request_tx
            .send(TemplateManagerRequest::LoadTemplateAbi { address, reply: tx })
            .await
            .map_err(|_| TemplateManagerError::SendError)?;
        rx.await.map_err(|_| TemplateManagerError::SendError)?
    }

    pub async fn get_templates(&self, limit: usize) -> Result<Vec<TemplateMetadata>, TemplateManagerError> {
        let (tx, rx) = oneshot::channel();
        self.request_tx
            .send(TemplateManagerRequest::GetTemplates { limit, reply: tx })
            .await
            .map_err(|_| TemplateManagerError::SendError)?;
        rx.await.map_err(|_| TemplateManagerError::SendError)?
    }
}

#[derive(Debug, Clone)]
pub struct TemplateRegistration {
    pub template_name: String,
    pub template_address: TemplateAddress,
    pub registration: CodeTemplateRegistration,
    pub mined_height: u64,
    pub mined_hash: FixedHash,
}
