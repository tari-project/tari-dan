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
    decode_transaction_with_max_size,
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
        resolve_substate_down_proof,
    },
};
use tari_ootle_transaction::TransactionId;
use tari_rpc_framework::{Request, Response, RpcStatus, Streaming};
use tari_state_tree::SubstateValueProof;
use tari_validator_node_rpc::{STATE_SYNC_MAX_BATCH_SIZE, rpc_service::ValidatorNodeRpcService};
use tokio::{sync::mpsc, task};

use crate::{
    consensus::ConsensusHandle,
    p2p::{
        rpc::{
            CONSENSUS_NOT_RUNNING,
            block_sync_task::BlockSyncTask,
            state_sync_task::{HeldHistory, ShardCursor, StateSyncTask, TipAuthority, ensure_epoch_reached},
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
    max_transaction_size_bytes: usize,
}

impl<TStateStore: StateStore> ValidatorNodeRpcServiceImpl<TStateStore> {
    pub fn new(
        epoch_manager: EpochManagerHandle<PeerAddress>,
        state_store: TStateStore,
        mempool: MempoolHandle,
        consensus: ConsensusHandle,
        max_transaction_size_bytes: usize,
    ) -> Self {
        Self {
            epoch_manager,
            state_store,
            mempool,
            consensus,
            max_transaction_size_bytes,
        }
    }

    /// Resolves which shards' committed history through `epoch` this node holds.
    async fn held_history(&self, epoch: Epoch) -> Result<HeldHistory, RpcStatus> {
        let checkpoint_shard_groups = self
            .state_store
            .with_read_tx(|tx| EpochCheckpoint::get_all_for_epoch(tx, epoch))
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .into_iter()
            .map(|checkpoint| checkpoint.checked_shard_group())
            .collect::<Result<Vec<_>, _>>()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let next_epoch = epoch
            .checked_add(Epoch(1))
            .ok_or_else(|| RpcStatus::bad_request(format!("No epoch follows {epoch}")))?;
        let committed_as = self.local_shard_group_at(epoch).await?;
        let synced_as = self.local_shard_group_at(next_epoch).await?;
        Ok(HeldHistory {
            checkpoint_shard_groups,
            committed_as,
            synced_as,
        })
    }

    async fn local_shard_group_at(&self, epoch: Epoch) -> Result<Option<ShardGroup>, RpcStatus> {
        Ok(self
            .epoch_manager
            .get_local_committee_info(epoch)
            .await
            .optional()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
            .map(|info| info.shard_group()))
    }

    fn check_consensus_state(&self) -> Result<(), RpcStatus> {
        if self.consensus.can_serve_committed_state() {
            Ok(())
        } else {
            Err(RpcStatus::unavailable(CONSENSUS_NOT_RUNNING))
        }
    }

    /// Attaches a commit proof and a substate value proof for `substate` to `resp`, anchored to the
    /// latest committed block. Generated within the caller's read tx (the same one used for the
    /// substate lookup) so the value proof's group root matches the commit proof's block header. If
    /// nothing is committed beyond the epoch genesis yet, no proof is attached and the caller treats
    /// the result as unverified.
    ///
    /// A down substate is proven only together with a [`SubstateDownProof`](tari_state_tree::SubstateDownProof),
    /// since its exclusion proof alone is also satisfied by a version that never existed. A down substate this node
    /// recorded no such proof for is answered with no proof at all.
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
        attach_substate_proof_at(tx, num_preshards, &commit_proof, substate, resp)
    }
}

/// Attaches the proof of `substate` anchored at `commit_proof` to `resp`, or leaves every proof field empty when the
/// anchor cannot speak for the substate or the substate is down without a recorded down proof.
fn attach_substate_proof_at<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    num_preshards: NumPreshards,
    commit_proof: &CommittedBlockProof,
    substate: &SubstateRecord,
    resp: &mut GetSubstateResponse,
) -> Result<(), RpcStatus> {
    let shard_group = proof_shard_group(commit_proof).map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
    // The anchor cannot speak for this substate - its shard is outside the group the anchor
    // commits, or has nothing committed. Answer unproven rather than not at all.
    let Some(value_proof) = SubstateProofGenerator::new(
        tx,
        shard_group,
        num_preshards,
        commit_proof
            .protocol_version()
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?,
    )
    .and_then(|mut generator| generator.generate(&substate.to_versioned_substate_id()))
    .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
    else {
        return Ok(());
    };

    let down_proof = encode_down_proof(tx, num_preshards, substate, &value_proof)
        .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
    if substate.is_destroyed() && down_proof.is_none() {
        return Ok(());
    }

    resp.commit_proof = commit_proof.to_bytes();
    resp.substate_value_proof =
        tari_bor::serde_codec::to_vec(&value_proof).map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
    resp.proof_epoch = substate.created().at_epoch.as_u64();
    resp.substate_down_proof = down_proof.unwrap_or_default();
    Ok(())
}

