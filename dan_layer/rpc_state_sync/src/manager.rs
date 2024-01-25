//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt::Display, ops::DerefMut};

use async_trait::async_trait;
use futures::StreamExt;
use log::*;
use tari_consensus::{
    hotstuff::ProposalValidationError,
    traits::{ConsensusSpec, LeaderStrategy, SyncManager, SyncStatus},
};
use tari_dan_common_types::{committee::Committee, optional::Optional, NodeHeight, PeerAddress};
use tari_dan_p2p::proto::rpc::{GetHighQcRequest, SyncBlocksRequest};
use tari_dan_storage::{
    consensus_models::{
        Block,
        BlockId,
        HighQc,
        LockedBlock,
        QuorumCertificate,
        SubstateUpdate,
        TransactionPoolRecord,
        TransactionRecord,
    },
    StateStore,
    StateStoreWriteTransaction,
};
use tari_epoch_manager::EpochManagerReader;
use tari_rpc_framework::RpcError;
use tari_transaction::Transaction;
use tari_validator_node_rpc::{
    client::{TariValidatorNodeRpcClientFactory, ValidatorNodeClientFactory},
    rpc_service::ValidatorNodeRpcClient,
};

use crate::error::CommsRpcConsensusSyncError;

const LOG_TARGET: &str = "tari::dan::comms_rpc_state_sync";

const MAX_SUBSTATE_UPDATES: usize = 10000;

pub struct RpcStateSyncManager<TConsensusSpec: ConsensusSpec> {
    epoch_manager: TConsensusSpec::EpochManager,
    state_store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    client_factory: TariValidatorNodeRpcClientFactory,
}

