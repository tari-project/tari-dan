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
use tari_app_grpc::tari_rpc as grpc;
use tari_dan_core::{services::WalletClient, DigitalAssetError};

const _LOG_TARGET: &str = "tari::validator_node::app";

type Client = grpc::wallet_client::WalletClient<tonic::transport::Channel>;

#[derive(Clone)]
pub struct GrpcWalletClient {
    _endpoint: SocketAddr,
    _client: Option<Client>,
}

impl GrpcWalletClient {
    pub fn _new(endpoint: SocketAddr) -> GrpcWalletClient {
        Self {
            _endpoint: endpoint,
            _client: None,
        }
    }

    pub async fn _connection(&mut self) -> Result<&mut Client, DigitalAssetError> {
        if self._client.is_none() {
            let url = format!("http://{}", self._endpoint);
            let inner = Client::connect(url).await?;
            self._client = Some(inner);
        }
        self._client
            .as_mut()
            .ok_or_else(|| DigitalAssetError::FatalError("no connection".into()))
    }
}
#[async_trait]
impl WalletClient for GrpcWalletClient {}
