use async_trait::async_trait;
use tari_engine_types::substate::Substate;

#[derive(thiserror::Error, Debug)]
#[error("Failed substate cache operation {0}")]
pub struct SubstateCacheError(pub String);

#[async_trait]
pub trait SubstateCache: Send + Sync {
    async fn read(self, address: String) -> Result<Option<Substate>, SubstateCacheError>;
    async fn write(self, address: String, substate: &Substate) -> Result<(), SubstateCacheError>;
}