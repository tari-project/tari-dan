//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::{Epoch, committee::CommitteeInfo};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{TransactionPool, TransactionRecord},
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tokio::sync::mpsc;

use crate::{
    hotstuff::error::HotStuffError,
    tracing::TraceTimer,
    traits::{BlockTransactionValidator, ConsensusSpec},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_receive_new_transaction";

/// Where a transaction entered consensus from, which determines how much validation it still requires.
enum TransactionSource {
    /// Received from our own mempool, where it has already passed full validation.
    OwnMempool,
    /// Requested from a peer as a missing transaction; not yet validated locally.
    Requested,
}

pub struct OnReceiveNewTransaction<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
    validator: TConsensusSpec::TransactionValidator,
    tx_missing_transactions: mpsc::UnboundedSender<Vec<TransactionId>>,
}

impl<TConsensusSpec> OnReceiveNewTransaction<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        transaction_pool: TransactionPool<TConsensusSpec::StateStore>,
        validator: TConsensusSpec::TransactionValidator,
        tx_missing_transactions: mpsc::UnboundedSender<Vec<TransactionId>>,
    ) -> Self {
        Self {
            store,
            transaction_pool,
            validator,
            tx_missing_transactions,
        }
    }

    pub async fn process_requested(
        &mut self,
        current_epoch: Epoch,
        from: TConsensusSpec::Addr,
        block_id: BlockId,
        transactions: Vec<Transaction>,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "OnReceiveRequestedTransactions");
        info!(target: LOG_TARGET, "Received {} requested transaction(s) for block {} from {:?}", transactions.len(), block_id, from);
        self.store.with_write_tx(|tx| {
            let recs = TransactionRecord::get_any_or_build(&**tx, transactions)?;
            let mut batch = Vec::with_capacity(recs.len());
            debug!(target: LOG_TARGET, "Processing {} requested transactions", recs.len());
            for transaction in recs {
                if let Some(validate_data) = self.validate_new_transaction(
                    tx,
                    current_epoch,
                    transaction,
                    local_committee_info,
                    TransactionSource::Requested,
                )? {
                    debug!(
                        target: LOG_TARGET,
                        "Transaction {} must sequence: {}.",
                        validate_data.transaction.id(),
                        validate_data.must_sequence
                    );
                    batch.push(validate_data);
                }
            }
            debug!(target: LOG_TARGET, "🎱 Adding {} transactions to pool", batch.len());

            self.transaction_pool.insert_new_batched(
                tx,
                local_committee_info.num_preshards(),
                local_committee_info.num_committees(),
                batch
                    .iter()
                    .filter(|data| data.must_sequence)
                    .map(|data| (&data.transaction, data.decision, true)),
            )?;

            // TODO: Could this cause a race-condition? Transaction could be proposed as LocalPrepare before the
            // unparked block is processed (however, if there's a parked block it's probably not our turn to
            // propose). Ideally we remove this channel because it's a work around
            self.tx_missing_transactions
                .send(batch.iter().map(|data| *data.transaction.id()).collect())
                .map_err(|_| HotStuffError::InternalChannelClosed {
                    context: "process_requested",
                })?;
            Ok(())
        })
    }

    pub fn try_sequence_transaction(
        &mut self,
        current_epoch: Epoch,
        transaction: TransactionRecord,
        local_committee_info: &CommitteeInfo,
    ) -> Result<Option<TransactionRecord>, HotStuffError> {
        self.store.with_write_tx(|tx| {
            let Some(validate_data) = self.validate_new_transaction(
                tx,
                current_epoch,
                transaction,
                local_committee_info,
                TransactionSource::OwnMempool,
            )?
            else {
                return Ok(None);
            };
            let NewTransactionValidation {
                transaction,
                decision,
                must_sequence,
            } = validate_data;

            debug!(
                target: LOG_TARGET,
                "Transaction {} must sequence: {}.",
                transaction.id(),
                must_sequence
            );

            if must_sequence {
                self.add_to_pool(tx, &transaction, decision, local_committee_info, true)?;
            }
            Ok(Some(transaction))
        })
    }

    /// Rebuilds pool records for transactions this node already holds but that are absent from the
    /// transaction pool.
    ///
    /// The pool holds derived per-transaction consensus state and is dropped independently of the
    /// transaction records - a state sync clears it wholesale. A proposal for a transaction whose
    /// record we still hold but whose pool record is gone is unvotable (`TransactionNotInPool`), and
    /// the missing-transaction request path cannot repair it because it only replaces absent records.
    ///
    /// A rebuilt record enters at the `New` pool stage and is inserted ready, so callers must
    /// pass only ids whose command in the block being validated is one the `New` stage serves. An id
    /// carrying a later command gains nothing here - the stage check rejects it either way - and the
    /// ready record would be proposed, putting a stale-stage command in every block this node leads.
    ///
    /// Safety comes from [`Self::validate_new_transaction`], which refuses to sequence an id that has
    /// already committed - by its finalized decision or by its receipt existing in state - so this can
    /// never resurrect a transaction the synced state has finalised.
    ///
    /// Returns the number of transactions put back in the pool.
    pub fn resequence_known_transactions<I: IntoIterator<Item = TransactionId>>(
        &self,
        tx: &mut <<TConsensusSpec as ConsensusSpec>::StateStore as StateStore>::WriteTransaction<'_>,
        current_epoch: Epoch,
        transaction_ids: I,
        local_committee_info: &CommitteeInfo,
    ) -> Result<usize, HotStuffError> {
        let mut unpooled = Vec::new();
        for id in transaction_ids {
            // Cheap membership check first: on the hot path almost every id is already pooled, and
            // this avoids loading its record to find that out.
            if self.transaction_pool.exists(&**tx, &id)? {
                continue;
            }
            // A finalized marker means the id has already run its course on this node. Whether an
            // aborted attempt may be sequenced again is the mempool's call; repairing the pool during
            // block validation must not resurrect one as a side effect.
            if TransactionRecord::is_record_finalized(&**tx, &id)? {
                continue;
            }
            unpooled.push(id);
        }
        if unpooled.is_empty() {
            return Ok(0);
        }

        let (recs, _) = TransactionRecord::get_any(&**tx, unpooled.iter())?;
        let mut batch = Vec::with_capacity(recs.len());
        for rec in recs {
            // A record is only ever persisted by `validate_new_transaction` below, after it has passed
            // full validation, so reading one back needs the epoch rules re-checked and nothing more.
            if let Some(validate_data) = self.validate_new_transaction(
                tx,
                current_epoch,
                rec,
                local_committee_info,
                TransactionSource::OwnMempool,
            )? {
                batch.push(validate_data);
            }
        }

        let num_sequenced = batch.iter().filter(|data| data.must_sequence).count();
        if num_sequenced == 0 {
            return Ok(0);
        }

        info!(
            target: LOG_TARGET,
            "🎱 Re-sequencing {num_sequenced} known transaction(s) that are missing from the pool",
        );
        self.transaction_pool.insert_new_batched(
            tx,
            local_committee_info.num_preshards(),
            local_committee_info.num_committees(),
            batch
                .iter()
                .filter(|data| data.must_sequence)
                .map(|data| (&data.transaction, data.decision, true)),
        )?;

        Ok(num_sequenced)
    }

    fn validate_new_transaction(
        &self,
        tx: &mut <<TConsensusSpec as ConsensusSpec>::StateStore as StateStore>::WriteTransaction<'_>,
        current_epoch: Epoch,
        rec: TransactionRecord,
        local_committee_info: &CommitteeInfo,
        source: TransactionSource,
    ) -> Result<Option<NewTransactionValidation>, HotStuffError> {
        if self.transaction_pool.exists(&**tx, rec.id())? {
            return Ok(None);
        }

        // Consensus refuses to re-sequence an id only once it has committed: a commit consumes the id
        // permanently (its effects, including fee payment, are in state). An abort is a circumstance,
        // not a verdict — it commits nothing, so the same transaction may be sequenced again and
        // either commit under current conditions or abort again. Deciding this on the commit decision
        // rather than on finalization keeps the outcome identical on every node regardless of what
        // its local records remember, so a proposal containing a previously-aborted id can never
        // split votes.
        match TransactionRecord::get_finalized_decision(&**tx, rec.id())? {
            Some(decision) if decision.is_commit() => {
                warn!(
                    target: LOG_TARGET, "Transaction {} is already committed. Consensus will ignore it.", rec.id()
                );
                return Ok(None);
            },
            Some(_) => {
                info!(
                    target: LOG_TARGET,
                    "🔄 Transaction {} was previously finalized as aborted. Removing the previous attempt and \
                     sequencing it again.",
                    rec.id()
                );
                // The aborted attempt's bookkeeping must not survive into the new lifecycle,
                // otherwise the stale execution could be returned as this id's finalized result
                // after the new attempt finalizes.
                TransactionRecord::remove_finalized(tx, rec.id())?;
            },
            None => {},
        }

        // The local records are only a cache: the receipt's existence in synced state proves the id
        // has committed even when this node's records no longer (or never) knew it — freshly synced
        // nodes included. The receipt's shard group is always involved because the receipt is a
        // known output of every transaction; shard groups that do not host it never observe the
        // receipt and rely on the receipt's home group refusing the replay.
        if TransactionRecord::receipt_exists(&**tx, rec.id())? {
            warn!(
                target: LOG_TARGET,
                "Transaction {} has a receipt in state and has therefore already committed. Consensus will ignore it.",
                rec.id()
            );
            return Ok(None);
        }

        // Mempool-originated transactions have already passed full validation; their structural properties are
        // immutable, so only the epoch-dependent rules are re-checked. Peer-requested transactions have not been
        // validated locally and are validated in full.
        let result = match source {
            TransactionSource::OwnMempool => self.validator.validate_epoch(current_epoch, rec.transaction()),
            TransactionSource::Requested => self.validator.validate_full(current_epoch, rec.transaction()),
        };

        if let Err(err) = result {
            warn!(
                target: LOG_TARGET,
                "Transaction {} failed validation and will be ignored: {}", rec.id(), err
            );
            return Ok(None);
        }

        rec.save(tx)?;

        let is_involved_in_inputs_or_all_foreign_inputs = rec.is_involved_in_inputs(local_committee_info) ||
            rec.has_all_foreign_input_pledges(&**tx, local_committee_info)?;

        if !is_involved_in_inputs_or_all_foreign_inputs {
            debug!(
                target: LOG_TARGET,
                "Transaction {} has no local inputs or all foreign inputs. Will sequence once we have received the LocalAccept foreign proposal.",
                rec.id()
            );
        }

        Ok(Some(NewTransactionValidation {
            transaction: rec,
            decision: Decision::Commit,
            // If we dont have the required data, dont add to pool yet. We will add it once we have processed the
            // LocalAccept foreign proposal
            must_sequence: is_involved_in_inputs_or_all_foreign_inputs,
        }))
    }

    fn add_to_pool(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        transaction: &TransactionRecord,
        decision: Decision,
        local_committee_info: &CommitteeInfo,
        is_ready: bool,
    ) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "🔥 Adding transaction {} {} ({} input(s)) to pool. Is ready: {}",
            transaction.id(),
            decision,
            transaction.transaction().inputs().len(),
            is_ready
        );
        self.transaction_pool.insert_new(
            tx,
            *transaction.id(),
            decision,
            &transaction.to_initial_evidence(
                local_committee_info.num_preshards(),
                local_committee_info.num_committees(),
            ),
            is_ready,
            transaction.transaction().is_global(),
            transaction.transaction().max_epoch(),
            transaction.transaction().calculate_transaction_weight().as_u64(),
        )?;
        Ok(())
    }
}

struct NewTransactionValidation {
    pub transaction: TransactionRecord,
    pub decision: Decision,
    pub must_sequence: bool,
}
