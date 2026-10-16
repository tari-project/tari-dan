//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use log::*;
use tari_ootle_common_types::optional::Optional;
use tari_ootle_storage::{StateStore, StateStoreReadTransaction, consensus_models::TransactionRecord};
use tari_ootle_transaction::TransactionId;

use crate::{
    bounded_spawn::PerKeyBoundedSpawn,
    hotstuff::error::HotStuffError,
    messages::{EncodedTransaction, HotstuffMessage, MissingTransactionsRequest, MissingTransactionsResponse},
    tracing::TraceTimer,
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_receive_request_missing_transactions";

/// Encoded transaction bytes one response carries. Leaves a MiB of the 4 MiB transport frame for the message envelope.
/// A response always carries at least one transaction the node holds, so a transaction of up to
/// `max_transaction_size_bytes` is always served. The requester asks again for the transactions a response leaves out.
pub const MAX_MISSING_TRANSACTIONS_RESPONSE_BYTES: usize = 3 * 1024 * 1024;
/// Missing-transaction requests served concurrently. Each one holds up to one response budget of encoded transactions
/// until it is sent.
const MAX_CONCURRENT_REQUESTS: usize = 16;
/// Room for a peer that has a few local and foreign blocks parked on transactions this node holds, without letting
/// one peer take the whole pool.
const MAX_REQUESTS_PER_PEER: usize = 4;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct OnReceiveRequestMissingTransactions<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    bounded_spawner: PerKeyBoundedSpawn<TConsensusSpec::Addr>,
}

impl<TConsensusSpec> OnReceiveRequestMissingTransactions<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(store: TConsensusSpec::StateStore, outbound_messaging: TConsensusSpec::OutboundMessaging) -> Self {
        Self {
            store,
            outbound_messaging,
            bounded_spawner: PerKeyBoundedSpawn::new(MAX_CONCURRENT_REQUESTS, MAX_REQUESTS_PER_PEER, RESPONSE_TIMEOUT),
        }
    }

    /// Serves the request off the consensus worker. A request beyond the pool's or the peer's allowance is dropped.
    pub fn handle(&mut self, from: TConsensusSpec::Addr, msg: MissingTransactionsRequest) {
        info!(target: LOG_TARGET, "{} requested {} missing transaction(s) from block {}", from, msg.transactions.len(), msg.block_id);
        let store = self.store.clone();
        let outbound_messaging = self.outbound_messaging.clone();
        if self
            .bounded_spawner
            .try_spawn(from.clone(), {
                let from = from.clone();
                async move {
                    if let Err(err) = Self::serve(store, outbound_messaging, from, msg).await {
                        error!(target: LOG_TARGET, "Error serving missing transactions request: {}", err);
                    }
                }
            })
            .is_err()
        {
            warn!(
                target: LOG_TARGET,
                "⚠️ No missing transactions request slot available for {}, dropping the request",
                from
            );
        }
    }

    async fn serve(
        store: TConsensusSpec::StateStore,
        mut outbound_messaging: TConsensusSpec::OutboundMessaging,
        from: TConsensusSpec::Addr,
        msg: MissingTransactionsRequest,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "OnReceiveRequestMissingTransactions");
        let transactions = store
            .with_read_tx(|tx| encode_within_budget(tx, &msg.transactions, MAX_MISSING_TRANSACTIONS_RESPONSE_BYTES))?;
        if transactions.len() < msg.transactions.len() {
            debug!(
                target: LOG_TARGET,
                "Serving {} of {} transaction(s) requested by {} for block {}",
                transactions.len(),
                msg.transactions.len(),
                from,
                msg.block_id,
            );
        }

        outbound_messaging
            .send(
                from,
                HotstuffMessage::MissingTransactionsResponse(MissingTransactionsResponse {
                    request_id: msg.request_id,
                    epoch: msg.epoch,
                    block_id: msg.block_id,
                    transactions,
                }),
            )
            .await?;
        Ok(())
    }
}

/// Encodes the requested transactions this node holds until the next one would take the total over `budget_bytes`,
/// always including the first one found. Encoding stops at the first transaction that does not fit, so the work one
/// request costs is bounded by the budget plus one transaction.
fn encode_within_budget<'a, TTx: StateStoreReadTransaction, I: IntoIterator<Item = &'a TransactionId>>(
    tx: &TTx,
    transaction_ids: I,
    budget_bytes: usize,
) -> Result<Vec<EncodedTransaction>, HotStuffError> {
    let mut encoded_transactions = Vec::new();
    let mut total_bytes = 0usize;
    for id in transaction_ids {
        let Some(record) = TransactionRecord::get(tx, id).optional()? else {
            continue;
        };
        let encoded = EncodedTransaction::encode(record.transaction())
            .map_err(|e| HotStuffError::InvariantError(format!("Failed to encode transaction {id}: {e}")))?;
        let new_total = total_bytes.saturating_add(encoded.len());
        if !encoded_transactions.is_empty() && new_total > budget_bytes {
            break;
        }
        total_bytes = new_total;
        encoded_transactions.push(encoded);
    }
    Ok(encoded_transactions)
}
