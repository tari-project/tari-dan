//  Copyright 2021, The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that
// the  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the
// following  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED
// WARRANTIES,  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
// PARTICULAR PURPOSE ARE  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL,  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
// PROCUREMENT OF SUBSTITUTE GOODS OR  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
// CAUSED AND ON ANY THEORY OF LIABILITY,  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR
// OTHERWISE) ARISING IN ANY WAY OUT OF THE  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH
// DAMAGE.
use std::{
    convert::{TryFrom, TryInto},
    num::NonZeroUsize,
};

use log::*;
use tari_bor::encode;
use tari_consensus::hotstuff::commit_proofs::generate_block_commit_proof;
use tari_consensus_types::{BlockId, HighPc, ProposalCertificate};
use tari_engine_types::substate::SubstateId;
use tari_epoch_manager::{EpochManagerReader, service::EpochManagerHandle};
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    NumPreshards,
    ShardGroup,
    SubstateRequirement,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_p2p::{
    PeerAddress,
    proto,
    proto::rpc::{
        ConsensusState as ProtoConsensusState,
        GetCheckpointsRequest,
        GetCheckpointsResponse,
        GetCommittedBlockProofRequest,
        GetCommittedBlockProofResponse,
        GetConsensusStateRequest,
        GetConsensusStateResponse,
        GetHighQcRequest,
        GetHighQcResponse,
        GetSubstateRequest,
        GetSubstateResponse,
        GetSubstatesBatchRequest,
        GetSubstatesBatchResponse,
        GetTransactionResultRequest,
        GetTransactionResultResponse,
        PayloadResultStatus,
        SubstateStatus,
        SyncBlocksRequest,
        SyncBlocksResponse,
        SyncStateRequest,
        SyncStateResponse,
        get_substates_batch_response as batch_response,
    },
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StorageError,
    SubstateProofGenerator,
    consensus_models::{
        Block,
        BookkeepingEpochAgnosticRead,
        CommittedBlockProof,
        EpochCheckpoint,
        SubstateRecord,
        SubstateValueFilterFlags,
        TransactionRecord,
    },
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_rpc_framework::{Request, Response, RpcStatus, Streaming};
use tari_validator_node_rpc::{STATE_SYNC_MAX_BATCH_SIZE, rpc_service::ValidatorNodeRpcService};
use tokio::{sync::mpsc, task};

use crate::{
    consensus::ConsensusHandle,
    p2p::{
        rpc::{
            CONSENSUS_NOT_RUNNING,
            block_sync_task::BlockSyncTask,
            state_sync_task::{ShardCursor, StateSyncTask, TipAuthority},
        },
        services::mempool::MempoolHandle,
    },
};

const LOG_TARGET: &str = "tari::ootle::p2p::rpc";

pub struct ValidatorNodeRpcServiceImpl<TStateStore> {
    epoch_manager: EpochManagerHandle<PeerAddress>,
    state_store: TStateStore,
    mempool: MempoolHandle,
    consensus: ConsensusHandle,
}

impl<TStateStore: StateStore> ValidatorNodeRpcServiceImpl<TStateStore> {
    pub fn new(
        epoch_manager: EpochManagerHandle<PeerAddress>,
        state_store: TStateStore,
        mempool: MempoolHandle,
        consensus: ConsensusHandle,
    ) -> Self {
        Self {
            epoch_manager,
            state_store,
            mempool,
            consensus,
        }
    }

    fn check_consensus_state(&self) -> Result<(), RpcStatus> {
        if self.consensus.can_serve_committed_state() {
            Ok(())
        } else {
            Err(RpcStatus::general(CONSENSUS_NOT_RUNNING))
        }
    }

    /// Attaches a commit proof and a substate value proof for `substate` to `resp`, anchored to the
    /// latest committed block. Generated within the caller's read tx (the same one used for the
    /// substate lookup) so the value proof's group root matches the commit proof's block header. If
    /// nothing is committed beyond the epoch genesis yet, no proof is attached and the caller treats
    /// the result as unverified.
    fn attach_substate_proof<TTx: StateStoreReadTransaction>(
        &self,
        tx: &TTx,
        num_preshards: NumPreshards,
        substate: &SubstateRecord,
        resp: &mut GetSubstateResponse,
    ) -> Result<(), RpcStatus> {
        let epoch = self.consensus.current_epoch();
        let Some(commit_proof) = latest_commit_proof(tx, epoch).map_err(RpcStatus::log_internal_error(LOG_TARGET))?
        else {
            return Ok(());
        };

        let shard_group = proof_shard_group(&commit_proof).map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        // The anchor cannot speak for this substate - its shard is outside the group the anchor
        // commits, or has nothing committed. Answer unproven rather than not at all.
        let Some(value_proof) = SubstateProofGenerator::new(tx, shard_group, num_preshards)
            .and_then(|mut generator| generator.generate(&substate.to_versioned_substate_id()))
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
        else {
            return Ok(());
        };

        resp.commit_proof = commit_proof.to_bytes();
        resp.substate_value_proof =
            tari_bor::serde_codec::to_vec(&value_proof).map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        resp.proof_epoch = substate.created().at_epoch.as_u64();
        Ok(())
    }
}

/// The shard group a commit proof's block header commits the state merkle root of.
///
/// Value proofs must be built against this group rather than one read from an epoch lookup: the two
/// disagree across an epoch boundary at which the committee count changed, and level-2 proofs shaped
/// by the wrong group verify against a root the anchor does not commit. Taking it from the anchor
/// makes the pair consistent by construction.
fn proof_shard_group(commit_proof: &CommittedBlockProof) -> Result<ShardGroup, StorageError> {
    commit_proof.shard_group().map_err(|e| StorageError::QueryError {
        reason: format!("commit proof has an invalid shard group: {e}"),
    })
}

/// What the responder needs to prove a batch of substates against its own committed state.
struct BatchProofContext {
    epoch: Epoch,
    num_preshards: NumPreshards,
    include_proofs: bool,
}

/// Reads the head version of each of `ids` and, when proofs were asked for, a single anchor followed
/// by a value proof per substate.
///
/// Everything is read in one transaction, so every value proof verifies against the anchor's
/// shard-group root. The anchor comes first so a client can establish that root before it has to
/// verify anything against it; `missing` comes last, once the ids that were found are known.
///
/// A substate whose shard has no committed state cannot be proved against the anchor. It is reported
/// as missing rather than failing the batch, so the ids that can be answered still are; the client
/// refetches whatever it did not get.
fn read_substate_batch<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    ids: &[SubstateId],
    ctx: BatchProofContext,
) -> Result<Vec<batch_response::Response>, StorageError> {
    let (substates, missing) = SubstateRecord::get_any_max_version(tx, ids)?;
    let mut missing = missing.into_iter().map(|id| id.to_bytes()).collect::<Vec<_>>();

    let mut generator = None;
    let mut messages = Vec::with_capacity(substates.len() + 2);
    if ctx.include_proofs &&
        let Some(commit_proof) = latest_commit_proof(tx, ctx.epoch)?
    {
        let shard_group = proof_shard_group(&commit_proof)?;
        messages.push(batch_response::Response::CommitProof(commit_proof.to_bytes()));
        generator = Some(SubstateProofGenerator::new(tx, shard_group, ctx.num_preshards)?);
    }

    for substate in substates {
        let mut value_proof = Vec::new();
        if let Some(generator) = generator.as_mut() {
            let Some(proof) = generator.generate(&substate.to_versioned_substate_id())? else {
                warn!(
                    target: LOG_TARGET,
                    "{} is stored but cannot be proved against the anchor; reporting it as missing",
                    substate.substate_id()
                );
                missing.push(substate.substate_id().to_bytes());
                continue;
            };
            value_proof = tari_bor::serde_codec::to_vec(&proof).map_err(|e| StorageError::QueryError {
                reason: format!("encode substate value proof: {e}"),
            })?;
        }

        messages.push(batch_response::Response::Substate(proto::rpc::ProvenSubstate {
            proof_epoch: substate.created().at_epoch.as_u64(),
            substate_value_proof: value_proof,
            substate: Some(proto::consensus::Substate {
                substate_id: substate.substate_id().to_bytes(),
                version: substate.version(),
                substate: substate.substate_value().map(|v| v.to_bytes()).unwrap_or_default(),
                created: Some(substate.created().into()),
                destroyed: substate.destroyed().map(Into::into),
            }),
        }));
    }

    if !missing.is_empty() {
        debug!(target: LOG_TARGET, "{} requested substate(s) not answered", missing.len());
        messages.push(batch_response::Response::Missing(proto::rpc::MissingSubstates {
            substate_ids: missing,
        }));
    }

    Ok(messages)
}

