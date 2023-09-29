//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_comms::protocol::rpc::RpcStatus;
use tari_dan_storage::{
    consensus_models::{Block, BlockId, QuorumCertificate, SubstateUpdate},
    StateStore,
    StateStoreReadTransaction,
    StorageError,
};
use tari_validator_node_rpc::proto::rpc::SyncBlocksResponse;
use tokio::sync::mpsc;

const LOG_TARGET: &str = "tari::dan::rpc::sync_task";

const BLOCK_BUFFER_SIZE: usize = 15;

type BlockBuffer<TAddr> = Vec<(Block<TAddr>, Vec<QuorumCertificate<TAddr>>, Vec<SubstateUpdate<TAddr>>)>;

pub struct BlockSyncTask<TStateStore: StateStore> {
    store: TStateStore,
    start_block: Block<TStateStore::Addr>,
    sender: mpsc::Sender<Result<SyncBlocksResponse, RpcStatus>>,
}

impl<TStateStore: StateStore> BlockSyncTask<TStateStore> {
    pub fn new(
        store: TStateStore,
        start_block: Block<TStateStore::Addr>,
        sender: mpsc::Sender<Result<SyncBlocksResponse, RpcStatus>>,
    ) -> Self {
        Self {
            store,
            start_block,
            sender,
        }
    }

    pub async fn run(mut self) -> Result<(), ()> {
        let mut buffer = Vec::with_capacity(BLOCK_BUFFER_SIZE);
        let mut current_block_id = *self.start_block.id();
        let mut counter = 0;
        loop {
            match self.fetch_next_batch(&mut buffer, &current_block_id) {
                Ok(last_block) => {
                    current_block_id = last_block;
                },
                Err(err) => {
                    self.send(Err(RpcStatus::log_internal_error(LOG_TARGET)(err))).await?;
                    return Err(());
                },
            }

            let num_items = buffer.len();
            debug!(
                target: LOG_TARGET,
                "Sending {} blocks to peer. Current block id: {}",
                num_items,
                current_block_id,
            );

            counter += buffer.len();
            for (block, quorum_certificates, updates) in buffer.drain(..) {
                self.send(Ok(SyncBlocksResponse {
                    block: Some(block.into()),
                    quorum_certificates: quorum_certificates.iter().map(Into::into).collect(),
                    substate_updates: updates.into_iter().map(Into::into).collect(),
                }))
                .await?;
            }

            // If we didnt fill up the buffer, send the final blocks
            if num_items < buffer.capacity() {
                debug!( target: LOG_TARGET, "Sync to last commit complete. Streamed {} item(s)", counter);
                break;
            }
        }

        // TODO: It may be better to ask each leader to resend each proposal
        match self.fetch_last_blocks(&mut buffer, &current_block_id) {
            Ok(_) => (),
            Err(err) => {
                self.send(Err(RpcStatus::log_internal_error(LOG_TARGET)(err))).await?;
                return Err(());
            },
        }

        debug!(
            target: LOG_TARGET,
            "Sending {} last blocks to peer.",
            buffer.len(),
        );

        for (block, quorum_certificates, updates) in buffer.drain(..) {
            self.send(Ok(SyncBlocksResponse {
                block: Some(block.into()),
                quorum_certificates: quorum_certificates.iter().map(Into::into).collect(),
                substate_updates: updates.into_iter().map(Into::into).collect(),
            }))
            .await?;
        }

        Ok(())
    }

    fn fetch_next_batch(
        &self,
        buffer: &mut BlockBuffer<TStateStore::Addr>,
        current_block_id: &BlockId,
    ) -> Result<BlockId, StorageError> {
        self.store.with_read_tx(|tx| {
            let mut current_block_id = *current_block_id;
            loop {
                let children = tx.blocks_get_all_by_parent(&current_block_id)?;
                let Some(child) = children.into_iter().find(|b| b.is_committed()) else {
                    break;
                };

                current_block_id = *child.id();
                let all_qcs = child
                    .commands()
                    .iter()
                    .flat_map(|cmd| cmd.evidence().qc_ids_iter())
                    .collect::<HashSet<_>>();
                let certificates = QuorumCertificate::get_all(tx, all_qcs)?;
                let updates = child.get_substate_updates(tx)?;
                buffer.push((child, certificates, updates));
                if buffer.len() == buffer.capacity() {
                    break;
                }
            }
            Ok::<_, StorageError>(current_block_id)
        })
    }

    fn fetch_last_blocks(
        &self,
        buffer: &mut BlockBuffer<TStateStore::Addr>,
        current_block_id: &BlockId,
    ) -> Result<(), StorageError> {
        self.store.with_read_tx(|tx| {
            let blocks = Block::get_all_blocks_after(tx, current_block_id)?;
            for block in blocks {
                debug!(
                    target: LOG_TARGET,
                    "Fetching last blocks. Current block: {} to target {}",
                    block,
                    current_block_id
                );
                let all_qcs = block
                    .commands()
                    .iter()
                    .flat_map(|cmd| cmd.evidence().qc_ids_iter())
                    .collect::<HashSet<_>>();
                let certificates = QuorumCertificate::get_all(tx, all_qcs)?;

                // No substate updates can occur for blocks after the last commit
                buffer.push((block, certificates, vec![]));
            }

            Ok::<_, StorageError>(())
        })
    }

    async fn send(&mut self, result: Result<SyncBlocksResponse, RpcStatus>) -> Result<(), ()> {
        if self.sender.send(result).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "Peer stream closed by client before completing. Aborting"
            );
            return Err(());
        }
        Ok(())
    }
}
