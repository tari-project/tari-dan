use async_trait::async_trait;
use tari_dan_core::{services::MempoolOutboundService, DigitalAssetError};
use tari_dan_engine::instructions::Instruction;

pub struct TariCommsMempoolOutboundService {}

impl TariCommsMempoolOutboundService {
    pub fn new() -> Self {
        // TODO
        Self {}
    }
}

#[async_trait]
impl MempoolOutboundService for TariCommsMempoolOutboundService {
    async fn propagate_instruction(&mut self, _instruction: Instruction) -> Result<(), DigitalAssetError> {
        // TODO
        Ok(())
    }
}
