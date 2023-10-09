//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, ops::DerefMut};

use log::*;
use tari_dan_storage::{
    consensus_models::{HighQc, TransactionRecord, ValidBlock},
    StateStore,
    StateStoreWriteTransaction,
};
use tari_transaction::TransactionId;
use tokio::sync::mpsc;

use crate::{
    hotstuff::{pacemaker_handle::PaceMakerHandle, HotStuffError},
    messages::{HotstuffMessage, RequestMissingTransactionsMessage},
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_new_valid_local_block";

#[derive(Debug)]
pub struct OnNewValidLocalBlock<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    pacemaker: PaceMakerHandle,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    tx_block_ready: mpsc::Sender<ValidBlock<TConsensusSpec::Addr>>,
}

impl<TConsensusSpec: ConsensusSpec> OnNewValidLocalBlock<TConsensusSpec> {
    pub fn new(
        store: TConsensusSpec::StateStore,
        pacemaker: PaceMakerHandle,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
        tx_block_ready: mpsc::Sender<ValidBlock<TConsensusSpec::Addr>>,
    ) -> Self {
        Self {
            store,
            pacemaker,
            tx_leader,
            tx_block_ready,
        }
    }

    pub async fn handle(&self, block: ValidBlock<TConsensusSpec::Addr>) -> Result<(), HotStuffError> {
        let (missing_tx_ids, awaiting_execution) = self
            .store
            .with_write_tx(|tx| self.check_for_missing_transactions(tx, &block))?;

        if !missing_tx_ids.is_empty() || !awaiting_execution.is_empty() {
            let block_id = *block.id();
            let block_height = block.height();
            let epoch = block.epoch();
            let block_proposed_by = block.proposed_by().clone();

            let high_qc = self.store.with_read_tx(|tx| HighQc::get(tx))?;

            self.pacemaker
                .reset_leader_timeout(block_height, high_qc.block_height())
                .await?;

            if !missing_tx_ids.is_empty() {
                self.send_message(
                    &block_proposed_by,
                    HotstuffMessage::RequestMissingTransactions(RequestMissingTransactionsMessage {
                        block_id,
                        epoch,
                        transactions: missing_tx_ids,
                    }),
                )
                .await;
            }

            return Ok(());
        }

        self.tx_block_ready
            .send(block)
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_block_ready",
            })?;

        Ok(())
    }

    fn check_for_missing_transactions(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        block: &ValidBlock<TConsensusSpec::Addr>,
    ) -> Result<(HashSet<TransactionId>, HashSet<TransactionId>), HotStuffError> {
        let block = block.block();
        let (transactions, missing_tx_ids) = TransactionRecord::get_any(tx.deref_mut(), block.all_transaction_ids())?;
        let awaiting_execution = transactions
            .into_iter()
            .filter(|tx| tx.result.is_none())
            .map(|tx| *tx.transaction.id())
            .collect::<HashSet<_>>();

        if missing_tx_ids.is_empty() && awaiting_execution.is_empty() {
            debug!(
                target: LOG_TARGET,
                "✅ Block {} has no missing transactions", block
            );
            return Ok((HashSet::new(), HashSet::new()));
        }

        info!(
            target: LOG_TARGET,
            "🔥 Block {} has {} missing transactions and {} awaiting execution", block, missing_tx_ids.len(), awaiting_execution.len(),
        );

        tx.missing_transactions_insert(block, &missing_tx_ids, &awaiting_execution)?;

        Ok((missing_tx_ids, awaiting_execution))
    }

    async fn send_message(&self, to: &TConsensusSpec::Addr, message: HotstuffMessage<TConsensusSpec::Addr>) {
        if self.tx_leader.send((to.clone(), message)).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "tx_leader in ProposalManager::send_message is closed",
            );
        }
    }
}

// impl clone
impl<TConsensusSpec> Clone for OnNewValidLocalBlock<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    TConsensusSpec::StateStore: Clone,
    TConsensusSpec::EpochManager: Clone,
{
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            pacemaker: self.pacemaker.clone(),
            tx_leader: self.tx_leader.clone(),
            tx_block_ready: self.tx_block_ready.clone(),
        }
    }
}
