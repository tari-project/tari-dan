use async_trait::async_trait;
use tari_dan_engine::instructions::Instruction;

use crate::DigitalAssetError;

#[async_trait]
pub trait MempoolInboundService {
    async fn wait_for_instruction(&self) -> Result<Instruction, DigitalAssetError>;
}
