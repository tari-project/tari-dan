//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::convert::{TryFrom, TryInto};

use anyhow::anyhow;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tari_bor::{decode, decode_exact, encode};
use tari_common_types::types::PublicKey;
use tari_comms::{
    connectivity::ConnectivityRequester,
    peer_manager::{NodeId, PeerIdentityClaim},
    protocol::rpc::RpcPoolClient,
    types::CommsPublicKey,
    PeerConnection,
};
use tari_crypto::tari_utilities::ByteArray;
use tari_dan_common_types::{NodeAddressable, ShardId};
use tari_dan_p2p::DanPeer;
use tari_dan_storage::consensus_models::Decision;
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateAddress, SubstateValue},
    virtual_substate::{VirtualSubstate, VirtualSubstateAddress},
};
use tari_transaction::{Transaction, TransactionId};
use tokio_stream::StreamExt;

use crate::{
    proto,
    proto::rpc::{
        GetPeersRequest,
        GetTransactionResultRequest,
        PayloadResultStatus,
        SubmitTransactionRequest,
        SubstateStatus,
    },
    rpc_service,
    ValidatorNodeRpcClientError,
};

pub trait ValidatorNodeClientFactory: Send + Sync {
    type Addr: NodeAddressable;
    type Client: ValidatorNodeRpcClient<Addr = Self::Addr>;

    fn create_client(&self, address: &Self::Addr) -> Self::Client;
}

#[async_trait]
pub trait ValidatorNodeRpcClient: Send + Sync {
    type Addr: NodeAddressable;
    type Error: std::error::Error + Send + Sync + 'static;

    async fn submit_transaction(&mut self, transaction: Transaction) -> Result<TransactionId, Self::Error>;
    async fn get_finalized_transaction_result(
        &mut self,
        transaction_id: TransactionId,
    ) -> Result<TransactionResultStatus, Self::Error>;

    async fn get_peers(&mut self) -> Result<Vec<DanPeer<Self::Addr>>, Self::Error>;

