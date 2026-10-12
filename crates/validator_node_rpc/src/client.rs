//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    convert::TryInto,
    future::Future,
    sync::Arc,
    time::Duration,
};

use anyhow::anyhow;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tari_bor::decode_with_max_depth;
use tari_consensus_types::Decision;
use tari_engine_types::{
    commit_result::ExecuteResult,
    limits::MAX_CBOR_NESTING_DEPTH,
    substate::{Substate, SubstateId, SubstateValue},
};
use tari_networking::{MessageSpec, NetworkingHandle, PeerId};
use tari_ootle_common_types::{NodeAddressable, SubstateRequirementRef, SubstateVersion};
use tari_ootle_p2p::{
    TariMessagingSpec,
    ToPeerId,
    proto,
    proto::rpc::{
        GetSubstateResponse,
        GetTransactionResultRequest,
        PayloadResultStatus,
        SubmitTransactionRequest,
        SubstateStatus,
        get_substates_batch_response as batch_response,
    },
};
use tari_ootle_storage::time::{PrimitiveDateTime, UtcDateTime};
use tari_ootle_transaction::{Transaction, TransactionId};
use tokio::{sync::RwLock, time};

use crate::{ValidatorNodeRpcClientError, rpc_service};

pub trait ValidatorNodeClientFactory<TAddr: NodeAddressable>: Send + Sync {
    type Client: ValidatorNodeRpcClient<TAddr>;

    fn create_client(&self, address: &TAddr) -> Self::Client;
}

pub trait ValidatorNodeRpcClient<TAddr: NodeAddressable>: Send + Sync {
    fn submit_transaction(
        &mut self,
        transaction: Transaction,
    ) -> impl Future<Output = Result<TransactionId, ValidatorNodeRpcClientError>> + Send;
    fn get_finalized_transaction_result(
        &mut self,
        transaction_id: TransactionId,
    ) -> impl Future<Output = Result<TransactionResultStatus, ValidatorNodeRpcClientError>> + Send;

    fn get_substate(
        &mut self,
        substate_req: SubstateRequirementRef<'_>,
    ) -> impl Future<Output = Result<SubstateResult, ValidatorNodeRpcClientError>> + Send;

    /// Like [`get_substate`](Self::get_substate) but additionally requests a verifiable proof. The
    /// raw proof bytes are returned for the caller (which has the committee) to verify; `None` if the
    /// responder did not include one.
    fn get_substate_with_proof(
        &mut self,
        substate_req: SubstateRequirementRef<'_>,
    ) -> impl Future<Output = Result<(SubstateResult, Option<SubstateProofData>), ValidatorNodeRpcClientError>> + Send;

    /// Fetches the head version of many substates in one round trip. With `include_proofs` the
    /// responder anchors the batch with a single commit proof and proves each result against it.
    fn get_substates_batch(
        &mut self,
        substate_ids: &[&SubstateId],
        include_proofs: bool,
    ) -> impl Future<Output = Result<SubstateBatch, ValidatorNodeRpcClientError>> + Send;
}

/// The results of one [`get_substates_batch`](ValidatorNodeRpcClient::get_substates_batch) call.
#[derive(Debug, Clone, Default)]
pub struct SubstateBatch {
    /// CBOR-encoded CommittedBlockProof anchoring every value proof in the batch. `None` when proofs
    /// were not requested, or when the responder had no committed block to anchor against.
    pub commit_proof: Option<Vec<u8>>,
    pub substates: Vec<BatchedSubstate>,
    /// Ids this response does not answer for, whatever the reason: the responder holds no record of
    /// them, or holds one it cannot prove. Never evidence that a substate does not exist - use it to
    /// decide what to ask for again, never to conclude absence, which is not provable here at all: a
    /// leaf key is version-scoped, so an exclusion proof states that one version is not up, never
    /// that an id was never created.
    pub missing: Vec<SubstateId>,
}

