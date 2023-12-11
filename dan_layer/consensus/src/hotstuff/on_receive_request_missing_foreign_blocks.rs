//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_storage::{StateStore, StateStoreReadTransaction};
use tari_epoch_manager::EpochManagerReader;
use tokio::sync::mpsc;

use crate::{
    hotstuff::error::HotStuffError,
    messages::{HotstuffMessage, ProposalMessage, RequestMissingForeignBlocksMessage},
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_request_missing_transactions";

pub struct OnReceiveRequestMissingForeignBlocksHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    tx_request_missing_foreign_blocks: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
}

impl<TConsensusSpec> OnReceiveRequestMissingForeignBlocksHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        tx_request_missing_foreign_blocks: mpsc::Sender<(TConsensusSpec::Addr, HotstuffMessage<TConsensusSpec::Addr>)>,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            tx_request_missing_foreign_blocks,
        }
    }

    pub async fn handle(
        &mut self,
        from: TConsensusSpec::Addr,
        msg: RequestMissingForeignBlocksMessage,
    ) -> Result<(), HotStuffError> {
        debug!(target: LOG_TARGET, "{} is requesting {}..{} missing blocks from epoch {}", from, msg.from, msg.to, msg.epoch);
        let foreign_shard = self
            .epoch_manager
            .get_committee_shard_by_validator_address(msg.epoch, &from)
            .await?;
        let missing_blocks = self
            .store
            .with_read_tx(|tx| tx.blocks_get_foreign_ids(foreign_shard.bucket(), msg.from, msg.to))?;
        for block in missing_blocks {
            // We send the proposal back to the requester via hotstuff, so they follow the normal path including
            // validation.
            self.tx_request_missing_foreign_blocks
                .send((
                    from.clone(),
                    HotstuffMessage::ForeignProposal(ProposalMessage { block: block.clone() }),
                ))
                .await
                .map_err(|_| HotStuffError::InternalChannelClosed {
                    context: "tx_leader in OnReceiveRequestMissingForeignBlocksHandler::handle",
                })?;
        }
        Ok(())
    }
}