impl<TConsensusSpec> RpcStateSyncManager<TConsensusSpec>
where TConsensusSpec: ConsensusSpec<Addr = PeerAddress>
{
    pub fn new(
        epoch_manager: TConsensusSpec::EpochManager,
        state_store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        client_factory: TariValidatorNodeRpcClientFactory,
    ) -> Self {
        Self {
            epoch_manager,
            state_store,
            leader_strategy,
            client_factory,
        }
    }

    async fn get_sync_peers(&self) -> Result<Committee<TConsensusSpec::Addr>, CommsRpcConsensusSyncError> {
        let current_epoch = self.epoch_manager.current_epoch().await?;
        let this_vn = self.epoch_manager.get_our_validator_node(current_epoch).await?;
        let mut committee = self.epoch_manager.get_local_committee(current_epoch).await?;
        committee.members.retain(|(addr, _)| *addr != this_vn.address);
        committee.shuffle();
        Ok(committee)
    }

    async fn sync_with_peer(
        &self,
        addr: &TConsensusSpec::Addr,
        locked_block: &LockedBlock,
    ) -> Result<(), CommsRpcConsensusSyncError> {
        self.create_zero_block_if_required()?;
        let mut rpc_client = self.client_factory.create_client(addr);
        let mut client = rpc_client.client_connection().await?;

        info!(target: LOG_TARGET, "🌐 Syncing blocks from peer '{}' from Locked block {}", addr, locked_block);
        self.sync_blocks(&mut client, locked_block).await?;

        Ok(())
    }

    fn create_zero_block_if_required(&self) -> Result<(), CommsRpcConsensusSyncError> {
        let mut tx = self.state_store.create_write_tx()?;

        let zero_block = Block::zero_block();
        if !zero_block.exists(tx.deref_mut())? {
            debug!(target: LOG_TARGET, "Creating zero block");
            zero_block.justify().insert(&mut tx)?;
            zero_block.insert(&mut tx)?;
            zero_block.as_locked_block().set(&mut tx)?;
            zero_block.as_leaf_block().set(&mut tx)?;
            zero_block.as_last_executed().set(&mut tx)?;
            zero_block.as_last_voted().set(&mut tx)?;
            zero_block.justify().as_high_qc().set(&mut tx)?;
            zero_block.commit(&mut tx)?;
        }

        tx.commit()?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn sync_blocks(
        &self,
        client: &mut ValidatorNodeRpcClient,
        locked_block: &LockedBlock,
    ) -> Result<(), CommsRpcConsensusSyncError> {
        let mut stream = client
            .sync_blocks(SyncBlocksRequest {
                start_block_id: locked_block.block_id.as_bytes().to_vec(),
            })
            .await?;

        let mut counter = 0usize;

        let mut expected_height = locked_block.height + NodeHeight(1);

        while let Some(resp) = stream.next().await {
            let msg = resp.map_err(RpcError::from)?;
            let new_block = msg.into_block().ok_or_else(|| {
                CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!("Expected peer to return a newblock",))
            })?;

            let block = Block::try_from(new_block).map_err(CommsRpcConsensusSyncError::InvalidResponse)?;
            if block.justifies_parent() && block.height() != expected_height {
                return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                    "Peer returned block at height {} but expected {}",
                    block.height(),
                    expected_height,
                )));
            }

            let Some(resp) = stream.next().await else {
                return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                    "Peer closed session before sending QC message"
                )));
            };
            let msg = resp.map_err(RpcError::from)?;
            let qcs = msg.into_quorum_certificates().ok_or_else(|| {
                CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!("Expected peer to return QCs"))
            })?;

            let qcs = qcs
                .into_iter()
                .map(QuorumCertificate::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommsRpcConsensusSyncError::InvalidResponse)?;

            // TODO: Validate

            let Some(resp) = stream.next().await else {
                return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                    "Peer closed session before sending substate update count message"
                )));
            };
            let msg = resp.map_err(RpcError::from)?;
            let num_substates = msg.substate_count().ok_or_else(|| {
                CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!("Expected peer to return substate count",))
            })? as usize;

            if num_substates > MAX_SUBSTATE_UPDATES {
                return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                    "Peer returned {} substate updates, but the maximum is {}",
                    num_substates,
                    MAX_SUBSTATE_UPDATES,
                )));
            }

            let mut updates = Vec::with_capacity(num_substates);
            for _ in 0..num_substates {
                let Some(resp) = stream.next().await else {
                    return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                        "Peer closed session before sending substate updates message"
                    )));
                };
                let msg = resp.map_err(RpcError::from)?;
                let update = msg.into_substate_update().ok_or_else(|| {
                    CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                        "Expected peer to return substate updates",
                    ))
                })?;

                let update = SubstateUpdate::try_from(update).map_err(CommsRpcConsensusSyncError::InvalidResponse)?;
                updates.push(update);
            }

            let Some(resp) = stream.next().await else {
                return Err(CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                    "Peer closed session before sending transactions message"
                )));
            };
            let msg = resp.map_err(RpcError::from)?;
            let transactions = msg.into_transactions().ok_or_else(|| {
                CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!("Expected peer to return QCs"))
            })?;

            debug!(target: LOG_TARGET, "🌐 Received block {}, {} transactions", block, transactions.len());

            let transactions = transactions
                .into_iter()
                .map(Transaction::try_from)
                .map(|r| r.map(TransactionRecord::new))
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommsRpcConsensusSyncError::InvalidResponse)?;

            // TODO: Validate
            debug!(
                target: LOG_TARGET,
                "🌐 Received block {}, {} qcs and {} substate updates",
                block,
                qcs.len(),
                updates.len(),
            );
            counter += 1;
            if counter % 100 == 0 {
                info!(target: LOG_TARGET, "🌐 Syncing block {block}");
            }
            if block.justifies_parent() {
                expected_height += NodeHeight(1);
            } else {
                expected_height = block.height() + NodeHeight(1);
            }
            self.process_block(block, qcs, updates, transactions).await?;
        }

        info!(target: LOG_TARGET, "🌐 {counter} blocks synced to height {}", expected_height - NodeHeight(1));

        Ok(())
    }

    async fn process_block(
        &self,
        block: Block,
        qcs: Vec<QuorumCertificate>,
        updates: Vec<SubstateUpdate>,
        transactions: Vec<TransactionRecord>,
    ) -> Result<(), CommsRpcConsensusSyncError> {
        // Note: this is only used for dummy block calculation, so we avoid the epoch manager call unless it is needed.
        // Otherwise, the committee is empty.
        let local_committee = if block.justifies_parent() {
            Committee::new(vec![])
        } else {
            self.epoch_manager.get_local_committee(block.epoch()).await?
        };

        self.state_store.with_write_tx(|tx| {
            for transaction in transactions {
                transaction.save(tx)?;
            }

            block.justify().save(tx)?;

            // Check if we need to calculate dummy blocks
            // TODO: Validate before doing this. e.g. block.height() is maliciously larger then block.justify().block_height()
            if !block.justifies_parent() {
                let mut last_dummy_block = BlockIdAndHeight {id: *block.justify().block_id(), height: block.justify().block_height()};
                // if the block parent is not the justify parent, then we have experienced a leader failure
                // and should make dummy blocks to fill in the gaps.
                while last_dummy_block.id != *block.parent() {
                    if last_dummy_block.height >= block.height() {
                        warn!(target: LOG_TARGET, "🔥 Bad proposal, dummy block height {} is greater than new height {}", last_dummy_block, block);
                        return Err( ProposalValidationError::CandidateBlockDoesNotExtendJustify {
                            justify_block_height: block.justify().block_height(),
                            candidate_block_height: block.height(),
                        }.into());
                    }

                    let next_height = last_dummy_block.height + NodeHeight(1);
                    let leader = self.leader_strategy.get_leader_public_key(&local_committee, next_height);

                    let dummy_block = Block::dummy_block(
                        last_dummy_block.id,
                        leader.clone(),
                        next_height,
                        block.justify().clone(),
                        block.epoch(),
                    );
                    dummy_block.save(tx)?;
                    last_dummy_block = BlockIdAndHeight { id: *dummy_block.id(), height: next_height };
                    debug!(target: LOG_TARGET, "🍼 DUMMY BLOCK: {}. Leader: {}", last_dummy_block, leader);
                }
            }

            if !block.is_safe(tx.deref_mut())? {
                return Err(CommsRpcConsensusSyncError::BlockNotSafe { block_id: *block.id() });
            }

            if !block.save(tx)? {
                // We've already seen this block. This could happen because we're syncing from high qc and we receive a
                // block that we already have
                return Ok(());
            }

            for qc in qcs {
                qc.save(tx)?;
            }
            block.set_as_processed(tx)?;

            block.update_nodes(
                tx,
                |_, _, _| Ok(()),
                |tx, _, block| {
                    let last_exec = block.as_last_executed();
                    block.commit(tx)?;
                    debug!(
                        target: LOG_TARGET,
                        "✅ COMMIT block {}, last executed height = {}",
                        block,
                        last_exec.height
                    );
                    last_exec.set(tx)?;

                    // Finalize any ACCEPTED transactions
                    for tx_atom in block.commands().iter().filter_map(|cmd| cmd.accept()) {
                        if let Some(mut transaction) = tx_atom.get_transaction(tx.deref_mut()).optional()? {
                            transaction.final_decision = Some(tx_atom.decision);
                            if tx_atom.decision.is_abort() {
                                transaction.abort_details = Some("Abort decision via sync".to_string());
                            }
                            // TODO: execution result - we should execute or we should get the execution result via sync
                            transaction.update(tx)?;
                        }
                    }

                    // Remove from pool including any pending updates
                    TransactionPoolRecord::remove_any(
                        tx,
                        block.commands().iter().filter_map(|cmd| cmd.accept()).map(|t| &t.id),
                    )?;

                    Ok::<_, CommsRpcConsensusSyncError>(())
                },
            )?;
            // Ensure we dont vote on a synced block
            block.as_last_voted().set(tx)?;
            let (ups, downs) = updates.into_iter().partition::<Vec<_>, _>(|u| u.is_create());
            // First do UPs then do DOWNs
            // TODO: stage the updates, then check against the state hash in the block, then persist
            for update in ups {
                update.apply(tx, &block)?;
            }
            for update in downs {
                update.apply(tx, &block)?;
            }
            Ok(())
        })
    }
}

