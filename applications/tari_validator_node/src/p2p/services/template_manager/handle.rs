use tokio::sync::{mpsc, oneshot};

use crate::p2p::services::template_manager::{
    manager::TemplateMetadata,
    template_manager_service::{TemplateManagerRequest, TemplateManagerResponse},
    TemplateManagerError,
};

pub struct TemplateManagerHandle {
    request_tx: mpsc::Sender<(
        TemplateManagerRequest,
        oneshot::Sender<Result<TemplateManagerResponse, TemplateManagerError>>,
    )>,
}

impl TemplateManagerHandle {
    pub fn new(
        request_tx: mpsc::Sender<(
            TemplateManagerRequest,
            oneshot::Sender<Result<TemplateManagerResponse, TemplateManagerError>>,
        )>,
    ) -> Self {
        Self { request_tx }
    }

    pub async fn add_templates(&self, templates: Vec<TemplateMetadata>) -> Result<(), TemplateManagerError> {
        let (tx, rx) = oneshot::channel();
        self.request_tx
            .send((TemplateManagerRequest::AddTemplates { templates }, tx))
            .await
            .map_err(|_| TemplateManagerError::SendError)?;
        let _result = rx.await.map_err(|_| TemplateManagerError::SendError)?;
        Ok(())
    }
}
