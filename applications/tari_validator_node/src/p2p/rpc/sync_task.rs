//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_dan_p2p::proto::rpc::{sync_blocks_response::SyncData, QuorumCertificates, SyncBlocksResponse, Transactions};
use tari_dan_storage::{
    consensus_models::{Block, BlockId, LeafBlock, QuorumCertificate, SubstateUpdate, TransactionRecord},
    StateStore,
    StateStoreReadTransaction,
    StorageError,
};
use tari_rpc_framework::RpcStatus;
use tokio::sync::mpsc;

const LOG_TARGET: &str = "tari::dan::rpc::sync_task";

const BLOCK_BUFFER_SIZE: usize = 15;

type BlockData = (
    Block,
    Vec<QuorumCertificate>,
    Vec<SubstateUpdate>,
    Vec<TransactionRecord>,
);
type BlockBuffer = Vec<BlockData>;

pub struct BlockSyncTask<TStateStore: StateStore> {
    store: TStateStore,
    start_block: Block,
    sender: mpsc::Sender<Result<SyncBlocksResponse, RpcStatus>>,
}

impl<TStateStore: StateStore> BlockSyncTask<TStateStore> {
    pub fn new(
        store: TStateStore,
        start_block: Block,
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
            for data in buffer.drain(..) {
                self.send_block_data(data).await?;
            }

            // If we didnt fill up the buffer, send the final blocks
            if num_items < buffer.capacity() {
                debug!( target: LOG_TARGET, "Sync to last commit complete. Streamed {} item(s)", counter);
                break;
            }
        }

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

        for data in buffer.drain(..) {
            self.send_block_data(data).await?;
        }

        Ok(())
    }

    fn fetch_next_batch(&self, buffer: &mut BlockBuffer, current_block_id: &BlockId) -> Result<BlockId, StorageError> {
        self.store.with_read_tx(|tx| {
            let mut current_block_id = *current_block_id;
            let mut last_block_id = current_block_id;
            loop {
                let children = tx.blocks_get_all_by_parent(&current_block_id)?;
                let Some(child) = children.into_iter().find(|b| b.is_committed()) else {
                    break;
                };

                current_block_id = *child.id();
                if child.is_dummy() {
                    continue;
                }

                last_block_id = current_block_id;
                let all_qcs = child
                    .commands()
                    .iter()
                    .filter_map(|cmd| cmd.transaction())
                    .flat_map(|transaction| transaction.evidence.qc_ids_iter())
                    .collect::<HashSet<_>>();
                let certificates = QuorumCertificate::get_all(tx, all_qcs)?;
                let updates = child.get_substate_updates(tx)?;

                buffer.push((child, certificates, updates, vec![]));
                if buffer.len() == buffer.capacity() {
                    break;
                }
            }
            Ok::<_, StorageError>(last_block_id)
        })
    }

    fn fetch_last_blocks(&self, buffer: &mut BlockBuffer, current_block_id: &BlockId) -> Result<(), StorageError> {
        self.store.with_read_tx(|tx| {
            // TODO: if there are any transactions this will break the syncing node.
            let leaf_block = LeafBlock::get(tx)?;
            let blocks = Block::get_all_blocks_between(tx, current_block_id, leaf_block.block_id(), false)?;
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
                    .filter(|cmd| cmd.transaction().is_some())
                    .flat_map(|cmd| cmd.evidence().qc_ids_iter())
                    .collect::<HashSet<_>>();
                let certificates = QuorumCertificate::get_all(tx, all_qcs)?;

                // No substate updates can occur for blocks after the last commit
                buffer.push((block, certificates, vec![], vec![]));
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

    async fn send_block_data(&mut self, (block, qcs, updates, transactions): BlockData) -> Result<(), ()> {
        self.send(Ok(SyncBlocksResponse {
            sync_data: Some(SyncData::Block((&block).into())),
        }))
        .await?;
        self.send(Ok(SyncBlocksResponse {
            sync_data: Some(SyncData::QuorumCertificates(QuorumCertificates {
                quorum_certificates: qcs.iter().map(Into::into).collect(),
            })),
        }))
        .await?;
        match u32::try_from(updates.len()) {
            Ok(count) => {
                self.send(Ok(SyncBlocksResponse {
                    sync_data: Some(SyncData::SubstateCount(count)),
                }))
                .await?;
            },
            Err(_) => {
                self.send(Err(RpcStatus::general("number of substates exceeds u32")))
                    .await?;
                return Err(());
            },
        }
        for update in updates {
            self.send(Ok(SyncBlocksResponse {
                sync_data: Some(SyncData::SubstateUpdate(update.into())),
            }))
            .await?;
        }

        self.send(Ok(SyncBlocksResponse {
            sync_data: Some(SyncData::Transactions(Transactions {
                transactions: transactions.iter().map(|t| &t.transaction).map(Into::into).collect(),
            })),
        }))
        .await?;

        Ok(())
    }
}
