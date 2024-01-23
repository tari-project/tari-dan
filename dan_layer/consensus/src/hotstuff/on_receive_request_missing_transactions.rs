//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_storage::{consensus_models::TransactionRecord, StateStore};

use crate::{
    hotstuff::error::HotStuffError,
    messages::{HotstuffMessage, RequestMissingTransactionsMessage, RequestedTransactionMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_request_missing_transactions";

pub struct OnReceiveRequestMissingTransactions<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
}

impl<TConsensusSpec> OnReceiveRequestMissingTransactions<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(store: TConsensusSpec::StateStore, outbound_messaging: TConsensusSpec::OutboundMessaging) -> Self {
        Self {
            store,
            outbound_messaging,
        }
    }

    pub async fn handle(
        &mut self,
        from: TConsensusSpec::Addr,
        msg: RequestMissingTransactionsMessage,
    ) -> Result<(), HotStuffError> {
        debug!(target: LOG_TARGET, "{} is requesting {} missing transactions from block {}", from, msg.transactions.len(), msg.block_id);
        let (txs, _) = self
            .store
            .with_read_tx(|tx| TransactionRecord::get_any(tx, &msg.transactions))?;
        self.outbound_messaging
            .send(
                from,
                HotstuffMessage::RequestedTransaction(RequestedTransactionMessage {
                    epoch: msg.epoch,
                    block_id: msg.block_id,
                    transactions: txs.into_iter().map(|tx| tx.into_transaction()).collect(),
                }),
            )
            .await?;
        Ok(())
    }
}
