//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_storage::{StateStore, StateStoreReadTransaction};
use tokio::sync::mpsc;

use crate::{
    hotstuff::error::HotStuffError,
    messages::{HotstuffMessage, RequestMissingTransactionsMessage, RequestedTransactionMessage},
    traits::{ConsensusSpec, EpochManager},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_request_missing_transactions";

pub struct OnReceiveRequestMissingTransactions<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    tx_leader: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage)>,
}

impl<TConsensusSpec> OnReceiveRequestMissingTransactions<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
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
        info!(target: LOG_TARGET, "{:?} is requesting missing transactions from block {} with ids {:?}", from,msg.block_id, msg.transactions);
        let txs = self
            .store
            .with_read_tx(|tx| tx.transactions_get_many(&msg.transactions));
        let txs = txs?;
        self.tx_leader
            .send((
                from,
                HotstuffMessage::RequestedTransaction(RequestedTransactionMessage {
                    epoch: msg.epoch,
                    block_id: msg.block_id,
                    transactions: txs,
                }),
            ))
            .await
            .map_err(|_| HotStuffError::InternalChannelClosed {
                context: "tx_new_transaction in OnReceiveRequestMissingTransactions::handle",
            })?;
        Ok(())
    }
}
