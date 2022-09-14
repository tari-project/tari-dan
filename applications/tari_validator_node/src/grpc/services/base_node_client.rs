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
use tari_app_grpc::tari_rpc::{self as grpc, GetCommitteeRequest};
use tari_comms::types::CommsPublicKey;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_core::{
    models::{BaseLayerMetadata, ValidatorNode},
    services::BaseNodeClient,
    DigitalAssetError,
};

// const LOG_TARGET: &str = "tari::validator_node::app";

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

    pub async fn connection(&mut self) -> Result<&mut Client, DigitalAssetError> {
        if self.client.is_none() {
            let url = format!("http://{}", self.endpoint);
            let inner = Client::connect(url).await?;
            self.client = Some(inner);
        }
        self.client
            .as_mut()
            .ok_or_else(|| DigitalAssetError::FatalError("no connection".into()))
    }
}
#[async_trait]
impl BaseNodeClient for GrpcBaseNodeClient {
    async fn get_tip_info(&mut self) -> Result<BaseLayerMetadata, DigitalAssetError> {
        let inner = self.connection().await?;
        let request = grpc::Empty {};
        let result = inner.get_tip_info(request).await?.into_inner();
        let metadata = result
            .metadata
            .ok_or_else(|| DigitalAssetError::InvalidPeerMessage("Base node returned no metadata".to_string()))?;
        Ok(BaseLayerMetadata {
            height_of_longest_chain: metadata.height_of_longest_chain,
            tip_hash: metadata.best_block.try_into().map_err(|_| {
                DigitalAssetError::InvalidPeerMessage("best_block was not a valid fixed hash".to_string())
            })?,
        })
    }

    async fn get_validator_nodes(&mut self, height: u64) -> Result<Vec<ValidatorNode>, DigitalAssetError> {
        let inner = self.connection().await?;
        let request = grpc::Empty {};
        let result = inner.get_tip_info(request).await?.into_inner();
        Ok(vec![])
    }

    async fn get_committee(
        &mut self,
        height: u64,
        shard_key: &[u8; 32],
    ) -> Result<Vec<CommsPublicKey>, DigitalAssetError> {
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

    async fn get_shard_key(&mut self, height: u64, public_key: &[u8; 32]) -> Result<&[u8; 32], DigitalAssetError> {
        todo!()
    }
}