    async fn get_substate(&mut self, shard: ShardId) -> Result<SubstateResult, Self::Error>;
    async fn get_virtual_substate(&mut self, address: VirtualSubstateAddress) -> Result<VirtualSubstate, Self::Error>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum TransactionResultStatus {
    Pending,
    Finalized(FinalizedResult),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FinalizedResult {
    pub execute_result: Option<ExecuteResult>,
    pub final_decision: Decision,
    pub abort_details: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SubstateResult {
    DoesNotExist,
    Up {
        address: SubstateAddress,
        substate: Substate,
        created_by_tx: TransactionId,
    },
    Down {
        address: SubstateAddress,
        version: u32,
        created_by_tx: TransactionId,
        deleted_by_tx: TransactionId,
    },
}

pub struct TariCommsValidatorNodeRpcClient {
    connectivity: ConnectivityRequester,
    address: PublicKey,
    connection: Option<(PeerConnection, rpc_service::ValidatorNodeRpcClient)>,
}

impl TariCommsValidatorNodeRpcClient {
    pub async fn client_connection(
        &mut self,
    ) -> Result<rpc_service::ValidatorNodeRpcClient, ValidatorNodeRpcClientError> {
        if let Some((_, ref client)) = self.connection {
            if client.is_connected() {
                return Ok(client.clone());
            }
        }
        let mut conn = self
            .connectivity
            .dial_peer(NodeId::from_public_key(&self.address))
            .await?;
        let client: rpc_service::ValidatorNodeRpcClient = conn.connect_rpc().await?;
        self.connection = Some((conn, client.clone()));
        Ok(client)
    }
}

#[async_trait]
impl ValidatorNodeRpcClient for TariCommsValidatorNodeRpcClient {
    type Addr = CommsPublicKey;
    type Error = ValidatorNodeRpcClientError;

    async fn submit_transaction(
        &mut self,
        transaction: Transaction,
    ) -> Result<TransactionId, ValidatorNodeRpcClientError> {
        let mut client = self.client_connection().await?;
        let request = SubmitTransactionRequest {
            transaction: Some(transaction.into()),
        };
        let response = client.submit_transaction(request).await?;

        let id = response.transaction_id.try_into().map_err(|_| {
            ValidatorNodeRpcClientError::InvalidResponse(anyhow!("Node returned an invalid or empty transaction id"))
        })?;

        Ok(id)
    }

    async fn get_peers(&mut self) -> Result<Vec<DanPeer<Self::Addr>>, ValidatorNodeRpcClientError> {
        let mut client = self.client_connection().await?;
        // TODO(perf): This doesnt scale, find a nice way to wrap up the stream
        let peers = client
            .get_peers(GetPeersRequest { since: 0 })
            .await?
            .map(|result| {
                let p = result?;
                let claims = p
                    .claims
                    .into_iter()
                    .map(|a| PeerIdentityClaim::try_from(a).map_err(ValidatorNodeRpcClientError::InvalidResponse))
                    .collect::<Result<Vec<_>, _>>()?;
                Result::<_, ValidatorNodeRpcClientError>::Ok(DanPeer {
                    identity: ByteArray::from_bytes(&p.identity)
                        .map_err(|_| ValidatorNodeRpcClientError::InvalidResponse(anyhow!("Invalid identity")))?,
                    claims,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .await?;
        Ok(peers)
    }

    async fn get_substate(&mut self, shard: ShardId) -> Result<SubstateResult, Self::Error> {
        let mut client = self.client_connection().await?;

        let request = crate::proto::rpc::GetSubstateRequest {
            shard: shard.as_bytes().to_vec(),
        };

        let resp = client.get_substate(request).await?;
        let status = SubstateStatus::from_i32(resp.status).ok_or_else(|| {
            ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                "Node returned invalid substate status {}",
                resp.status
            ))
        })?;

        // TODO: verify the quorum certificates
        // for qc in resp.quorum_certificates {
        //     let qc = QuorumCertificate::try_from(&qc)?;
        // }

        match status {
            SubstateStatus::Up => {
                let tx_hash = resp.created_transaction_hash.try_into().map_err(|_| {
                    ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                        "Node returned an invalid or empty transaction hash"
                    ))
                })?;
                let substate = SubstateValue::from_bytes(&resp.substate)
                    .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!(e)))?;
                Ok(SubstateResult::Up {
                    substate: Substate::new(resp.version, substate),
                    address: SubstateAddress::from_bytes(&resp.address)
                        .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!(e)))?,
                    created_by_tx: tx_hash,
                })
            },
            SubstateStatus::Down => {
                let created_by_tx = resp.created_transaction_hash.try_into().map_err(|_| {
                    ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                        "Node returned an invalid or empty created transaction hash"
                    ))
                })?;
                let deleted_by_tx = resp.destroyed_transaction_hash.try_into().map_err(|_| {
                    ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                        "Node returned an invalid or empty destroyed transaction hash"
                    ))
                })?;
                Ok(SubstateResult::Down {
                    address: SubstateAddress::from_bytes(&resp.address)
                        .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!(e)))?,
                    version: resp.version,
                    deleted_by_tx,
                    created_by_tx,
                })
            },
            SubstateStatus::DoesNotExist => Ok(SubstateResult::DoesNotExist),
        }
    }

    async fn get_virtual_substate(&mut self, address: VirtualSubstateAddress) -> Result<VirtualSubstate, Self::Error> {
        let mut client = self.client_connection().await?;

        let request = proto::rpc::GetVirtualSubstateRequest {
            address: encode(&address)?,
        };

        let resp = client.get_virtual_substate(request).await?;

        // TODO: verify the quorum certificates
        // for qc in resp.quorum_certificates {
        //     let qc = QuorumCertificate::try_from(&qc)?;
        // }

        decode_exact(&resp.substate).map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!(e)))
    }

    async fn get_finalized_transaction_result(
        &mut self,
        transaction_id: TransactionId,
    ) -> Result<TransactionResultStatus, ValidatorNodeRpcClientError> {
        let mut client = self.client_connection().await?;
        let request = GetTransactionResultRequest {
            transaction_id: transaction_id.as_bytes().to_vec(),
        };
        let response = client.get_transaction_result(request).await?;

        match PayloadResultStatus::from_i32(response.status) {
            Some(PayloadResultStatus::Pending) => Ok(TransactionResultStatus::Pending),
            Some(PayloadResultStatus::Finalized) => {
                let proto_decision =
                    proto::consensus::Decision::from_i32(response.final_decision).ok_or_else(|| {
                        ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                            "Invalid decision value {}",
                            response.final_decision
                        ))
                    })?;
                let final_decision = proto_decision
                    .try_into()
                    .map_err(ValidatorNodeRpcClientError::InvalidResponse)?;
                let execution_result = Some(response.execution_result)
                    .filter(|r| !r.is_empty())
                    .map(|r| decode(&r))
                    .transpose()
                    .map_err(|_| {
                        ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                            "Node returned an invalid or empty execution result"
                        ))
                    })?;

                Ok(TransactionResultStatus::Finalized(FinalizedResult {
                    execute_result: execution_result,
                    final_decision,
                    abort_details: Some(response.abort_details).filter(|s| s.is_empty()),
                }))
            },
            None => Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                "Node returned invalid payload status {}",
                response.status
            ))),
        }
    }
}

#[derive(Clone, Debug)]
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
            connection: None,
        }
    }
}
