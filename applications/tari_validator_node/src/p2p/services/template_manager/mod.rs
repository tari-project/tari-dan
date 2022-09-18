pub mod handle;
mod initializer;
pub mod template_manager;
pub mod template_manager_service;

pub use initializer::spawn;
use tari_dan_core::storage::StorageError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TemplateManagerError {
    #[error("There was an error sending to a channel")]
    SendError,
    #[error("Could not fetch the template code from the web")]
    TemplateCodeFetchError,
    #[error("The hash of the template code does not match the metadata")]
    TemplateCodeHashMismatch,
    #[error("Unsupported template method {name}")]
    TemplateUnsupportedMethod { name: String },
    #[error("Storage error: {0}")]
    StorageError(#[from] StorageError),
}
