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

use std::{convert::TryInto, net::SocketAddr};

use async_trait::async_trait;
use log::trace;
use tari_app_grpc::tari_rpc::{self as grpc, GetShardKeyRequest};
use tari_base_node_grpc_client::BaseNodeGrpcClient;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::types::CommsPublicKey;
use tari_core::{blocks::BlockHeader, transactions::transaction_components::CodeTemplateRegistration};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    consensus_constants::BaseLayerConsensusConstants,
    models::{BaseLayerMetadata, ValidatorNode},
    services::{base_node_error::BaseNodeError, BaseNodeClient, BlockInfo, SideChainUtxos},
};

const LOG_TARGET: &str = "tari::validator_node::app";

type Client = BaseNodeGrpcClient<tonic::transport::Channel>;

#[derive(Clone)]
pub struct GrpcBaseNodeClient {
    endpoint: SocketAddr,
    client: Option<Client>,
}

impl GrpcBaseNodeClient {
    pub fn new(endpoint: SocketAddr) -> GrpcBaseNodeClient {
        Self { endpoint, client: None }
    }

    async fn connection(&mut self) -> Result<&mut Client, BaseNodeError> {
        if self.client.is_none() {
            let url = format!("http://{}", self.endpoint);
            let inner = Client::connect(url).await?;
            self.client = Some(inner);
        }
        self.client.as_mut().ok_or(BaseNodeError::ConnectionError)
    }

    pub async fn get_consensus_constants(
        &mut self,
        block_height: u64,
    ) -> Result<BaseLayerConsensusConstants, BaseNodeError> {
        let inner = self.connection().await?;

        let request = grpc::BlockHeight { block_height };
        let result = inner.get_constants(request).await?.into_inner();

        let consensus_constants = BaseLayerConsensusConstants::new(result.validator_node_timeout);
        Ok(consensus_constants)
    }
}

#[async_trait]
impl BaseNodeClient for GrpcBaseNodeClient {
    async fn test_connection(&mut self) -> Result<(), BaseNodeError> {
        self.connection().await?;
        Ok(())
    }

    async fn get_tip_info(&mut self) -> Result<BaseLayerMetadata, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::Empty {};
        let result = inner.get_tip_info(request).await?.into_inner();
        let metadata = result
            .metadata
            .ok_or_else(|| BaseNodeError::InvalidPeerMessage("Base node returned no metadata".to_string()))?;
        Ok(BaseLayerMetadata {
            height_of_longest_chain: metadata.height_of_longest_chain,
            tip_hash: metadata
                .best_block
                .try_into()
                .map_err(|_| BaseNodeError::InvalidPeerMessage("best_block was not a valid fixed hash".to_string()))?,
        })
    }

    async fn get_validator_nodes(&mut self, height: u64) -> Result<Vec<ValidatorNode>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::GetActiveValidatorNodesRequest { height };
        let mut vns = vec![];
        let mut stream = inner.get_active_validator_nodes(request).await?.into_inner();
        loop {
            match stream.message().await {
                Ok(Some(val)) => {
                    vns.push(ValidatorNode {
                        public_key: CommsPublicKey::from_bytes(&val.public_key).map_err(|_| {
                            BaseNodeError::InvalidPeerMessage("public_key was not a valid public key".to_string())
                        })?,
                        shard_key: ShardId::from_bytes(&val.shard_key).map_err(|_| {
                            BaseNodeError::InvalidPeerMessage("shard_id was not a valid fixed hash".to_string())
                        })?,
                    });
                },
                Ok(None) => {
                    trace!(target: LOG_TARGET, "No new validator nodes for this epoch");
                    break;
                },
                Err(e) => {
                    return Err(BaseNodeError::InvalidPeerMessage(format!(
                        "Error reading stream: {}",
                        e
                    )));
                },
            }
        }
        Ok(vns)
    }

    async fn get_shard_key(&mut self, height: u64, public_key: &PublicKey) -> Result<Option<ShardId>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = GetShardKeyRequest {
            height,
            public_key: public_key.to_vec(),
        };
        let result = inner.get_shard_key(request).await?.into_inner();
        if result.shard_key.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ShardId::from_bytes(result.shard_key.as_bytes())?))
        }
    }

    async fn get_template_registrations(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<CodeTemplateRegistration>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::GetTemplateRegistrationsRequest {
            start_hash: start_hash.map(|v| v.to_vec()).unwrap_or_default(),
            count,
        };
        let mut templates = vec![];
        let mut stream = inner.get_template_registrations(request).await?.into_inner();
        loop {
            match stream.message().await {
                Ok(Some(val)) => {
                    let template_registration: CodeTemplateRegistration = val
                        .registration
                        .ok_or_else(|| {
                            BaseNodeError::InvalidPeerMessage("Base node returned no template registration".to_string())
                        })?
                        .try_into()
                        .map_err(|_| BaseNodeError::InvalidPeerMessage("invalid template registration".to_string()))?;
                    templates.push(template_registration);
                },
                Ok(None) => {
                    break;
                },
                Err(e) => {
                    return Err(BaseNodeError::InvalidPeerMessage(format!(
                        "Error reading stream: {}",
                        e
                    )));
                },
            }
        }
        Ok(templates)
    }

    async fn get_header_by_hash(&mut self, block_hash: FixedHash) -> Result<BlockHeader, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::GetHeaderByHashRequest {
            hash: block_hash.to_vec(),
        };
        let result = inner.get_header_by_hash(request).await?.into_inner();
        let header = result
            .header
            .ok_or_else(|| BaseNodeError::InvalidPeerMessage("Base node returned no header".to_string()))?;
        let header = header.try_into().map_err(BaseNodeError::InvalidPeerMessage)?;
        Ok(header)
    }

    async fn get_sidechain_utxos(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<SideChainUtxos>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = grpc::GetSideChainUtxosRequest {
            start_hash: start_hash.map(|v| v.to_vec()).unwrap_or_default(),
            count,
        };
        let mut stream = inner.get_side_chain_utxos(request).await?.into_inner();
        let mut responses = Vec::with_capacity(count as usize);
        loop {
            match stream.message().await {
                Ok(Some(resp)) => {
                    let block_info = resp.block_info.ok_or_else(|| {
                        BaseNodeError::InvalidPeerMessage("Base node returned no block info".to_string())
                    })?;
                    let resp = SideChainUtxos {
                        block_info: BlockInfo {
                            height: block_info.height,
                            hash: block_info.hash.try_into()?,
                            next_block_hash: Some(block_info.next_block_hash)
                                .filter(|v| !v.is_empty())
                                .map(TryInto::try_into)
                                .transpose()?,
                        },
                        outputs: resp
                            .outputs
                            .into_iter()
                            .map(TryInto::try_into)
                            .collect::<Result<_, _>>()
                            .map_err(BaseNodeError::InvalidPeerMessage)?,
                    };
                    responses.push(resp);
                },
                Ok(None) => {
                    break;
                },
                Err(e) => {
                    return Err(BaseNodeError::InvalidPeerMessage(format!(
                        "Error reading stream: {}",
                        e
                    )));
                },
            }
        }

        Ok(responses)
    }
}
