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

use std::net::SocketAddr;

use async_trait::async_trait;
use tari_app_grpc::tari_rpc::{self as grpc, RegisterValidatorNodeRequest, RegisterValidatorNodeResponse};
use tari_comms::NodeIdentity;
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_core::{services::WalletClient, DigitalAssetError};

use crate::registration_signing::sign_registration;

const _LOG_TARGET: &str = "tari::validator_node::app";

type Client = grpc::wallet_client::WalletClient<tonic::transport::Channel>;

#[derive(Clone)]
pub struct GrpcWalletClient {
    endpoint: SocketAddr,
    client: Option<Client>,
}

impl GrpcWalletClient {
    pub fn new(endpoint: SocketAddr) -> GrpcWalletClient {
        Self { endpoint, client: None }
    }

    pub async fn connection(&mut self) -> Result<&mut Client, DigitalAssetError> {
        if self.client.is_none() {
            let url = format!("http://{}", self.endpoint);
            let inner = Client::connect(url).await?;
            self.client = Some(inner);
        }
        dbg!(self.endpoint);
        self.client
            .as_mut()
            .ok_or_else(|| DigitalAssetError::FatalError("no connection".into()))
    }

    pub async fn register_validator_node(
        &mut self,
        node_identity: &NodeIdentity,
    ) -> Result<RegisterValidatorNodeResponse, DigitalAssetError> {
        let inner = self.connection().await?;
        let signature = sign_registration(node_identity.secret_key(), 123);
        let request = RegisterValidatorNodeRequest {
            validator_node_public_key: node_identity.public_key().to_vec(),
            validator_node_signature: Some(signature.into()),
            fee_per_gram: 1,
            message: "Registering VN".to_string(),
        };
        let result = inner.register_validator_node(request).await?.into_inner();
        Ok(result)
    }
}

#[async_trait]
impl WalletClient for GrpcWalletClient {}