/// The encoded down proof of `substate`, completed with `exclusion`, its exclusion proof under the root the response
/// is anchored to. `None` for a live substate, and for a down one this node recorded no proof of having been up, or
/// for which it no longer holds the commit proof of the version the record proves it up at.
fn encode_down_proof<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    num_preshards: NumPreshards,
    substate: &SubstateRecord,
    exclusion: &SubstateValueProof,
) -> Result<Option<Vec<u8>>, StorageError> {
    if !substate.is_destroyed() {
        return Ok(None);
    }
    let id = substate.to_versioned_substate_id();
    let shard = id.to_shard(num_preshards);
    let Some(record) = tx.substate_down_proofs_get(shard, &id)? else {
        return Ok(None);
    };
    let state_version = record.state_version;
    let Some(proof) = resolve_substate_down_proof(tx, shard, record, exclusion.clone())? else {
        warn!(
            target: LOG_TARGET,
            "The down proof of {id} relies on the commit proof of {shard} v{state_version}, which is not held",
        );
        return Ok(None);
    };
    let bytes = tari_bor::serde_codec::to_vec(&proof).map_err(|e| StorageError::QueryError {
        reason: format!("encode substate down proof: {e}"),
    })?;
    Ok(Some(bytes))
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
        generator = Some(SubstateProofGenerator::new(
            tx,
            shard_group,
            ctx.num_preshards,
            commit_proof.protocol_version().map_err(|e| StorageError::QueryError {
                reason: format!("commit proof: {e}"),
            })?,
        )?);
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

        messages.push(batch_response::Response::Substate(proven_substate(
            &substate,
            value_proof,
        )));
    }

    if !missing.is_empty() {
        debug!(target: LOG_TARGET, "{} requested substate(s) not answered", missing.len());
        messages.push(batch_response::Response::Missing(proto::rpc::MissingSubstates {
            substate_ids: missing,
        }));
    }

    Ok(messages)
}