#[derive(Debug, Clone)]
pub struct BatchedSubstate {
    pub substate_id: SubstateId,
    pub result: SubstateResult,
    /// CBOR-encoded SubstateValueProof, verified against [`SubstateBatch::commit_proof`]'s root.
    /// `None` when the batch carries no anchor.
    pub value_proof: Option<Vec<u8>>,
    /// Epoch the substate value hash was computed at; needed to re-derive the leaf value hash when
    /// verifying an inclusion proof.
    pub proof_epoch: u64,
    /// CBOR-encoded SubstateDownProof, decoded for wire compatibility. Validators leave it empty: a batch answers
    /// with heads, which a down proof cannot settle.
    pub substate_down_proof: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum TransactionResultStatus {
    Pending,
    Finalized(Box<FinalizedResult>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FinalizedResult {
    pub execute_result: Option<ExecuteResult>,
    pub final_decision: Decision,
    pub execution_time: Duration,
    pub finalized_time: PrimitiveDateTime,
    pub abort_details: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub enum SubstateResult {
    #[n(0)]
    DoesNotExist,
    #[n(1)]
    Up {
        #[n(0)]
        substate: Box<Substate>,
    },
    #[n(2)]
    Down {
        #[n(0)]
        version: SubstateVersion,
    },
}

impl SubstateResult {
    pub fn version(&self) -> Option<SubstateVersion> {
        match self {
            SubstateResult::Up { substate, .. } => Some(substate.version()),
            SubstateResult::Down { version, .. } => Some(*version),
            SubstateResult::DoesNotExist => None,
        }
    }

    pub fn up(&self) -> Option<&Substate> {
        match self {
            SubstateResult::Up { substate, .. } => Some(substate),
            _ => None,
        }
    }

    pub fn into_up(self) -> Option<Substate> {
        match self {
            SubstateResult::Up { substate } => Some(*substate),
            _ => None,
        }
    }
}

/// Raw (unverified) proof bytes accompanying a substate result, returned by a validator when a proof
/// was requested. Verification happens in the indexer, which has the shard group committee.
#[derive(Debug, Clone)]
pub struct SubstateProofData {
    /// CBOR-encoded SubstateValueProof for the substate's leaf within the shard-group state.
    pub substate_value_proof: Vec<u8>,
    /// CBOR-encoded CommittedBlockProof anchoring the trusted shard-group state merkle root.
    pub commit_proof: Vec<u8>,
    /// Epoch the substate value hash was computed at; needed to re-derive the leaf value hash when
    /// verifying an inclusion proof.
    pub proof_epoch: u64,
    /// CBOR-encoded SubstateDownProof proving a down result was committed before it went down. A down result is
    /// only proven by this: an exclusion proof alone is also satisfied by a version that never existed.
    pub substate_down_proof: Option<Vec<u8>>,
    /// The shard state version at which the responder recorded the substate's destruction, for a down result.
    pub destroyed_at_state_version: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TariValidatorNodeRpcClient<TAddr, TMsg: MessageSpec> {
    address: TAddr,
    pool: RpcMultiPool<TMsg>,
}

impl<TAddr: NodeAddressable + ToPeerId, TMsg: MessageSpec> TariValidatorNodeRpcClient<TAddr, TMsg> {
    pub fn new(address: TAddr, pool: RpcMultiPool<TMsg>) -> Self {
        Self { address, pool }
    }
}

impl<TAddr: ToPeerId, TMsg: MessageSpec> TariValidatorNodeRpcClient<TAddr, TMsg> {
    pub fn address(&self) -> &TAddr {
        &self.address
    }

    pub async fn client_connection(&self) -> Result<rpc_service::ValidatorNodeRpcClient, ValidatorNodeRpcClientError> {
        let client = self.pool.get_or_connect(&self.address.to_peer_id()).await?;
        Ok(client)
    }
}

impl<TAddr: NodeAddressable + ToPeerId, TMsg: MessageSpec> ValidatorNodeRpcClient<TAddr>
    for TariValidatorNodeRpcClient<TAddr, TMsg>
{
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
                    // A peer's bytes, decoded before any content validation runs over them.
                    .map(|r| decode_with_max_depth(&r, MAX_CBOR_NESTING_DEPTH))
                    .transpose()
                    .map_err(|_| {
                        ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                            "Node returned an invalid or empty execution result"
                        ))
                    })?;

                let execution_time = Duration::from_micros(response.execution_time_us);
                let finalized_time = UtcDateTime::from_unix_timestamp(response.finalized_timestamp).map_err(|e| {
                    ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                        "Node returned an invalid finalized timestamp: {e}"
                    ))
                })?;

                Ok(TransactionResultStatus::Finalized(Box::new(FinalizedResult {
                    execute_result: execution_result,
                    final_decision,
                    execution_time,
                    finalized_time: PrimitiveDateTime::new(finalized_time.date(), finalized_time.time()),
                    abort_details: Some(response.abort_details).filter(|s| !s.is_empty()),
                })))
            },
            Err(_) => Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                "Node returned invalid payload status {}",
                response.status
            ))),
        }
    }

    async fn get_substate(
        &mut self,
        substate_req: SubstateRequirementRef<'_>,
    ) -> Result<SubstateResult, ValidatorNodeRpcClientError> {
        let mut client = self.client_connection().await?;

        let request = proto::rpc::GetSubstateRequest {
            substate_requirement: Some(substate_req.into()),
            // Proof verification is wired in a follow-up; request the unverified value for now.
            include_proof: false,
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
                let substate = SubstateValue::from_bytes(&resp.substate)
                    .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!(e)))?;
                Ok(SubstateResult::Up {
                    substate: Box::new(Substate::new(SubstateVersion::new(resp.version), substate)),
                })
            },
            SubstateStatus::Down => Ok(SubstateResult::Down {
                version: SubstateVersion::new(resp.version),
            }),
            SubstateStatus::DoesNotExist => Ok(SubstateResult::DoesNotExist),
        }
    }

    async fn get_substate_with_proof(
        &mut self,
        substate_req: SubstateRequirementRef<'_>,
    ) -> Result<(SubstateResult, Option<SubstateProofData>), ValidatorNodeRpcClientError> {
        let mut client = self.client_connection().await?;

        let request = proto::rpc::GetSubstateRequest {
            substate_requirement: Some(substate_req.into()),
            include_proof: true,
        };

        let mut resp = client.get_substate(request).await?;
        let status = SubstateStatus::try_from(resp.status).map_err(|e| {
            ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                "Node returned invalid substate status {}: {e}",
                resp.status
            ))
        })?;

        let proof = decode_substate_proof(status, &mut resp);

        let result = match status {
            SubstateStatus::Up => {
                let substate = SubstateValue::from_bytes(&resp.substate)
                    .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!(e)))?;
                SubstateResult::Up {
                    substate: Box::new(Substate::new(SubstateVersion::new(resp.version), substate)),
                }
            },
            SubstateStatus::Down => SubstateResult::Down {
                version: SubstateVersion::new(resp.version),
            },
            SubstateStatus::DoesNotExist => SubstateResult::DoesNotExist,
        };

        Ok((result, proof))
    }

    async fn get_substates_batch(
        &mut self,
        substate_ids: &[&SubstateId],
        include_proofs: bool,
    ) -> Result<SubstateBatch, ValidatorNodeRpcClientError> {
        let mut conn = self.client_connection().await?;
        // NOTE: current maximum is 50 substates per request
        let mut stream = conn
            .get_substate_batch(proto::rpc::GetSubstatesBatchRequest {
                substate_ids: substate_ids.iter().map(|id| id.to_bytes()).collect(),
                include_proofs,
            })
            .await?;

        // For simplicity, we'll collect the stream instead of returning a decoded stream
        let requested = substate_ids.iter().copied().collect::<HashSet<_>>();
        let mut batch = SubstateBatch {
            substates: Vec::with_capacity(substate_ids.len()),
            ..Default::default()
        };
        while let Some(resp) = stream.next().await {
            let Some(response) = resp?.response else {
                continue;
            };
            match response {
                batch_response::Response::CommitProof(commit_proof) => {
                    batch.commit_proof = Some(commit_proof);
                },
                batch_response::Response::Substate(proven) => {
                    // A responder can only answer for what was asked. Bounding the stream by the
                    // request keeps a peer from growing this vec without limit, and dropping ids we
                    // did not ask for keeps them out of the caller's results.
                    if batch.substates.len() >= substate_ids.len() {
                        return Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                            "Node returned more substates than the {} requested",
                            substate_ids.len()
                        )));
                    }
                    let substate = decode_batched_substate(proven)?;
                    if !requested.contains(&substate.substate_id) {
                        return Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                            "Node returned {} which was not requested",
                            substate.substate_id
                        )));
                    }
                    batch.substates.push(substate);
                },
                batch_response::Response::Missing(missing) => {
                    for id in &missing.substate_ids {
                        batch.missing.push(
                            SubstateId::from_bytes(id)
                                .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!("{}", e)))?,
                        );
                    }
                },
            }
        }

        // Every requested id is either answered or named as missing. A responder that accounts for
        // none of them is not answering this protocol - most likely it predates the batch response
        // becoming a oneof, and its substates decoded as an unknown field. Failing here keeps that
        // skew from reading as "none of these substates exist".
        if batch.substates.is_empty() && batch.missing.is_empty() {
            return Err(ValidatorNodeRpcClientError::InvalidResponse(anyhow!(
                "Node accounted for none of the {} requested substates",
                substate_ids.len()
            )));
        }

        Ok(batch)
    }
}

