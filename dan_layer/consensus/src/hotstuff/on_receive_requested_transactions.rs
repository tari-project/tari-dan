//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_storage::consensus_models::ExecutedTransaction;
use tokio::sync::mpsc;

use crate::{
    hotstuff::error::HotStuffError,
    messages::RequestedTransactionMessage,
    traits::{ConsensusSpec, EpochManager},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_requested_transactions";

pub struct OnReceiveRequestedTransactions<TConsensusSpec: ConsensusSpec> {
    tx_new_transaction: mpsc::Sender<ExecutedTransaction>,
    _phantom: std::marker::PhantomData<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveRequestedTransactions<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
{
    pub fn new(tx_new_transaction: mpsc::Sender<ExecutedTransaction>) -> Self {
        Self {
            tx_new_transaction,
            _phantom: Default::default(),
        }
    }

    pub async fn handle(
        &mut self,
        from: TConsensusSpec::Addr,
        msg: RequestedTransactionMessage,
    ) -> Result<(), HotStuffError> {
        info!(target: LOG_TARGET, "{:?} sent the requested transactions for block {}", from,msg.block_id);
        for tx in msg.transactions {
            self.tx_new_transaction
                .send(tx)
                .await
                .map_err(|_| HotStuffError::InternalChannelClosed {
                    context: "tx_new_transaction in OnReceiveRequestedTransactions::handle",
                })?;
        }
        Ok(())
    }
}
