//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
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

use futures_util::TryStreamExt;
use log::*;
use minotari_app_grpc::{
    tari_rpc,
    tari_rpc::{self as grpc, GetValidatorNodeChangesRequest, base_node_client::BaseNodeClient as BaseNodeGrpcClient},
};
use tari_node_components::blocks::BlockHeader;
use tari_ootle_common_types::{Epoch, SubstateAddress, SubstateVersion};
use tari_sidechain::FixedHash;
use tari_template_lib::types::{Hash32, crypto::RistrettoPublicKeyBytes};
use tari_utilities::ByteArray;
use tonic::codegen::tokio_stream::Stream;
use url::Url;

use crate::{
    BaseNodeClient,
    BaseNodeClientError,
    types::{BaseLayerConsensusConstants, BaseLayerMetadata, BaseLayerValidatorNode, BlockInfo, SideChainUtxos},
};

const LOG_TARGET: &str = "tari::validator_node::app";

type Client = BaseNodeGrpcClient<tonic::transport::Channel>;

#[derive(Clone)]
pub struct GrpcBaseNodeClient {
    endpoint: Url,
    client: Option<Client>,
}

impl GrpcBaseNodeClient {
    pub fn new(endpoint: Url) -> Self {
        Self { endpoint, client: None }
    }

    pub async fn connect(endpoint: Url) -> Result<Self, BaseNodeClientError> {
        let mut client = Self { endpoint, client: None };
        client.test_connection().await?;
        Ok(client)
    }

    async fn connection(&mut self) -> Result<&mut Client, BaseNodeClientError> {
        if self.client.is_none() {
            let inner = Client::connect(self.endpoint.to_string()).await?;
            self.client = Some(inner);
        }
        self.client.as_mut().ok_or(BaseNodeClientError::ConnectionError)
    }

    pub async fn get_mempool_transaction_count(&mut self) -> Result<usize, BaseNodeClientError> {
        let inner = self.connection().await.unwrap();
        let request = grpc::GetMempoolTransactionsRequest {};

        let mut count = 0;
        let mut stream = inner.get_mempool_transactions(request).await?.into_inner();
        loop {
            match stream.message().await {
                Ok(Some(_val)) => {
                    count += 1;
                },
                Ok(None) => {
                    break;
                },
                Err(e) => {
                    warn!(target: LOG_TARGET, "Error getting mempool transaction count: {}", e);
                    return Err(BaseNodeClientError::ConnectionError);
                },
            }
        }
        Ok(count)
    }
}

impl BaseNodeClient for GrpcBaseNodeClient {
    async fn test_connection(&mut self) -> Result<(), BaseNodeClientError> {
        self.connection().await?;
        Ok(())
    }

    async fn get_network(&mut self) -> Result<u8, BaseNodeClientError> {
        let inner = self.connection().await?;
        let request = grpc::Empty {};
        let result = inner.get_version(request).await?.into_inner();
        u8::try_from(result.network)
            .map_err(|_| BaseNodeClientError::InvalidPeerMessage(format!("Invalid network byte {}", result.network)))
    }

    async fn get_tip_info(&mut self) -> Result<BaseLayerMetadata, BaseNodeClientError> {
        let inner = self.connection().await?;
        let request = grpc::Empty {};
        let result = inner.get_tip_info(request).await?.into_inner();
        let metadata = result
            .metadata
            .ok_or_else(|| BaseNodeClientError::InvalidPeerMessage("Base node returned no metadata".to_string()))?;
        Ok(BaseLayerMetadata {
            height_of_longest_chain: metadata.best_block_height,
            tip_hash: metadata.best_block_hash.try_into().map_err(|_| {
                BaseNodeClientError::InvalidPeerMessage("best_block was not a valid fixed hash".to_string())
            })?,
        })
    }

    async fn get_validator_node_changes(
        &mut self,
        epoch: Epoch,
        sidechain_id: Option<&RistrettoPublicKeyBytes>,
    ) -> Result<Vec<tari_rpc::ValidatorNodeChange>, BaseNodeClientError> {
        let client = self.connection().await?;
        let result = client
            .get_validator_node_changes(GetValidatorNodeChangesRequest {
                epoch: epoch.as_u64(),
                sidechain_id: sidechain_id.map(|id| id.as_bytes().to_vec()).unwrap_or_default(),
            })
            .await?
            .into_inner();

        let changes = result.changes;
        Ok(changes)
    }