/// The proof a single-substate response carries, taken out of `resp`. The responder omits the commit proof when it has
/// nothing committed to anchor against.
fn decode_substate_proof(status: SubstateStatus, resp: &mut GetSubstateResponse) -> Option<SubstateProofData> {
    if resp.commit_proof.is_empty() {
        return None;
    }
    Some(SubstateProofData {
        substate_value_proof: std::mem::take(&mut resp.substate_value_proof),
        commit_proof: std::mem::take(&mut resp.commit_proof),
        proof_epoch: resp.proof_epoch,
        substate_down_proof: Some(std::mem::take(&mut resp.substate_down_proof)).filter(|proof| !proof.is_empty()),
        destroyed_at_state_version: (status == SubstateStatus::Down).then_some(resp.destroyed_at_state_version),
    })
}

fn decode_batched_substate(proven: proto::rpc::ProvenSubstate) -> Result<BatchedSubstate, ValidatorNodeRpcClientError> {
    let substate = proven
        .substate
        .ok_or_else(|| ValidatorNodeRpcClientError::InvalidResponse(anyhow!("Batched substate has no substate")))?;
    let substate_id = SubstateId::from_bytes(&substate.substate_id)
        .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!("{}", e)))?;

    // A batch always answers with the head version, which is down when the substate's latest version
    // has been spent. Only an up substate carries a value.
    let result = if substate.destroyed.is_some() {
        SubstateResult::Down {
            version: SubstateVersion::new(substate.version),
        }
    } else {
        let value = SubstateValue::from_bytes(&substate.substate)
            .map_err(|e| ValidatorNodeRpcClientError::InvalidResponse(anyhow!("{}", e)))?;
        SubstateResult::Up {
            substate: Box::new(Substate::new(SubstateVersion::new(substate.version), value)),
        }
    };

    Ok(BatchedSubstate {
        substate_id,
        result,
        value_proof: Some(proven.substate_value_proof).filter(|proof| !proof.is_empty()),
        proof_epoch: proven.proof_epoch,
        substate_down_proof: Some(proven.substate_down_proof).filter(|proof| !proof.is_empty()),
    })
}

