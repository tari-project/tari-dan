//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{committee::CommitteeInfo, Epoch};

use super::vote_collector::VoteCollector;
use crate::{
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle},
    messages::VoteMessage,
    tracing::TraceTimer,
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_vote";

pub struct OnReceiveVoteHandler<TConsensusSpec: ConsensusSpec> {
    pacemaker: PaceMakerHandle,
    vote_collector: VoteCollector<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveVoteHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(pacemaker: PaceMakerHandle, vote_collector: VoteCollector<TConsensusSpec>) -> Self {
        Self {
            vote_collector,
            pacemaker,
        }
    }

    pub async fn handle(
        &self,
        from: TConsensusSpec::Addr,
        current_epoch: Epoch,
        message: VoteMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::info(LOG_TARGET, "OnReceiveVote");
        match self
            .vote_collector
            .check_and_collect_vote(from, current_epoch, message, local_committee_info)
            .await
        {
            Ok(Some((_, high_qc))) => {
                // HighQc from votes will trigger a view-change in the proposer
                // self.pacemaker
                //     .update_view(high_qc.epoch(), high_qc.block_height(), high_qc.block_height())
                //     .await?;
                // Reset the block time and leader timeouts
                self.pacemaker.reset_leader_timeout(high_qc.block_height()).await?;
                // If we reached quorum, trigger a check to see if we should propose
                self.pacemaker.beat();
            },
            Ok(None) => {},
            Err(err) => {
                // We don't want bad vote messages to kick us out of running mode
                warn!(target: LOG_TARGET, "❌ Error handling vote: {}", err);
            },
        }
        Ok(())
    }
}
