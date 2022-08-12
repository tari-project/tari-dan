use async_trait::async_trait;
use tari_dan_engine::instructions::Instruction;

use crate::DigitalAssetError;

#[async_trait]
pub trait MempoolOutboundService: Sync + Send + 'static {
    async fn propagate_instruction(&mut self, instruction: Instruction) -> Result<(), DigitalAssetError>;
}