#[derive(Clone, Debug)]
pub struct TariValidatorNodeRpcClientFactory {
    pool: RpcMultiPool<TariMessagingSpec>,
}

impl TariValidatorNodeRpcClientFactory {
    pub fn new(networking: NetworkingHandle<TariMessagingSpec>) -> Self {
        Self {
            pool: RpcMultiPool::new(networking),
        }
    }
}

impl<TAddr: NodeAddressable + ToPeerId> ValidatorNodeClientFactory<TAddr> for TariValidatorNodeRpcClientFactory {
    type Client = TariValidatorNodeRpcClient<TAddr, TariMessagingSpec>;

    fn create_client(&self, address: &TAddr) -> Self::Client {
        TariValidatorNodeRpcClient {
            address: address.clone(),
            pool: self.pool.clone(),
        }
    }
}

/// How long a dial to a validator may take before it is treated as unreachable. The transport's own
/// failure modes are far slower - an unroutable host times out at the OS level, and the RPC handshake
/// allows 90s of its own - while a caller that gives up here still has the rest of the committee to
/// try.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// An RPC pool that holds a session for multiple validator nodes
#[derive(Debug, Clone)]
pub struct RpcMultiPool<TMsg: MessageSpec> {
    sessions: Arc<RwLock<HashMap<PeerId, rpc_service::ValidatorNodeRpcClient>>>,
    networking: NetworkingHandle<TMsg>,
}

impl<TMsg: MessageSpec> RpcMultiPool<TMsg> {
    pub fn new(networking: NetworkingHandle<TMsg>) -> Self {
        Self {
            sessions: Default::default(),
            networking,
        }
    }

