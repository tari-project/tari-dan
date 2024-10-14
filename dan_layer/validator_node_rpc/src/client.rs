//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, convert::TryInto, sync::Arc, time::Duration};

use anyhow::anyhow;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tari_bor::{decode, decode_exact, encode};
use tari_dan_common_types::{NodeAddressable, PeerAddress, SubstateAddress};
use tari_dan_p2p::{
    proto,
    proto::rpc::{GetTransactionResultRequest, PayloadResultStatus, SubmitTransactionRequest, SubstateStatus},
    TariMessagingSpec,
};
use tari_dan_storage::consensus_models::Decision;
use tari_engine_types::{
    commit_result::ExecuteResult,
    substate::{Substate, SubstateId, SubstateValue},
    virtual_substate::{VirtualSubstate, VirtualSubstateId},
};
use tari_networking::{MessageSpec, NetworkingHandle};
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::RwLock;

use crate::{rpc_service, ValidatorNodeRpcClientError};

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

    async fn get_substate(&mut self, shard: SubstateAddress) -> Result<SubstateResult, Self::Error>;
    async fn get_virtual_substate(&mut self, address: VirtualSubstateId) -> Result<VirtualSubstate, Self::Error>;
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
    pub execution_time: Duration,
    pub finalized_time: Duration,
    pub abort_details: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum SubstateResult {
    DoesNotExist,
    Up {
        id: SubstateId,
        substate: Substate,
        created_by_tx: TransactionId,
    },
    Down {
        id: SubstateId,
        version: u32,
        created_by_tx: TransactionId,
        deleted_by_tx: TransactionId,
    },
}

pub struct TariValidatorNodeRpcClient<TMsg: MessageSpec> {
    address: PeerAddress,
    pool: RpcPool<TMsg>,
}

impl<TMsg: MessageSpec> TariValidatorNodeRpcClient<TMsg> {
    pub fn address(&self) -> &PeerAddress {
        &self.address
    }

    pub async fn client_connection(
        &mut self,
    ) -> Result<rpc_service::ValidatorNodeRpcClient, ValidatorNodeRpcClientError> {
        let client = self.pool.get_or_connect(&self.address).await?;
        Ok(client)
    }
}

#[async_trait]
impl<TMsg: MessageSpec> ValidatorNodeRpcClient for TariValidatorNodeRpcClient<TMsg> {
    type Addr = PeerAddress;
    type Error = ValidatorNodeRpcClientError;

    async fn submit_transaction(
        &mut self,
        transaction: Transaction,
    ) -> Result<TransactionId, ValidatorNodeRpcClientError> {
        let mut client = self.client_connection().await?;
        let request = SubmitTransactionRequest {
            transaction: Some((&transaction).into()),
        };
        let response = client.submit_transaction(request).await?;

        let id = response.transaction_id.try_into().map_err(|_| {
            ValidatorNodeRpcClientError::InvalidResponse(anyhow!("Node returned an invalid or empty transaction id"))
        })?;

        Ok(id)
    }

    async fn get_substate(&mut self, address: SubstateAddress) -> Result<SubstateResult, Self::Error> {
        let mut client = self.client_connection().await?;

        let request = proto::rpc::GetSubstateRequest {
            address: address.as_bytes().to_vec(),
        };

        let resp = client.get_substate(request).await?;
        let status = SubstateStatus::try_from(resp.status).map_err(|e| {
            ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                "Node returned invalid substate status {}: {e}",
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
                    id: SubstateId::from_bytes(&resp.address)
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
                    id: SubstateId::from_bytes(&resp.address)
                        .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!(e)))?,
                    version: resp.version,
                    deleted_by_tx,
                    created_by_tx,
                })
            },
            SubstateStatus::DoesNotExist => Ok(SubstateResult::DoesNotExist),
        }
    }

    async fn get_virtual_substate(&mut self, address: VirtualSubstateId) -> Result<VirtualSubstate, Self::Error> {
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

        match PayloadResultStatus::try_from(response.status) {
            Ok(PayloadResultStatus::Pending) => Ok(TransactionResultStatus::Pending),
            Ok(PayloadResultStatus::Finalized) => {
                let proto_decision = response
                    .final_decision
                    .ok_or(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                        "Missing decision!"
                    )))?;
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

                let execution_time = Duration::from_millis(response.execution_time_ms);
                let finalized_time = Duration::from_millis(response.finalized_time_ms);

                Ok(TransactionResultStatus::Finalized(FinalizedResult {
                    execute_result: execution_result,
                    final_decision,
                    execution_time,
                    finalized_time,
                    abort_details: Some(response.abort_details).filter(|s| s.is_empty()),
                }))
            },
            Err(_) => Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                "Node returned invalid payload status {}",
                response.status
            ))),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TariValidatorNodeRpcClientFactory {
    pool: RpcPool<TariMessagingSpec>,
}

impl TariValidatorNodeRpcClientFactory {
    pub fn new(networking: NetworkingHandle<TariMessagingSpec>) -> Self {
        Self {
            pool: RpcPool::new(networking),
        }
    }
}

impl ValidatorNodeClientFactory for TariValidatorNodeRpcClientFactory {
    type Addr = PeerAddress;
    type Client = TariValidatorNodeRpcClient<TariMessagingSpec>;

    fn create_client(&self, address: &Self::Addr) -> Self::Client {
        TariValidatorNodeRpcClient {
            address: *address,
            pool: self.pool.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RpcPool<TMsg: MessageSpec> {
    sessions: Arc<RwLock<HashMap<PeerAddress, rpc_service::ValidatorNodeRpcClient>>>,
    networking: NetworkingHandle<TMsg>,
}

impl<TMsg: MessageSpec> RpcPool<TMsg> {
    pub fn new(networking: NetworkingHandle<TMsg>) -> Self {
        Self {
            sessions: Default::default(),
            networking,
        }
    }

    async fn get_or_connect(
        &mut self,
        addr: &PeerAddress,
    ) -> Result<rpc_service::ValidatorNodeRpcClient, ValidatorNodeRpcClientError> {
        let mut sessions = self.sessions.write().await;
        if let Some(client) = sessions.get(addr) {
            if client.is_connected() {
                return Ok(client.clone());
            } else {
                sessions.remove(addr);
            }
        }

        let client: rpc_service::ValidatorNodeRpcClient = self.networking.connect_rpc(addr.as_peer_id()).await?;
        sessions.insert(*addr, client.clone());

        Ok(client)
    }
}
