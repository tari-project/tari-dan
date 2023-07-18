//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_storage::{consensus_models::ExecutedTransaction, StateStore};
use tokio::sync::mpsc;

use crate::{
    hotstuff::error::HotStuffError,
    messages::{HotstuffMessage, RequestMissingTransactionsMessage, RequestedTransactionMessage},
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_request_missing_transactions";

pub struct OnReceiveRequestMissingTransactions<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
}

impl<TConsensusSpec> OnReceiveRequestMissingTransactions<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
    ) -> Self {
        Self { store, tx_leader }
    }

    pub async fn handle(
        &mut self,
        from: TConsensusSpec::Addr,
        msg: RequestMissingTransactionsMessage,
    ) -> Result<(), HotStuffError> {
        debug!(target: LOG_TARGET, "{:?} is requesting missing transactions from block {} with ids {:?}", from,msg.block_id, msg.transactions);
        let txs = self
            .store
            .with_read_tx(|tx| ExecutedTransaction::get_many(tx, &msg.transactions))?;
        self.tx_leader
            .send((
                from,
                HotstuffMessage::RequestedTransaction(RequestedTransactionMessage {
                    epoch: msg.epoch,
                    block_id: msg.block_id,
                    transactions: txs.into_iter().map(|tx| tx.into_transaction()).collect(),
                }),
            ))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_new_transaction in OnReceiveRequestMissingTransactions::handle",
            })?;
        Ok(())
    }
}