    pub async fn get_or_connect(
        &self,
        addr: &PeerId,
    ) -> Result<rpc_service::ValidatorNodeRpcClient, ValidatorNodeRpcClientError> {
        if let Some(client) = self.connected_session(addr).await {
            return Ok(client);
        }

        // Dial without holding the lock. A peer that is unreachable takes as long to fail as its
        // transport allows, and the pool is shared by every caller: holding the lock across the dial
        // would queue all of them - including those bound for peers that are up - behind the one that
        // is down.
        let client: rpc_service::ValidatorNodeRpcClient =
            time::timeout(CONNECT_TIMEOUT, self.networking.connect_rpc(*addr))
                .await
                .map_err(|_| ValidatorNodeRpcClientError::ConnectTimeout {
                    peer_id: *addr,
                    timeout: CONNECT_TIMEOUT,
                })??;

        let mut sessions = self.sessions.write().await;
        // Another caller may have connected to the same peer while this dial was in flight. Keep
        // whichever session is already pooled so that concurrent callers converge on one.
        match sessions.get(addr) {
            Some(pooled) if pooled.is_connected() => Ok(pooled.clone()),
            _ => {
                sessions.insert(*addr, client.clone());
                Ok(client)
            },
        }
    }

    async fn connected_session(&self, addr: &PeerId) -> Option<rpc_service::ValidatorNodeRpcClient> {
        let sessions = self.sessions.read().await;
        sessions.get(addr).filter(|client| client.is_connected()).cloned()
    }
}

#[cfg(test)]
mod tests {
    use prost::Message;
    use tari_engine_types::substate::SubstateId;
    use tari_template_lib_types::{ComponentAddress, ObjectKey};

    use super::*;

    fn substate_id() -> SubstateId {
        SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([1; ObjectKey::LENGTH])))
    }

    /// The response as the client receives it, having crossed the wire.
    fn over_the_wire<M: Message + Default>(message: &M) -> M {
        M::decode(message.encode_to_vec().as_slice()).unwrap()
    }

    #[test]
    fn a_single_down_response_carries_its_down_proof() {
        let mut resp = over_the_wire(&GetSubstateResponse {
            status: SubstateStatus::Down as i32,
            version: 3,
            destroyed_at_state_version: 9,
            substate_value_proof: vec![1],
            commit_proof: vec![2],
            substate_down_proof: vec![3, 4],
            ..Default::default()
        });
        let proof = decode_substate_proof(SubstateStatus::Down, &mut resp).unwrap();
        assert_eq!(proof.substate_down_proof, Some(vec![3, 4]));
        assert_eq!(proof.destroyed_at_state_version, Some(9));
        assert_eq!(proof.commit_proof, vec![2]);
    }

    #[test]
    fn a_single_response_without_a_down_proof_has_none() {
        let mut resp = over_the_wire(&GetSubstateResponse {
            status: SubstateStatus::Up as i32,
            substate_value_proof: vec![1],
            commit_proof: vec![2],
            ..Default::default()
        });
        let proof = decode_substate_proof(SubstateStatus::Up, &mut resp).unwrap();
        assert_eq!(proof.substate_down_proof, None);
        assert_eq!(proof.destroyed_at_state_version, None);

        let mut unanchored = over_the_wire(&GetSubstateResponse {
            status: SubstateStatus::Down as i32,
            ..Default::default()
        });
        assert!(decode_substate_proof(SubstateStatus::Down, &mut unanchored).is_none());
    }

    #[test]
    fn a_batched_down_substate_carries_its_down_proof() {
        let destroyed = over_the_wire(&proto::rpc::ProvenSubstate {
            substate: Some(proto::consensus::Substate {
                substate_id: substate_id().to_bytes(),
                version: 2,
                substate: vec![],
                created: None,
                destroyed: Some(proto::consensus::SubstateDestroyedMetadata {
                    at_epoch: None,
                    at_state_version: 5,
                }),
            }),
            substate_value_proof: vec![1],
            proof_epoch: 0,
            substate_down_proof: vec![7, 8],
        });
        let batched = decode_batched_substate(destroyed).unwrap();
        assert!(matches!(batched.result, SubstateResult::Down { .. }));
        assert_eq!(batched.substate_down_proof, Some(vec![7, 8]));

        let without = over_the_wire(&proto::rpc::ProvenSubstate {
            substate: Some(proto::consensus::Substate {
                substate_id: substate_id().to_bytes(),
                version: 2,
                substate: vec![],
                created: None,
                destroyed: Some(proto::consensus::SubstateDestroyedMetadata {
                    at_epoch: None,
                    at_state_version: 5,
                }),
            }),
            substate_value_proof: vec![1],
            proof_epoch: 0,
            substate_down_proof: vec![],
        });
        assert_eq!(decode_batched_substate(without).unwrap().substate_down_proof, None);
    }
}
