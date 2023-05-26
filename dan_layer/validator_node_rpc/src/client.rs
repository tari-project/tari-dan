//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    convert::{TryFrom, TryInto},
    ops::{Deref, DerefMut},
    sync::Arc,
};

use anyhow::anyhow;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tari_bor::decode;
use tari_common_types::types::{FixedHash, PublicKey};
use tari_comms::{
    connectivity::ConnectivityRequester,
    multiaddr::Multiaddr,
    peer_manager::{NodeId, PeerIdentityClaim},
    protocol::rpc::RpcPoolClient,
    types::CommsPublicKey,
    PeerConnection,
};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{NodeAddressable, PayloadId};
use tari_dan_core::services::{DanPeer, ValidatorNodeClientError};
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateAddress},
};
use tari_transaction::Transaction;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio_stream::StreamExt;
use tonic::codegen::Body;

use crate::{
    proto::rpc::{
        GetPeersRequest,
        GetTransactionResultRequest,
        PayloadResultStatus,
        SubmitTransactionRequest,
        SubstateStatus,
    },
    rpc_service,
};

#[async_trait]
pub trait ValidatorNodeClientFactory: Send + Sync {
    type Addr: NodeAddressable;
    type Client<'a>: ValidatorNodeRpcClient<Addr = Self::Addr>
    where Self: 'a;

    async fn create_client<'b: 'a, 'a>(&'b self, address: &Self::Addr) -> Self::Client<'a>;
}

#[async_trait]
pub trait ValidatorNodeRpcClient: Send + Sync {
    type Addr: NodeAddressable;
    type Error: std::error::Error + Send + Sync + 'static;

    async fn submit_transaction(&mut self, transaction: Transaction) -> Result<PayloadId, Self::Error>;
    async fn get_finalized_transaction_result(
        &mut self,
        payload_id: PayloadId,
    ) -> Result<TransactionResultStatus, Self::Error>;

    async fn get_peers(&mut self) -> Result<Vec<DanPeer<Self::Addr>>, Self::Error>;