/// The commit proof for the latest committed block: the quorum-signed anchor for the shard-group
/// state merkle root that substate value proofs generated in the same read transaction verify
/// against. `None` when nothing is committed beyond the epoch genesis yet, in which case results go
/// out unproven and the caller treats them as unverified.
fn latest_commit_proof<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    epoch: Epoch,
) -> Result<Option<CommittedBlockProof>, StorageError> {
    // `last_executed_get` reports both "nothing committed here" cases as `NotFound`: a node that has
    // never committed anything has no row, and one whose row belongs to another epoch is either in
    // the window after an epoch change or has not started consensus since restarting. Neither is a
    // failure to read; both mean there is nothing to anchor to.
    let Some(last_executed) = tx.last_executed_get(epoch).optional()? else {
        return Ok(None);
    };
    if last_executed.height.is_zero() {
        return Ok(None);
    }

    let block = Block::get(tx, &last_executed.block_id)?;
    let commit_qc = block.get_commit_qc(tx)?;
    let proof = generate_block_commit_proof(tx, &commit_qc, &block).map_err(|e| StorageError::QueryError {
        reason: format!("generate_block_commit_proof: {e}"),
    })?;
    Ok(Some(CommittedBlockProof::new(proof)))
}

#[tari_rpc_framework::async_trait]
impl<TStateStore: StateStore + Clone + Send + Sync + 'static> ValidatorNodeRpcService
    for ValidatorNodeRpcServiceImpl<TStateStore>
{
    async fn submit_transaction(
        &self,
        request: Request<proto::rpc::SubmitTransactionRequest>,
    ) -> Result<Response<proto::rpc::SubmitTransactionResponse>, RpcStatus> {
        let request = request.into_message();
        let transaction: Transaction = request
            .transaction
            .ok_or_else(|| RpcStatus::bad_request("Missing transaction"))?
            .try_into()
            .map_err(|e| RpcStatus::bad_request(format!("Malformed transaction: {}", e)))?;

        let transaction_id = transaction.calculate_id();
        info!(target: LOG_TARGET, "🌐 Received transaction {transaction_id} from peer");

        self.mempool
            .submit_transaction(transaction)
            .await
            .map_err(|e| RpcStatus::bad_request(format!("Invalid transaction: {}", e)))?;

        debug!(target: LOG_TARGET, "Accepted transaction {transaction_id} into mempool");

        Ok(Response::new(proto::rpc::SubmitTransactionResponse {
            transaction_id: transaction_id.as_bytes().to_vec(),
        }))
    }

    async fn get_substate(&self, req: Request<GetSubstateRequest>) -> Result<Response<GetSubstateResponse>, RpcStatus> {
        let req = req.into_message();

        let substate_requirement = req
            .substate_requirement
            .map(SubstateRequirement::try_from)
            .transpose()
            .map_err(|e| RpcStatus::bad_request(format!("Invalid substate requirement: {e}")))?
            .ok_or_else(|| RpcStatus::bad_request("Missing substate requirement"))?;

        // We need our local committee info to (a) confirm we store a non-global substate and (b) know
        // the preshard count when generating a proof. The shard group a proof is built against comes
        // from the anchor rather than from here, so this is only ever a question about the node.
        let local_committee_info = if !substate_requirement.substate_id().is_global() || req.include_proof {
            let current_epoch = self
                .epoch_manager
                .current_epoch()
                .await
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            let info = self
                .epoch_manager
                .get_local_committee_info(current_epoch)
                .await
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            if !substate_requirement.substate_id().is_global() &&
                !info.includes_substate_id(substate_requirement.substate_id())
            {
                return Err(RpcStatus::bad_request(format!(
                    "This node in {} does not store {}",
                    info.shard_group(),
                    substate_requirement
                )));
            }
            Some(info)
        } else {
            None
        };

        debug!(
            target: LOG_TARGET,
            "Querying substate {substate_requirement} from the state store"
        );
        let tx = self
            .state_store
            .create_read_tx()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let maybe_substate = substate_requirement
            .to_substate_address()
            .map(|address| SubstateRecord::get(&tx, &address))
            // Just fetch the latest if no version is supplied as a requirement
            .unwrap_or_else(|| SubstateRecord::get_latest(&tx, substate_requirement.substate_id()))
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let Some(substate) = maybe_substate else {
            return Ok(Response::new(GetSubstateResponse {
                status: SubstateStatus::DoesNotExist as i32,
                ..Default::default()
            }));
        };

        let mut resp = if let Some(destroyed) = substate.destroyed() {
            GetSubstateResponse {
                status: SubstateStatus::Down as i32,
                address: substate.substate_id().to_bytes(),
                substate: vec![],
                version: substate.version(),
                created_at_state_version: substate.created().at_state_version,
                destroyed_at_state_version: destroyed.at_state_version,
                ..Default::default()
            }
        } else {
            GetSubstateResponse {
                status: SubstateStatus::Up as i32,
                address: substate.substate_id().to_bytes(),
                version: substate.version(),
                substate: substate
                    .substate_value()
                    .map(|v| v.to_bytes())
                    .ok_or_else(|| RpcStatus::general("NEVER HAPPEN: UP substate has no value"))?,
                created_at_state_version: substate.created().at_state_version,
                ..Default::default()
            }
        };

        if req.include_proof {
            let info = local_committee_info
                .as_ref()
                .expect("committee info is fetched whenever include_proof is set");
            self.attach_substate_proof(&tx, info.num_preshards(), &substate, &mut resp)?;
        }

        Ok(Response::new(resp))
    }

    async fn get_transaction_result(
        &self,
        req: Request<GetTransactionResultRequest>,
    ) -> Result<Response<GetTransactionResultResponse>, RpcStatus> {
        let req = req.into_message();
        let tx = self
            .state_store
            .create_read_tx()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let tx_id = TransactionId::try_from(req.transaction_id)
            .map_err(|_| RpcStatus::bad_request("Invalid transaction id"))?;
        let transaction = TransactionRecord::get(&tx, &tx_id)
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .ok_or_else(|| RpcStatus::not_found("Transaction not found"))?;

        let Some(execution) = transaction
            .get_finalized_execution(&tx)
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
        else {
            return Ok(Response::new(GetTransactionResultResponse {
                status: PayloadResultStatus::Pending.into(),
                ..Default::default()
            }));
        };

        let finalized_time = transaction
            .get_finalized_time(&tx)
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        Ok(Response::new(GetTransactionResultResponse {
            status: PayloadResultStatus::Finalized.into(),

            final_decision: Some(proto::consensus::Decision::from(execution.decision())),
            execution_time_ms: u64::try_from(execution.execution_time().as_millis()).unwrap_or(u64::MAX),
            finalized_timestamp: finalized_time
                .map(|t| t.assume_utc().unix_timestamp())
                .unwrap_or_default(),
            abort_details: execution.abort_reason().map(|r| r.to_string()).unwrap_or_default(),
            // For simplicity, we simply encode the whole result as a CBOR blob.
            execution_result: encode(execution.result()).map_err(RpcStatus::log_internal_error(LOG_TARGET))?,
        }))
    }

    async fn sync_blocks(
        &self,
        request: Request<SyncBlocksRequest>,
    ) -> Result<Streaming<SyncBlocksResponse>, RpcStatus> {
        self.check_consensus_state()?;
        let req = request.into_message();
        let store = self.state_store.clone();

        if proto::rpc::StreamSubstateSelection::try_from(req.stream_substates).is_err() {
            return Err(RpcStatus::bad_request("StreamSubstateSelection is invalid"));
        }

        let current_epoch = self
            .epoch_manager
            .current_epoch()
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let start_block_id = Some(req.start_block_id.as_slice())
            .filter(|i| !i.is_empty())
            .map(BlockId::try_from)
            .transpose()
            .map_err(|e| RpcStatus::bad_request(format!("Invalid encoded block id: {}", e)))?;

        let start_block_id = {
            let tx = store
                .create_read_tx()
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

            match start_block_id {
                Some(id) => {
                    if !Block::record_exists(&tx, &id).map_err(RpcStatus::log_internal_error(LOG_TARGET))? {
                        return Err(RpcStatus::not_found(format!("start_block_id {id} not found",)));
                    }
                    id
                },
                None => {
                    let epoch = req
                        .epoch
                        .map(Epoch::from)
                        .map(|end| end.min(current_epoch))
                        .unwrap_or(current_epoch);

                    let mut block_ids = Block::get_ids_by_epoch_and_height(&tx, epoch, NodeHeight::zero())
                        .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

                    let Some(block_id) = block_ids.pop() else {
                        return Err(RpcStatus::not_found(format!(
                            "Block not found with epoch={epoch},height=0"
                        )));
                    };
                    if !block_ids.is_empty() {
                        return Err(RpcStatus::conflict(format!(
                            "Multiple applicable blocks for epoch={} and height=0",
                            current_epoch
                        )));
                    }

                    block_id
                },
            }
        };

        let committee_info = self
            .epoch_manager
            .get_local_committee_info(current_epoch)
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        let (sender, receiver) = mpsc::channel(10);
        task::spawn(BlockSyncTask::new(store, start_block_id, None, sender, committee_info.num_preshards()).run(req));

        Ok(Streaming::new(receiver))
    }

    async fn get_checkpoints(
        &self,
        request: Request<GetCheckpointsRequest>,
    ) -> Result<Response<GetCheckpointsResponse>, RpcStatus> {
        let msg = request.into_message();
        if !self
            .epoch_manager
            .is_initial_scanning_complete()
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
        {
            return Err(RpcStatus::general("Node is still catching up to the epoch"));
        }
        let current_epoch = self
            .epoch_manager
            .current_epoch()
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let consensus_epoch = self.epoch_manager.get_current_epoch();
        if consensus_epoch != current_epoch {
            return Err(RpcStatus::general(format!(
                "Node is not in sync with the consensus epoch. Current epoch: {}, Consensus epoch: {}",
                current_epoch, consensus_epoch
            )));
        }
        let from_epoch = msg
            .from_epoch
            .ok_or_else(|| RpcStatus::bad_request("from_epoch is required"))?
            .into();

        if from_epoch >= consensus_epoch {
            // This may occur if one of the nodes has not fully scanned the base layer
            return Err(RpcStatus::bad_request(format!(
                "Peer requested checkpoint with epoch {} but the current epoch is {}",
                from_epoch, consensus_epoch
            )));
        }

        if msg.num_to_return > 100 {
            return Err(RpcStatus::bad_request("num_to_return must be less than 100"));
        }

        let limit = NonZeroUsize::new(msg.num_to_return as usize).ok_or_else(|| {
            RpcStatus::bad_request(format!(
                "Invalid number of checkpoints requested: {}. Must be a integer.",
                msg.num_to_return
            ))
        })?;

        let checkpoints = self
            .state_store
            .with_read_tx(|tx| EpochCheckpoint::get_all_from_epoch(tx, from_epoch, limit.get()))
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        Ok(Response::new(GetCheckpointsResponse {
            checkpoints: checkpoints.into_iter().map(Into::into).collect(),
        }))
    }

    async fn sync_state(&self, request: Request<SyncStateRequest>) -> Result<Streaming<SyncStateResponse>, RpcStatus> {
        self.check_consensus_state()?;
        let req = request.into_message();

        let (sender, receiver) = mpsc::channel(10);

        let cursors = ShardCursor::validate_all(req.cursors)?;

        let end_epoch = req.until_epoch.map(Epoch::from);

        // A bounded stream is answered out of history, which has no tip to follow.
        if req.follow && end_epoch.is_some() {
            return Err(RpcStatus::bad_request(
                "A bounded (until_epoch) request cannot follow the tip",
            ));
        }

        // An unbounded request asks for this node's tip, which only a member of the committee that
        // currently stores those shards can answer: a non-member receives no further transitions for
        // them and streams silence, which the caller cannot tell apart from being caught up. A bounded
        // request is answered out of history and the caller checks it against a quorum-signed
        // checkpoint, so any node still holding that history may serve it.
        //
        // Membership is resolved at the epoch consensus is in, which is what governs the transitions
        // this node receives, and can lag the epoch reached by scanning the base layer. The completion
        // marker names the epoch its claim is made as of, so the claim and the marker must be anchored
        // to the same one.
        let tip_authority = if end_epoch.is_none() {
            // A tip claim needs more than the history check that admitted the request: only a node
            // participating in consensus is receiving the transitions it would claim to be level on.
            if !self.consensus.is_running() {
                return Err(RpcStatus::general(CONSENSUS_NOT_RUNNING));
            }
            let epoch = self.consensus.current_epoch();
            // A node that has not entered a view has no committee to answer for.
            if epoch.is_zero() {
                return Err(RpcStatus::general("Consensus has not started on this node"));
            }
            let local_committee_info = self
                .epoch_manager
                .get_local_committee_info(epoch)
                .await
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            ShardCursor::ensure_all_stored(&cursors, &local_committee_info)?;
            Some(TipAuthority::new(epoch, local_committee_info))
        } else {
            None
        };

        let value_filter_flags = SubstateValueFilterFlags::from_bits_truncate(req.value_filters);
        if value_filter_flags.is_empty() {
            return Err(RpcStatus::bad_request(
                "At least one SubstateValueFilterFlag must be set",
            ));
        }

        debug!(
            target: LOG_TARGET,
            "🌍 peer initiated sync with this node for {} shard(s) ({} to {}) to {} (values: {:?}, follow: {})",
            cursors.len(),
            cursors.first().map(|c| c.shard).display(),
            cursors.last().map(|c| c.shard).display(),
            end_epoch.display(),
            value_filter_flags,
            req.follow,
        );

        task::spawn(
            StateSyncTask::new(
                self.state_store.clone(),
                sender,
                cursors,
                end_epoch,
                self.consensus.clone(),
                self.epoch_manager.clone(),
                tip_authority,
                req.follow,
                STATE_SYNC_MAX_BATCH_SIZE
                    .try_into()
                    .expect("STATE_SYNC_MAX_BATCH_SIZE is not zero"),
                value_filter_flags,
            )
            .run(),
        );

        Ok(Streaming::new(receiver))
    }

    async fn get_consensus_state(
        &self,
        _req: Request<GetConsensusStateRequest>,
    ) -> Result<Response<GetConsensusStateResponse>, RpcStatus> {
        let view = self.consensus.current_view();
        let epoch = self.consensus.current_epoch();
        let state: ProtoConsensusState = self.consensus.get_current_state().into();

        Ok(Response::new(GetConsensusStateResponse {
            epoch: Some(epoch.into()),
            height: view.get_height().as_u64(),
            state: state as i32,
        }))
    }

    async fn get_high_qc(&self, req: Request<GetHighQcRequest>) -> Result<Response<GetHighQcResponse>, RpcStatus> {
        let req = req.into_message();
        let from_epoch = req.from_epoch.map(Epoch::from).unwrap_or(Epoch::zero());

        let store = self.state_store.clone();
        let (high_pc, qc): (HighPc, ProposalCertificate) = task::spawn_blocking(move || {
            store
                .with_read_tx(|tx| {
                    let high_pc = HighPc::get_any(tx)?;
                    let qc = tx.proposal_certificates_get(high_pc.epoch(), high_pc.id())?;
                    Ok::<_, tari_ootle_storage::StorageError>((high_pc, qc))
                })
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))
        })
        .await
        .map_err(RpcStatus::log_internal_error(LOG_TARGET))??;

        // Reject if caller is genuinely ahead of us — no useful answer to give.
        if from_epoch > high_pc.epoch() {
            return Err(RpcStatus::not_found(format!(
                "Our high QC epoch {} is behind caller's leaf epoch {}",
                high_pc.epoch(),
                from_epoch
            )));
        }

        Ok(Response::new(GetHighQcResponse {
            high_qc: Some((&qc).into()),
        }))
    }

    async fn get_committed_block_proof(
        &self,
        _req: Request<GetCommittedBlockProofRequest>,
    ) -> Result<Response<GetCommittedBlockProofResponse>, RpcStatus> {
        let epoch = self.consensus.current_epoch();
        let store = self.state_store.clone();

        let maybe_proof = task::spawn_blocking(move || {
            store
                .with_read_tx(|tx| latest_commit_proof(tx, epoch))
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))
        })
        .await
        .map_err(RpcStatus::log_internal_error(LOG_TARGET))??;

        let proof = maybe_proof.ok_or_else(|| RpcStatus::not_found("No committed block beyond genesis yet"))?;

        Ok(Response::new(GetCommittedBlockProofResponse {
            commit_proof: proof.to_bytes(),
        }))
    }

    async fn get_substate_batch(
        &self,
        req: Request<GetSubstatesBatchRequest>,
    ) -> Result<Streaming<GetSubstatesBatchResponse>, RpcStatus> {
        // Proving a batch costs a fixed amount per request - reading every shard root in the group and
        // building the tree over them - plus a leaf traversal per substate. Measured on a 256-shard
        // group (see the `proof_cost` test), that is ~220us fixed against ~4us per substate, so a
        // request answering 50 costs ~450us where 50 single-substate requests cost ~11ms. A lower cap
        // would therefore make a flood *more* expensive to serve, not less; what 50 bounds is the
        // response, which the responder holds in memory before streaming.
        const MAX_REQUESTS: usize = 50;
        let req = req.into_message();

        if req.substate_ids.len() > MAX_REQUESTS {
            return Err(RpcStatus::bad_request("Cannot request more than 50 substates at once"));
        }
        if req.substate_ids.is_empty() {
            return Err(RpcStatus::bad_request("No substate ids requested"));
        }

        debug!(
            target: LOG_TARGET,
            "Querying {} substate(s) from the state store", req.substate_ids.len()
        );
        let ids = req
            .substate_ids
            .iter()
            .map(|x| SubstateId::from_bytes(x))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RpcStatus::bad_request(format!("Invalid substate ID: {e}")))?;

        // Which of the requested ids we are responsible for. This is a membership question about the
        // node, not about the state being proved - the shard group a proof is built against comes
        // from the anchor - so it is asked at the epoch manager's epoch, which is set before
        // consensus starts.
        let current_epoch = self
            .epoch_manager
            .current_epoch()
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let local_committee_info = self
            .epoch_manager
            .get_local_committee_info(current_epoch)
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        if let Some(id) = ids.iter().find(|id| !local_committee_info.includes_substate_id(id)) {
            return Err(RpcStatus::bad_request(format!(
                "This node in {} does not store {}",
                local_committee_info.shard_group(),
                id
            )));
        }

        let (sender, receiver) = mpsc::channel(req.substate_ids.len() + 2);

        let store = self.state_store.clone();
        // The epoch to look for a committed block at. Before consensus starts this is zero, which
        // finds nothing and yields no anchor, so results go out unproven rather than failing.
        let consensus_epoch = self.consensus.current_epoch();
        let num_preshards = local_committee_info.num_preshards();
        let include_proofs = req.include_proofs;
        let responses = task::spawn_blocking(move || {
            // TODO: we should use a snapshot - will need to refactor the state store to support this, by abstracting
            // the .cf(X) call and implementing read only transaction for all implementors of this trait
            store.with_read_tx(|tx| {
                read_substate_batch(tx, &ids, BatchProofContext {
                    epoch: consensus_epoch,
                    num_preshards,
                    include_proofs,
                })
            })
        })
        .await
        .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
        .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;

        task::spawn(async move {
            for resp in responses {
                if sender
                    .send(Ok(GetSubstatesBatchResponse { response: Some(resp) }))
                    .await
                    .is_err()
                {
                    warn!(target: LOG_TARGET, "Receiver dropped the stream, stopping substate batch response stream");
                    break;
                }
            }
        });

        Ok(Streaming::new(receiver))
    }
}
