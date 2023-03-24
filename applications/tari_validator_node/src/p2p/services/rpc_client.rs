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

use std::convert::TryFrom;

use anyhow::anyhow;
use async_trait::async_trait;
use tari_common_types::types::PublicKey;
use tari_comms::{
    connectivity::ConnectivityRequester,
    multiaddr::Multiaddr,
    peer_manager::{NodeId, PeerIdentityClaim},
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
                let addresses: Vec<Multiaddr> = p
                    .addresses
                    .into_iter()
                    .map(|a| {
                        Multiaddr::try_from(a)
                            .map_err(|_| ValidatorNodeClientError::InvalidResponse(anyhow!("Invalid address")))
                    })
                    .collect::<Result<_, _>>()?;
                let claims: Vec<PeerIdentityClaim> = p
                    .claims
                    .into_iter()
                    .map(|c| {
                        PeerIdentityClaim::try_from(c)
                            .map_err(|_| ValidatorNodeClientError::InvalidResponse(anyhow!("Invalid claim")))
                    })
                    .collect::<Result<_, _>>()?;
                Result::<_, ValidatorNodeClientError>::Ok(DanPeer {
                    identity: CommsPublicKey::from_bytes(&p.identity)
                        .map_err(|_| ValidatorNodeClientError::InvalidResponse(anyhow!("Invalid identity")))?,
                    addresses: addresses.into_iter().zip(claims).collect(),
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .await?;
        Ok(peers)
    }
    // async fn get_sidechain_state(
    //     &mut self,
    //     contract_id: &FixedHash,
    // ) -> Result<Vec<SchemaState>, ValidatorNodeClientError> {
    //     let mut connection = self.create_connection().await?;
    //     let mut client = connection.connect_rpc::<rpc::ValidatorNodeRpcClient>().await?;
    //     let request = proto::GetSidechainStateRequest {
    //         contract_id: contract_id.to_vec(),
    //     };
    //
    //     let mut stream = client.get_sidechain_state(request).await?;
    //     // TODO: Same issue as get_sidechain_blocks
    //     let mut schemas = Vec::new();
    //     let mut current_schema = None;
    //     while let Some(resp) = stream.next().await {
    //         let resp = resp?;
    //
    //         match resp.state {
    //             Some(proto::get_sidechain_state_response::State::Schema(name)) => {
    //                 if let Some(schema) = current_schema.take() {
    //                     schemas.push(schema);
    //                 }
    //                 current_schema = Some(SchemaState::new(name, vec![]));
    //             },
    //             Some(proto::get_sidechain_state_response::State::KeyValue(kv)) => match current_schema.as_mut() {
    //                 Some(schema) => {
    //                     let kv = kv.try_into().map_err(ValidatorNodeClientError::InvalidPeerMessage)?;
    //                     schema.push_key_value(kv);
    //                 },
    //                 None => {
    //                     return Err(ValidatorNodeClientError::InvalidPeerMessage(anyhow!(
    //                         "Peer {} sent a key value response without first defining the schema",
    //                         self.address
    //                     )))
    //                 },
    //             },
    //             None => {
    //                 return Err(ValidatorNodeClientError::ProtocolViolation {
    //                     peer: self.address.clone(),
    //                     details: "get_sidechain_state: Peer sent response without state".to_string(),
    //                 })
    //             },
    //         }
    //     }
    //
    //     if let Some(schema) = current_schema {
    //         schemas.push(schema);
    //     }
    //
    //     Ok(schemas)
    // }
    //
    // async fn get_op_logs(
    //     &mut self,
    //     contract_id: &FixedHash,
    //     height: u64,
    // ) -> Result<Vec<StateOpLogEntry>, ValidatorNodeClientError> {
    //     let mut connection = self.create_connection().await?;
    //     let mut client = connection.connect_rpc::<rpc::ValidatorNodeRpcClient>().await?;
    //     let request = proto::GetStateOpLogsRequest {
    //         contract_id: contract_id.as_bytes().to_vec(),
    //         height,
    //     };
    //
    //     let resp = client.get_op_logs(request).await?;
    //     let op_logs = resp
    //         .op_logs
    //         .into_iter()
    //         .map(TryInto::try_into)
    //         .collect::<Result<Vec<_>, _>>()
    //         .map_err(ValidatorNodeClientError::InvalidPeerMessage)?;
    //
    //     Ok(op_logs)
    // }
    //
    // async fn get_tip_node(&mut self, contract_id: &FixedHash) -> Result<Option<Node>, ValidatorNodeClientError> {
    //     let mut connection = self.create_connection().await?;
    //     let mut client = connection.connect_rpc::<rpc::ValidatorNodeRpcClient>().await?;
    //     let request = proto::GetTipNodeRequest {
    //         contract_id: contract_id.to_vec(),
    //     };
    //     let resp = client.get_tip_node(request).await?;
    //     resp.tip_node
    //         .map(TryInto::try_into)
    //         .transpose()
    //         .map_err(ValidatorNodeClientError::InvalidPeerMessage)
    // }
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