/// A batch entry for `substate` with its value proof.
///
/// A batch answers with each substate's head, which a down proof cannot settle (it says nothing of later versions),
/// so a batch entry carries no down proof.
fn proven_substate(substate: &SubstateRecord, value_proof: Vec<u8>) -> proto::rpc::ProvenSubstate {
    proto::rpc::ProvenSubstate {
        proof_epoch: substate.created().at_epoch.as_u64(),
        substate_value_proof: value_proof,
        substate_down_proof: Vec::new(),
        substate: Some(proto::consensus::Substate {
            substate_id: substate.substate_id().to_bytes(),
            version: substate.version().as_u64(),
            substate: substate.substate_value().map(|v| v.to_bytes()).unwrap_or_default(),
            created: Some(substate.created().into()),
            destroyed: substate.destroyed().map(Into::into),
        }),
    }
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
        let transaction = request
            .transaction
            .ok_or_else(|| RpcStatus::bad_request("Missing transaction"))?;
        let transaction = decode_transaction_with_max_size(&transaction, self.max_transaction_size_bytes)
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
                version: substate.version().as_u64(),
                created_at_state_version: substate.created().at_state_version,
                destroyed_at_state_version: destroyed.at_state_version,
                ..Default::default()
            }
        } else {
            GetSubstateResponse {
                status: SubstateStatus::Up as i32,
                address: substate.substate_id().to_bytes(),
                version: substate.version().as_u64(),
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
            execution_time_us: u64::try_from(execution.execution_time().as_micros()).unwrap_or(u64::MAX),
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
            return Err(RpcStatus::unavailable("Node is still catching up to the epoch"));
        }
        let current_epoch = self
            .epoch_manager
            .current_epoch()
            .await
            .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
        let consensus_epoch = self.epoch_manager.get_current_epoch();
        if consensus_epoch != current_epoch {
            return Err(RpcStatus::unavailable(format!(
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
            return Err(RpcStatus::unavailable(format!(
                "Peer requested checkpoint with epoch {} but the current epoch is {}",
                from_epoch, consensus_epoch
            )));
        }

        if let Some(shard_group) = msg.shard_group {
            let shard_group = ShardGroup::decode_from_u32(shard_group)
                .ok_or_else(|| RpcStatus::bad_request(format!("Invalid shard group {shard_group}")))?;
            let checkpoint = self
                .state_store
                .with_read_tx(|tx| EpochCheckpoint::get_by_shard_group(tx, from_epoch, shard_group))
                .optional()
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?
                .ok_or_else(|| {
                    RpcStatus::not_found(format!(
                        "No checkpoint for epoch {from_epoch} shard group {shard_group}"
                    ))
                })?;
            return Ok(Response::new(GetCheckpointsResponse {
                checkpoints: vec![checkpoint.into()],
            }));
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
        let tip_authority = if let Some(end_epoch) = end_epoch {
            let current_epoch = self
                .epoch_manager
                .current_epoch()
                .await
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            ensure_epoch_reached(end_epoch, current_epoch)?;
            self.held_history(end_epoch)
                .await?
                .ensure_holds_all(&cursors, end_epoch)?;
            None
        } else {
            // Only a node participating in consensus is receiving the transitions it would claim to be
            // level on.
            if !self.consensus.is_running() {
                return Err(RpcStatus::unavailable(CONSENSUS_NOT_RUNNING));
            }
            let epoch = self.consensus.current_epoch();
            // A node that has not entered a view has no committee to answer for.
            if epoch.is_zero() {
                return Err(RpcStatus::unavailable("Consensus has not started on this node"));
            }
            let local_committee_info = self
                .epoch_manager
                .get_local_committee_info(epoch)
                .await
                .map_err(RpcStatus::log_internal_error(LOG_TARGET))?;
            ShardCursor::ensure_all_stored(&cursors, &local_committee_info)?;
            Some(TipAuthority::new(epoch, local_committee_info))
        };

        if req.include_version_proofs && end_epoch.is_none() {
            return Err(RpcStatus::bad_request(
                "State version proofs are only streamed for a request bounded by until_epoch",
            ));
        }

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
                req.include_version_proofs,
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

#[cfg(test)]
mod tests {
    use tari_engine_types::{
        non_fungible::NonFungibleContainer,
        substate::{SubstateId, SubstateValue, hash_substate},
    };
    use tari_ootle_common_types::{SubstateVersion, VersionedSubstateId};
    use tari_ootle_storage::{
        StateStoreWriteTransaction,
        consensus_models::{SubstateCreated, SubstateDestroyed, SubstateDownProofRecord},
    };
    use tari_ootle_transaction::Network;
    use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
    use tari_state_tree::{KeyedProofTree, LeafKey, SubstateDownProof, TreeHash};
    use tari_template_lib_types::{ComponentAddress, ObjectKey};

    use super::*;

    const NUM_PRESHARDS: NumPreshards = NumPreshards::P256;

    fn proof_ext(seed: u8) -> tari_state_tree::SparseMerkleProofExt {
        let key = LeafKey::new(TreeHash::new([seed; 32]));
        let tree = KeyedProofTree::build([(key, TreeHash::new([seed; 32]))]).unwrap();
        tree.get_proof(&key).unwrap().1
    }

    fn substate(destroyed: bool) -> SubstateRecord {
        let substate_id = SubstateId::Component(ComponentAddress::new(ObjectKey::from_array([3; ObjectKey::LENGTH])));
        let version = SubstateVersion::ZERO;
        let value = SubstateValue::NonFungible(NonFungibleContainer::no_data());
        SubstateRecord {
            state_hash: hash_substate(Network::LocalNet, &value, version, Epoch(1)),
            created: SubstateCreated {
                at_epoch: Epoch(1),
                in_shard: VersionedSubstateId::new(substate_id.clone(), version).to_shard(NUM_PRESHARDS),
                at_state_version: 1,
            },
            destroyed: destroyed.then_some(SubstateDestroyed {
                at_epoch: Epoch(1),
                at_state_version: 2,
            }),
            substate_id,
            version,
            substate_value: Some(value),
        }
    }

    /// The block that committed the state version a test record proves its substate up at.
    const COMMITTING_BLOCK: BlockId = BlockId::zero();

    /// A proof that `shard` was at `state_version` in [`COMMITTING_BLOCK`].
    fn committed_version_proof<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        shard: tari_ootle_common_types::shard::Shard,
        state_version: u64,
    ) -> Result<(), StorageError> {
        tx.state_version_proofs_insert(&tari_ootle_storage::consensus_models::StateVersionProof {
            shard,
            state_version,
            source: tari_ootle_storage::consensus_models::StateVersionProofSource::Committed {
                block_id: COMMITTING_BLOCK,
            },
            shard_root_proof: proof_ext(3),
        })
    }

    fn exclusion() -> SubstateValueProof {
        SubstateValueProof::new(TreeHash::new([5; 32]), 2, proof_ext(5), proof_ext(6))
    }

    /// An anchor for `shard`'s group of one. The responder serves its own state under the anchor it is given, so the
    /// root the header names does not matter here.
    fn anchor_of(shard: tari_ootle_common_types::shard::Shard) -> CommittedBlockProof {
        let protocol_version = tari_engine_types::ProtocolVersion::at(Network::LocalNet, Epoch(1));
        CommittedBlockProof::new(tari_sidechain::SidechainBlockCommitProof {
            header: tari_sidechain::SidechainBlockHeader {
                network: Network::LocalNet.as_byte(),
                protocol_version: protocol_version.as_u32(),
                parent_id: Default::default(),
                justify_id: Default::default(),
                height: 4,
                epoch: 1,
                epoch_hash: Default::default(),
                shard_group: tari_sidechain::ShardGroup {
                    start: shard.as_u32(),
                    end_inclusive: shard.as_u32(),
                },
                proposed_by: Default::default(),
                state_merkle_root: Default::default(),
                command_merkle_root: Default::default(),
                transaction_merkle_root: None,
                signature: Default::default(),
                accumulated_data: Default::default(),
                metadata_hash: Default::default(),
            },
            proof_elements: vec![],
        })
    }

    /// An exclusion proof alone does not show a version ever existed, so a down substate with no recorded down proof
    /// is answered with no proof at all, while one with a record carries all three.
    #[test]
    fn a_down_substate_without_a_recorded_down_proof_is_served_unproven() {
        use tari_ootle_storage::ShardScopedTreeStoreWriter;
        use tari_state_tree::{SpreadPrefixStateTree, SubstateTreeChange};

        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::<String>::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();
        let down = substate(true);
        let id = down.to_versioned_substate_id();
        let shard = id.to_shard(NUM_PRESHARDS);
        store
            .with_write_tx(|tx| {
                let mut tree_store = ShardScopedTreeStoreWriter::new(tx, shard);
                SpreadPrefixStateTree::new(&mut tree_store)
                    .batch_put_substate_changes(None, 1, vec![SubstateTreeChange::Up {
                        id: id.clone(),
                        value_hash: *down.state_hash(),
                    }])
                    .unwrap();
                SpreadPrefixStateTree::new(&mut tree_store)
                    .batch_put_substate_changes(Some(1), 2, vec![SubstateTreeChange::Down { id: id.clone() }])
                    .unwrap();
                tx.state_tree_shard_versions_set(shard, 2)
            })
            .unwrap();

        {
            let tx = store.create_read_tx().unwrap();
            let mut resp = GetSubstateResponse::default();
            attach_substate_proof_at(&tx, NUM_PRESHARDS, &anchor_of(shard), &down, &mut resp).unwrap();
            assert!(resp.commit_proof.is_empty());
            assert!(resp.substate_value_proof.is_empty());
            assert!(resp.substate_down_proof.is_empty());
        }

        let record = SubstateDownProofRecord {
            state_version: 1,
            value_hash: TreeHash::new([1; 32]),
            leaf_proof: proof_ext(1),
            shard_root: TreeHash::new([2; 32]),
            shard_root_proof: proof_ext(2),
        };
        store
            .with_write_tx(|tx| {
                committed_version_proof(tx, shard, 1)?;
                tx.block_commit_proofs_insert(&COMMITTING_BLOCK, &[9, 9, 9])?;
                tx.substate_down_proofs_insert(shard, &id, &record)
            })
            .unwrap();
        let tx = store.create_read_tx().unwrap();
        let mut resp = GetSubstateResponse::default();
        attach_substate_proof_at(&tx, NUM_PRESHARDS, &anchor_of(shard), &down, &mut resp).unwrap();
        assert!(!resp.commit_proof.is_empty());
        assert!(!resp.substate_value_proof.is_empty());
        assert!(!resp.substate_down_proof.is_empty());
    }

    /// A batch answers with heads, which a down proof cannot settle, so a down substate's batch entry carries none,
    /// even where the single-substate path would serve one.
    #[test]
    fn a_batch_entry_carries_no_down_proof() {
        let entry = proven_substate(&substate(true), vec![1]);
        assert!(entry.substate_down_proof.is_empty());
        assert!(entry.substate.unwrap().destroyed.is_some());
    }

    #[test]
    fn a_recorded_down_substate_is_served_with_its_down_proof() {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::<String>::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();
        let down = substate(true);
        let id = down.to_versioned_substate_id();
        let record = SubstateDownProofRecord {
            state_version: 1,
            value_hash: TreeHash::new([1; 32]),
            leaf_proof: proof_ext(1),
            shard_root: TreeHash::new([2; 32]),
            shard_root_proof: proof_ext(2),
        };

        {
            let tx = store.create_read_tx().unwrap();
            assert!(
                encode_down_proof(&tx, NUM_PRESHARDS, &down, &exclusion())
                    .unwrap()
                    .is_none()
            );
        }
        store
            .with_write_tx(|tx| tx.substate_down_proofs_insert(id.to_shard(NUM_PRESHARDS), &id, &record))
            .unwrap();
        // A record whose version's commit proof is not held cannot be served: first no state version proof, then one
        // citing a committed block whose commit proof is not stored.
        for setup in [false, true] {
            if setup {
                store
                    .with_write_tx(|tx| committed_version_proof(tx, id.to_shard(NUM_PRESHARDS), 1))
                    .unwrap();
            }
            let tx = store.create_read_tx().unwrap();
            assert!(
                encode_down_proof(&tx, NUM_PRESHARDS, &down, &exclusion())
                    .unwrap()
                    .is_none()
            );
        }
        store
            .with_write_tx(|tx| tx.block_commit_proofs_insert(&COMMITTING_BLOCK, &[9, 9, 9]))
            .unwrap();

        let tx = store.create_read_tx().unwrap();
        let bytes = encode_down_proof(&tx, NUM_PRESHARDS, &down, &exclusion())
            .unwrap()
            .unwrap();
        let proof: SubstateDownProof = tari_bor::serde_codec::from_slice(&bytes).unwrap();
        assert_eq!(proof.up_commit_proof, vec![9, 9, 9]);
        assert_eq!(proof.up_value_hash, TreeHash::new([1; 32]));
        assert_eq!(proof.up.shard_state_version, 1);
        assert_eq!(proof.down.shard_root, TreeHash::new([5; 32]));
        assert_eq!(proof.down.shard_state_version, 2);

        // A live substate is never served with a down proof.
        assert!(
            encode_down_proof(&tx, NUM_PRESHARDS, &substate(false), &exclusion())
                .unwrap()
                .is_none()
        );
    }
}
