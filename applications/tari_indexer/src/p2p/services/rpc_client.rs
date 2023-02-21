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

use std::convert::{TryFrom, TryInto};

use anyhow::anyhow;
use async_trait::async_trait;
use tari_common_types::types::PublicKey;
use tari_comms::{
    connectivity::ConnectivityRequester,
    multiaddr::Multiaddr,
    peer_manager::NodeId,
    types::CommsPublicKey,
};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_app_grpc::proto::rpc::{GetPeersRequest, SubmitTransactionRequest};
use tari_dan_core::services::{DanPeer, ValidatorNodeClientError, ValidatorNodeClientFactory, ValidatorNodeRpcClient};
use tari_transaction::Transaction;
use tokio_stream::StreamExt;

use crate::p2p::rpc;

pub struct TariCommsValidatorNodeRpcClient {
    connectivity: ConnectivityRequester,
    address: PublicKey,
}

impl TariCommsValidatorNodeRpcClient {
    pub async fn create_connection(&mut self) -> Result<rpc::ValidatorNodeRpcClient, ValidatorNodeClientError> {
        let mut conn = self
            .connectivity
            .dial_peer(NodeId::from_public_key(&self.address))
            .await?;
        let client = conn.connect_rpc().await?;
        Ok(client)
    }
}

#[async_trait]
impl ValidatorNodeRpcClient for TariCommsValidatorNodeRpcClient {
    async fn submit_transaction(
        &mut self,
        transaction: Transaction,
    ) -> Result<Option<Vec<u8>>, ValidatorNodeClientError> {
        let mut client = self.create_connection().await?;
        let request: SubmitTransactionRequest = SubmitTransactionRequest {
            transaction: Some(transaction.into()),
        };
        let response = client.submit_transaction(request).await?;

        Ok(if response.result.is_empty() {
            None
        } else {
            Some(response.result)
        })
    }

    async fn get_peers(&mut self) -> Result<Vec<DanPeer<CommsPublicKey>>, ValidatorNodeClientError> {
        let mut client = self.create_connection().await?;
        // TODO(perf): This doesnt scale, find a nice way to wrap up the stream
        let peers = client
            .get_peers(GetPeersRequest { since: 0 })
            .await?
            .map(|result| {
                let p = result?;
                let identity_sig = p.identity_signature.unwrap_or_default();
                Result::<_, ValidatorNodeClientError>::Ok(DanPeer {
                    identity: CommsPublicKey::from_bytes(&p.identity)
                        .map_err(|_| ValidatorNodeClientError::InvalidResponse(anyhow!("Invalid identity")))?,
                    addresses: p
                        .addresses
                        .into_iter()
                        .map(|a| {
                            Multiaddr::try_from(a)
                                .map_err(|_| ValidatorNodeClientError::InvalidResponse(anyhow!("Invalid address")))
                        })
                        .collect::<Result<_, _>>()?,
                    identity_signature: Some(
                        identity_sig
                            .try_into()
                            .map_err(ValidatorNodeClientError::InvalidResponse)?,
                    ),
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .await?;
        Ok(peers)
    }
}

#[derive(Clone)]
pub struct TariCommsValidatorNodeClientFactory {
    connectivity: ConnectivityRequester,
}

impl TariCommsValidatorNodeClientFactory {
    pub fn new(connectivity: ConnectivityRequester) -> Self {
        Self { connectivity }
    }
}

impl ValidatorNodeClientFactory for TariCommsValidatorNodeClientFactory {
    type Addr = PublicKey;
    type Client = TariCommsValidatorNodeRpcClient;

    fn create_client(&self, address: &Self::Addr) -> Self::Client {
        TariCommsValidatorNodeRpcClient {
            connectivity: self.connectivity.clone(),
            address: address.clone(),
        }
    }
}
