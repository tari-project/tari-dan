//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_transaction::Transaction;
use tokio::sync::mpsc;

use crate::{hotstuff::error::HotStuffError, messages::RequestedTransactionMessage, traits::ConsensusSpec};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_requested_transactions";

pub struct OnReceiveRequestedTransactions<TConsensusSpec: ConsensusSpec> {
    tx_mempool: mpsc::UnboundedSender<Transaction>,
    _phantom: std::marker::PhantomData<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveRequestedTransactions<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(tx_mempool: mpsc::UnboundedSender<Transaction>) -> Self {
        Self {
            tx_mempool,
            _phantom: Default::default(),
        }
    }

    pub async fn handle(
        &mut self,
        from: TConsensusSpec::Addr,
        msg: RequestedTransactionMessage,
    ) -> Result<(), HotStuffError> {
        info!(target: LOG_TARGET, "{:?} receiving {} requested transactions for block {}", from, msg.transactions.len(), msg.block_id);
        for tx in msg.transactions {
            self.tx_mempool
                .send(tx)
                .map_err(|_| HotStuffError::InternalChannelClosed {
                    context: "tx_new_transaction in OnReceiveRequestedTransactions::handle",
                })?;
        }
        Ok(())
    }
}