    async fn get_substate(&mut self, address: &SubstateAddress, version: u32) -> Result<SubstateResult, Self::Error>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum TransactionResultStatus {
    Pending,
    Finalized(ExecuteResult),
}

impl TransactionResultStatus {
    pub fn into_finalized(&self) -> Option<ExecuteResult> {
        match self {
            Self::Pending => None,
            Self::Finalized(result) => Some(result.clone()),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SubstateResult {
    DoesNotExist,
    Up {
        substate: Substate,
        created_by_tx: FixedHash,
    },
    Down {
        version: u32,
    },
}

pub struct TariCommsValidatorNodeRpcClient {
    connectivity: ConnectivityRequester,
    address: PublicKey,
    connection: Option<(PeerConnection, rpc_service::ValidatorNodeRpcClient)>,
}

impl TariCommsValidatorNodeRpcClient {
    pub async fn client_connection(&mut self) -> Result<rpc_service::ValidatorNodeRpcClient, ValidatorNodeClientError> {
        if let Some((_, ref client)) = self.connection {
            if client.is_connected() {
                return Ok(client.clone());
            }
        }
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
    type Addr = CommsPublicKey;
    type Error = ValidatorNodeClientError;

    async fn submit_transaction(&mut self, transaction: Transaction) -> Result<PayloadId, ValidatorNodeClientError> {
        let mut client = self.client_connection().await?;
        let request = SubmitTransactionRequest {
            transaction: Some(transaction.into()),
        };
        let response = client.submit_transaction(request).await?;

        let payload_id = response.transaction_hash.try_into().map_err(|_| {
            ValidatorNodeClientError::InvalidResponse(anyhow!("Node returned an invalid or empty payload id"))
        })?;

        Ok(payload_id)
    }

    async fn get_peers(&mut self) -> Result<Vec<DanPeer<Self::Addr>>, ValidatorNodeClientError> {
        let mut client = self.client_connection().await?;
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

    async fn get_substate(&mut self, address: &SubstateAddress, version: u32) -> Result<SubstateResult, Self::Error> {
        let mut client = self.client_connection().await?;
        // request the shard substate to the VN
        let request = crate::proto::rpc::GetSubstateRequest {
            address: address.to_bytes(),
            version,
        };

        let resp = client.get_substate(request).await?;
        let status = SubstateStatus::from_i32(resp.status).ok_or_else(|| {
            ValidatorNodeClientError::InvalidResponse(anyhow!("Node returned invalid substate status {}", resp.status))
        })?;

        // TODO: verify the quorum certificates
        // for qc in resp.quorum_certificates {
        //     let qc = QuorumCertificate::try_from(&qc)?;
        // }

        match status {
            SubstateStatus::Up => {
                let tx_hash = resp.transaction_hash.try_into().map_err(|_| {
                    ValidatorNodeClientError::InvalidResponse(anyhow!(
                        "Node returned an invalid or empty transaction hash"
                    ))
                })?;
                let substate = Substate::from_bytes(&resp.substate)
                    .map_err(|e| ValidatorNodeClientError::InvalidResponse(anyhow!(e)))?;
                Ok(SubstateResult::Up {
                    substate,
                    created_by_tx: tx_hash,
                })
            },
            SubstateStatus::Down => Ok(SubstateResult::Down { version: resp.version }),
            SubstateStatus::DoesNotExist => Ok(SubstateResult::DoesNotExist),
        }
    }

    async fn get_finalized_transaction_result(
        &mut self,
        payload_id: PayloadId,
    ) -> Result<TransactionResultStatus, ValidatorNodeClientError> {
        let mut client = self.client_connection().await?;
        let request = GetTransactionResultRequest {
            payload_id: payload_id.as_bytes().to_vec(),
        };
        let response = client.get_transaction_result(request).await?;

        match PayloadResultStatus::from_i32(response.status) {
            Some(PayloadResultStatus::Pending) => Ok(TransactionResultStatus::Pending),
            Some(PayloadResultStatus::Finalized) => {
                let execution_result = decode(&response.execution_result).map_err(|_| {
                    ValidatorNodeClientError::InvalidResponse(anyhow!("Node returned an invalid or empty payload id"))
                })?;
                Ok(TransactionResultStatus::Finalized(execution_result))
            },
            None => Err(ValidatorNodeClientError::InvalidResponse(anyhow!(
                "Node returned invalid payload status {}",
                response.status
            ))),
        }
    }
}

#[derive(Debug)]
pub struct TariCommsValidatorNodeClientFactory {
    connectivity: ConnectivityRequester,
    limit: Semaphore,
    max_permits: usize,
}

impl Clone for TariCommsValidatorNodeClientFactory {
    fn clone(&self) -> Self {
        Self {
            connectivity: self.connectivity.clone(),
            limit: Semaphore::new(self.max_permits),
            max_permits: self.max_permits,
        }
    }
}

impl TariCommsValidatorNodeClientFactory {
    pub fn new(connectivity: ConnectivityRequester, max_connections: usize) -> Self {
        Self {
            connectivity,
            limit: Semaphore::new(max_connections),
            max_permits: max_connections,
        }
    }
}

pub struct SemaphoreWrappedClient<'a> {
    client: TariCommsValidatorNodeRpcClient,
    permit: SemaphorePermit<'a>,
}

impl<'a> Deref for SemaphoreWrappedClient<'a> {
    type Target = TariCommsValidatorNodeRpcClient;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl<'a> DerefMut for SemaphoreWrappedClient<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}

// impl Deref for SemaphoreWrappedClient {
//     type Target = TariCommsValidatorNodeRpcClient;
//
//     fn deref(&self) -> &Self::Target {
//         &self.client
//     }
// }

#[async_trait]
impl<'a> ValidatorNodeRpcClient for SemaphoreWrappedClient<'a> {
    type Addr = CommsPublicKey;
    type Error = ValidatorNodeClientError;

    async fn submit_transaction(&mut self, transaction: Transaction) -> Result<PayloadId, Self::Error> {
        self.client.submit_transaction(transaction).await.map_err(Into::into)
    }

    async fn get_finalized_transaction_result(
        &mut self,
        payload_id: PayloadId,
    ) -> Result<TransactionResultStatus, Self::Error> {
        self.client
            .get_finalized_transaction_result(payload_id)
            .await
            .map_err(Into::into)
    }

    async fn get_peers(&mut self) -> Result<Vec<DanPeer<Self::Addr>>, Self::Error> {
        self.client.get_peers().await.map_err(Into::into)
    }

    async fn get_substate(&mut self, address: &SubstateAddress, version: u32) -> Result<SubstateResult, Self::Error> {
        self.client.get_substate(address, version).await.map_err(Into::into)
    }
}

#[async_trait]
impl ValidatorNodeClientFactory for TariCommsValidatorNodeClientFactory {
    type Addr = PublicKey;
    type Client<'a> = SemaphoreWrappedClient<'a> where Self:'a;

    async fn create_client<'b: 'a, 'a>(&'b self, address: &Self::Addr) -> Self::Client<'a> {
        let permit = self.limit.acquire().await.expect("TODO: Handle this error");
        SemaphoreWrappedClient {
            permit,
            client: TariCommsValidatorNodeRpcClient {
                connectivity: self.connectivity.clone(),
                address: address.clone(),
                connection: None,
            },
        }
    }
}
