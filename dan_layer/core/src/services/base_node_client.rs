//  Copyright 2021. The Tari Project
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

use async_trait::async_trait;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::types::CommsPublicKey;
use tari_core::{
    blocks::BlockHeader,
    transactions::transaction_components::{CodeTemplateRegistration, TransactionOutput},
};
use tari_dan_common_types::ShardId;

use crate::{
    models::{BaseLayerMetadata, ValidatorNode},
    services::base_node_error::BaseNodeError,
};

#[async_trait]
pub trait BaseNodeClient: Send + Sync + Clone {
    async fn test_connection(&mut self) -> Result<(), BaseNodeError>;
    async fn get_tip_info(&mut self) -> Result<BaseLayerMetadata, BaseNodeError>;
    async fn get_validator_nodes(&mut self, height: u64) -> Result<Vec<ValidatorNode<CommsPublicKey>>, BaseNodeError>;
    async fn get_shard_key(&mut self, height: u64, public_key: &PublicKey) -> Result<Option<ShardId>, BaseNodeError>;
    async fn get_template_registrations(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<CodeTemplateRegistration>, BaseNodeError>;
    async fn get_header_by_hash(&mut self, block_hash: FixedHash) -> Result<BlockHeader, BaseNodeError>;
    async fn get_sidechain_utxos(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<SideChainUtxos>, BaseNodeError>;
}

#[derive(Debug, Clone)]
pub struct SideChainUtxos {
    pub block_info: BlockInfo,
    pub outputs: Vec<TransactionOutput>,
}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub hash: FixedHash,
    pub height: u64,
    pub next_block_hash: Option<FixedHash>,
}