    async fn get_validator_nodes(
        &mut self,
        height: u64,
    ) -> Result<impl Stream<Item = Result<BaseLayerValidatorNode, BaseNodeClientError>> + Send, BaseNodeClientError>
    {
        let inner = self.connection().await?;

        // SidechainId is empty because we need all the sidechain nodes to create the merkle root
        let request = grpc::GetActiveValidatorNodesRequest {
            height,
            sidechain_id: vec![],
        };
        let stream = inner.get_active_validator_nodes(request).await?.into_inner();

        let stream = stream
            .map_err(|e| BaseNodeClientError::GrpcStatus {
                code: e.code(),
                message: e.message().to_string(),
            })
            .and_then(|resp| async move {
                Ok(BaseLayerValidatorNode {
                    public_key: RistrettoPublicKeyBytes::from_bytes(&resp.public_key).map_err(|_| {
                        BaseNodeClientError::InvalidPeerMessage("public_key was not a valid public key".to_string())
                    })?,
                    shard_key: {
                        let hash = Hash32::try_from(resp.shard_key.as_slice()).map_err(|_| {
                            BaseNodeClientError::InvalidPeerMessage("shard_key was not a valid fixed hash".to_string())
                        })?;
                        SubstateAddress::from_hash_and_version(hash, SubstateVersion::ZERO)
                    },
                    sidechain_id: Some(&resp.sidechain_id)
                        .filter(|s| !s.is_empty())
                        .map(|sid| {
                            RistrettoPublicKeyBytes::from_bytes(sid).map_err(|_| {
                                BaseNodeClientError::InvalidPeerMessage(
                                    "sidechain_id was not a valid public key".to_string(),
                                )
                            })
                        })
                        .transpose()?,
                })
            });

        Ok(Box::pin(stream))
    }

    async fn get_header_by_hash(&mut self, block_hash: &FixedHash) -> Result<BlockHeader, BaseNodeClientError> {
        let inner = self.connection().await?;
        let request = grpc::GetHeaderByHashRequest {
            hash: block_hash.to_vec(),
        };
        let result = inner.get_header_by_hash(request).await?.into_inner();
        let header = result
            .header
            .ok_or_else(|| BaseNodeClientError::InvalidPeerMessage("Base node returned no header".to_string()))?;
        let header = header.try_into().map_err(BaseNodeClientError::InvalidPeerMessage)?;
        Ok(header)
    }

    async fn stream_headers(
        &mut self,
        from_height: u64,
        limit: u64,
    ) -> Result<impl Stream<Item = Result<BlockHeader, BaseNodeClientError>> + Unpin + Send, BaseNodeClientError> {
        let inner = self.connection().await?;
        let request = grpc::ListHeadersRequest {
            from_height,
            num_headers: limit,
            sorting: tari_rpc::Sorting::Asc as i32,
        };
        let stream = inner.list_headers(request).await?.into_inner();
        let stream = stream
            .map_err(|e| BaseNodeClientError::GrpcStatus {
                code: e.code(),
                message: e.message().to_string(),
            })
            .and_then(|header_msg| async {
                let header = header_msg.header.ok_or_else(|| {
                    BaseNodeClientError::InvalidPeerMessage("Base node returned no header".to_string())
                })?;
                header.try_into().map_err(BaseNodeClientError::InvalidPeerMessage)
            });

        Ok(Box::pin(stream))
    }

    async fn get_consensus_constants(
        &mut self,
        block_height: u64,
    ) -> Result<BaseLayerConsensusConstants, BaseNodeClientError> {
        let inner = self.connection().await?;

        let request = grpc::BlockHeight { block_height };
        let result = inner.get_constants(request).await?.into_inner();
        if result.epoch_length == 0 {
            return Err(BaseNodeClientError::InvalidPeerMessage(
                "Base node returned a zero epoch_length in consensus constants".to_string(),
            ));
        }

        let consensus_constants = BaseLayerConsensusConstants {
            epoch_length: result.epoch_length,
            validator_node_registration_min_deposit_amount: result
                .validator_node_registration_min_deposit_amount
                .into(),
        };
        Ok(consensus_constants)
    }

    async fn get_sidechain_utxos(
        &mut self,
        start_hash: Option<FixedHash>,
        count: u64,
    ) -> Result<Vec<SideChainUtxos>, BaseNodeClientError> {
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
                        BaseNodeClientError::InvalidPeerMessage("Base node returned no block info".to_string())
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
                            .map_err(BaseNodeClientError::InvalidPeerMessage)?,
                    };
                    responses.push(resp);
                },
                Ok(None) => {
                    break;
                },
                Err(e) => {
                    return Err(BaseNodeClientError::InvalidPeerMessage(format!(
                        "Error reading stream: {}",
                        e
                    )));
                },
            }
        }

        Ok(responses)
    }
}
