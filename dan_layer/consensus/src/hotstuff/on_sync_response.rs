//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_dan_storage::{consensus_models::Block, StateStore};
use tari_transaction::Transaction;
use tokio::sync::mpsc;

use crate::{hotstuff::HotStuffError, messages::SyncResponseMessage, traits::ConsensusSpec};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_sync_request";

#[derive(Debug)]
pub struct OnSyncResponse<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    inflight_requests: HashSet<TConsensusSpec::Addr>,
    tx_mempool: mpsc::UnboundedSender<Transaction>,
}

impl<TConsensusSpec: ConsensusSpec> OnSyncResponse<TConsensusSpec> {
    pub fn new(store: TConsensusSpec::StateStore, tx_mempool: mpsc::UnboundedSender<Transaction>) -> Self {
        Self {
            store,
            inflight_requests: HashSet::new(),
            tx_mempool,
        }
    }

    pub fn add_inflight_request(&mut self, addr: TConsensusSpec::Addr) {
        self.inflight_requests.insert(addr);
    }

    pub fn handle(
        &mut self,
        from: TConsensusSpec::Addr,
        msg: SyncResponseMessage<TConsensusSpec::Addr>,
    ) -> Result<Vec<Block<TConsensusSpec::Addr>>, HotStuffError> {
        if !self.inflight_requests.remove(&from) {
            warn!(
                target: LOG_TARGET,
                "⚠️ Ignoring unrequested SyncResponse from {}", from
            );
            return Ok(vec![]);
        }

        if msg.blocks.is_empty() {
            warn!(
                target: LOG_TARGET,
                "⚠️ Ignoring empty SyncResponse from {}", from
            );
            return Ok(vec![]);
        }

        let mut blocks = Vec::with_capacity(msg.blocks.len());
        for full_block in msg.blocks {
            for transaction in full_block.transactions {
                if self.tx_mempool.send(transaction).is_err() {
                    warn!(target: LOG_TARGET, "Mempool channel closed while sending transactions from SyncResponse");
                    return Ok(vec![]);
                }
            }
            self.store.with_write_tx(|tx| {
                // TODO: validate
                for qc in full_block.qcs {
                    qc.save(tx)?;
                }
                Ok::<_, HotStuffError>(())
            })?;

            blocks.push(full_block.block);
        }

        blocks.sort_by_key(|b| b.height());

        Ok(blocks)
    }
}
