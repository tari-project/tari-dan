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
use log::info;
use tari_app_grpc::tari_rpc::{self as grpc, GetCommitteeRequest, GetShardKeyRequest};
use tari_common_types::types::PublicKey;
use tari_comms::types::CommsPublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{BaseLayerMetadata, ValidatorNode},
    services::{base_node_error::BaseNodeError, BaseNodeClient},
};

const LOG_TARGET: &str = "tari::validator_node::app";

type Client = grpc::base_node_client::BaseNodeClient<tonic::transport::Channel>;

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
}
#[async_trait]
impl BaseNodeClient for GrpcBaseNodeClient {
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
        dbg!(&request);
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
                    info!(target: LOG_TARGET, "No more validator nodes");

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

    async fn get_committee(&mut self, height: u64, shard_key: &[u8; 32]) -> Result<Vec<CommsPublicKey>, BaseNodeError> {
        let inner = self.connection().await?;
        let request = GetCommitteeRequest {
            height,
            shard_key: shard_key.to_vec(),
        };
        let result = inner.get_committee(request).await?.into_inner();
        Ok(result
            .public_key
            .iter()
            .map(|a| CommsPublicKey::from_vec(a).unwrap())
            .collect())
    }

    async fn get_shard_key(&mut self, height: u64, public_key: &PublicKey) -> Result<ShardId, BaseNodeError> {
        let inner = self.connection().await?;
        let request = GetShardKeyRequest {
            height,
            public_key: public_key.to_vec(),
        };
        let result = inner.get_shard_key(request).await?.into_inner();
        println!("res {:?}", result);
        Ok(ShardId::from_bytes(result.shard_key.as_bytes())?)
    }
}
