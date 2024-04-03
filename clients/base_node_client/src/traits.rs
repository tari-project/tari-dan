//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use async_trait::async_trait;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_core::{blocks::BlockHeader, transactions::transaction_components::CodeTemplateRegistration};
use tari_dan_common_types::SubstateAddress;

use crate::{
    error::BaseNodeClientError,
    types::{BaseLayerConsensusConstants, BaseLayerMetadata, BaseLayerValidatorNode, SideChainUtxos},
};

#[async_trait]
pub trait BaseNodeClient: Send + Sync + Clone {
    async fn test_connection(&mut self) -> Result<(), BaseNodeClientError>;
    async fn get_tip_info(&mut self) -> Result<BaseLayerMetadata, BaseNodeClientError>;
    async fn get_validator_nodes(&mut self, height: u64) -> Result<Vec<BaseLayerValidatorNode>, BaseNodeClientError>;
    async fn get_shard_key(
        &mut self,
        height: u64,
        public_key: &PublicKey,
    ) -> Result<Option<SubstateAddress>, BaseNodeClientError>;
    async fn get_template_registrations(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<CodeTemplateRegistration>, BaseNodeClientError>;
    async fn get_header_by_hash(&mut self, block_hash: FixedHash) -> Result<BlockHeader, BaseNodeClientError>;
    async fn get_consensus_constants(&mut self, tip: u64) -> Result<BaseLayerConsensusConstants, BaseNodeClientError>;
    async fn get_sidechain_utxos(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<SideChainUtxos>, BaseNodeClientError>;
}