#[async_trait]
impl<TConsensusSpec> SyncManager for RpcStateSyncManager<TConsensusSpec>
where TConsensusSpec: ConsensusSpec<Addr = PeerAddress> + Send + Sync + 'static
{
    type Error = CommsRpcConsensusSyncError;

    async fn check_sync(&self) -> Result<SyncStatus, Self::Error> {
        let committee = self.get_sync_peers().await?;
        if committee.is_empty() {
            warn!(target: LOG_TARGET, "No peers available for sync");
            return Ok(SyncStatus::UpToDate);
        }
        let mut highest_qc: Option<QuorumCertificate> = None;
        let mut num_succeeded = 0;
        let max_failures = committee.max_failures();
        let committee_size = committee.len();
        for addr in committee.addresses() {
            let mut rpc_client = self.client_factory.create_client(addr);
            let mut client = match rpc_client.client_connection().await {
                Ok(client) => client,
                Err(err) => {
                    warn!(target: LOG_TARGET, "Failed to connect to peer {}: {}", addr, err);
                    continue;
                },
            };
            let result = client
                .get_high_qc(GetHighQcRequest {})
                .await
                .map_err(CommsRpcConsensusSyncError::RpcError)
                .and_then(|resp| {
                    resp.high_qc
                        .map(QuorumCertificate::try_from)
                        .transpose()
                        .map_err(CommsRpcConsensusSyncError::InvalidResponse)?
                        .ok_or_else(|| {
                            CommsRpcConsensusSyncError::InvalidResponse(anyhow::anyhow!(
                                "Peer returned an empty high qc"
                            ))
                        })
                });
            let remote_high_qc = match result {
                Ok(resp) => resp,
                Err(err) => {
                    warn!("Failed to get high qc from peer {}: {}", addr, err);
                    continue;
                },
            };

            num_succeeded += 1;
            if highest_qc
                .as_ref()
                .map(|qc| qc.block_height() < remote_high_qc.block_height())
                .unwrap_or(true)
            {
                // TODO: validate

                highest_qc = Some(remote_high_qc);
            }

            if num_succeeded == max_failures {
                break;
            }
        }

        let Some(highest_qc) = highest_qc else {
            return Err(CommsRpcConsensusSyncError::NoPeersAvailable { committee_size });
        };

        let local_high_qc = self.state_store.with_read_tx(|tx| HighQc::get(tx).optional())?;
        let local_height = local_high_qc
            .as_ref()
            .map(|qc| qc.block_height())
            .unwrap_or(NodeHeight(0));
        if highest_qc.block_height() > local_height {
            info!(
                target: LOG_TARGET,
                "Highest QC from peers is at height {} and local high QC is at height {}",
                highest_qc.block_height(),
                local_height,
            );
            return Ok(SyncStatus::Behind);
        }

        Ok(SyncStatus::UpToDate)
    }

    async fn sync(&self) -> Result<(), Self::Error> {
        let committee = self.get_sync_peers().await?;
        if committee.is_empty() {
            warn!(target: LOG_TARGET, "No peers available for sync");
            return Ok(());
        }

        let mut sync_error = None;
        for member in committee.addresses() {
            // Refresh the HighQC each time because a partial sync could have been achieved from a peer
            let locked_block = self
                .state_store
                .with_read_tx(|tx| LockedBlock::get(tx).optional())?
                .unwrap_or_else(|| Block::zero_block().as_locked_block());

            match self.sync_with_peer(member, &locked_block).await {
                Ok(()) => {
                    sync_error = None;
                    break;
                },
                Err(err) => {
                    warn!(target: LOG_TARGET, "Failed to sync with peer {}: {}", member, err);
                    sync_error = Some(err);
                    continue;
                },
            }
        }

        if let Some(err) = sync_error {
            return Err(err);
        }

        Ok(())
    }
}

struct BlockIdAndHeight {
    id: BlockId,
    height: NodeHeight,
}

impl Display for BlockIdAndHeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Block: {} (#{})", self.id, self.height)
    }
}
