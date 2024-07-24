//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{optional::Optional, Epoch};
use tari_dan_storage::{
    consensus_models::{TransactionPool, TransactionRecord},
    StateStore,
};
use tari_transaction::{Transaction, TransactionId};
use tokio::sync::mpsc;

use crate::{
    hotstuff::error::HotStuffError,
    messages::MissingTransactionsResponse,
    traits::{BlockTransactionExecutor, ConsensusSpec},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_requested_transactions";

pub struct OnReceiveNewTransaction<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    executor: TConsensusSpec::TransactionExecutor,
    tx_missing_transactions: mpsc::UnboundedSender<TransactionId>,
}

impl<TConsensusSpec> OnReceiveNewTransaction<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        executor: TConsensusSpec::TransactionExecutor,
        tx_missing_transactions: mpsc::UnboundedSender<TransactionId>,
    ) -> Self {
        Self {
            store,
            transaction_pool,
            executor,
            tx_missing_transactions,
        }
    }

    pub async fn process_requested(
        &mut self,
        current_epoch: Epoch,
        from: TConsensusSpec::Addr,
        msg: MissingTransactionsResponse,
    ) -> Result<(), HotStuffError> {
        info!(target: LOG_TARGET, "Receiving {} requested transactions for block {} from {:?}", msg.transactions.len(), msg.block_id, from, );
        self.store.with_write_tx(|tx| {
            for transaction in msg.transactions {
                if let Some(rec) = self.validate_and_sequence_transaction(tx, current_epoch, transaction)? {
                    // TODO: Could this cause a race-condition? Transaction could be proposed as Prepare before the
                    // unparked block is processed (however, if there's a parked block it's probably not our turn to
                    // propose). Ideally we remove this channel because it's a work around
                    self.tx_missing_transactions
                        .send(*rec.id())
                        .map_err(|_| HotStuffError::InternalChannelClosed {
                            context: "process_requested",
                        })?;
                }
            }
            Ok(())
        })
    }

    pub fn try_sequence_transaction(
        &mut self,
        current_epoch: Epoch,
        transaction: Transaction,
    ) -> Result<Option<TransactionRecord>, HotStuffError> {
        self.store
            .with_write_tx(|tx| self.validate_and_sequence_transaction(tx, current_epoch, transaction))
    }

    fn validate_and_sequence_transaction(
        &self,
        tx: &mut <<TConsensusSpec as ConsensusSpec>::StateStore as StateStore>::WriteTransaction<'_>,
        current_epoch: Epoch,
        transaction: Transaction,
    ) -> Result<Option<TransactionRecord>, HotStuffError> {
        if self.transaction_pool.exists(&**tx, transaction.id())? {
            return Ok(None);
        }

        let mut rec = TransactionRecord::get(&**tx, transaction.id())
            .optional()?
            .unwrap_or_else(|| TransactionRecord::new(transaction));

        // Edge case: a validator sends a transaction that is already finalized as a missing transaction or via
        // propagation
        if rec.is_finalized() {
            warn!(
                target: LOG_TARGET, "Transaction {} is already finalized. Consensus will ignore it.", rec.id()
            );
            return Ok(None);
        }

        let result = self.executor.validate(&**tx, current_epoch, rec.transaction());

        if let Err(err) = result {
            warn!(
                target: LOG_TARGET,
                "Transaction {} failed validation: {}", rec.id(), err
            );
            rec.set_current_decision_to_abort(err.to_string()).insert(tx)?;
            self.add_to_pool(tx, &rec)?;
            return Ok(Some(rec));
        }
        rec.save(tx)?;
        self.add_to_pool(tx, &rec)?;
        Ok(Some(rec))
    }

    fn add_to_pool(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &TransactionRecord,
    ) -> Result<(), HotStuffError> {
        self.transaction_pool
            .insert_new(tx, *transaction.id(), transaction.current_decision())?;
        Ok(())
    }
}
