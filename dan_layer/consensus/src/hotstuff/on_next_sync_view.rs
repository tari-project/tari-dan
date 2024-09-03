//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{optional::Optional, Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{HighQc, LastSentVote},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::HotStuffError,
    messages::{HotstuffMessage, NewViewMessage, VoteMessage},
    traits::{ConsensusSpec, LeaderStrategy, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_next_sync_view";

pub struct OnNextSyncViewHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
}

impl<TConsensusSpec: ConsensusSpec> OnNextSyncViewHandler<TConsensusSpec> {
    pub fn new(
        store: TConsensusSpec::StateStore,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
    ) -> Self {
        Self {
            store,
            outbound_messaging,
            leader_strategy,
            epoch_manager,
        }
    }

    pub async fn handle(&mut self, epoch: Epoch, new_height: NodeHeight) -> Result<(), HotStuffError> {
        info!(target: LOG_TARGET, "⚠️ Leader failure: NEXTSYNCVIEW for epoch {} and node height {}", epoch, new_height);
        // Is the VN registered?
        if !self.epoch_manager.is_epoch_active(epoch).await? {
            info!(
                target: LOG_TARGET,
                "[on_leader_timeout] Validator is not active within this epoch"
            );
            return Ok(());
        }

        let (high_qc, last_sent_vote) = self.store.with_read_tx(|tx| {
            let high_qc = HighQc::get(tx, epoch)?.get_quorum_certificate(tx)?;
            let last_sent_vote = LastSentVote::get(tx)
                .optional()?
                .filter(|vote| high_qc.block_height() < vote.block_height);
            Ok::<_, HotStuffError>((high_qc, last_sent_vote))
        })?;

        let local_committee = self.epoch_manager.get_local_committee(epoch).await?;
        let next_leader = self
            .leader_strategy
            .get_leader_for_next_block(&local_committee, new_height);

        info!(target: LOG_TARGET, "🌟 Send NEWVIEW {new_height} HighQC: {} to {next_leader}", high_qc);
        let message = NewViewMessage {
            high_qc,
            new_height,
            last_vote: last_sent_vote.map(VoteMessage::from),
        };

        self.outbound_messaging
            .send(next_leader.clone(), HotstuffMessage::NewView(message))
            .await?;

        Ok(())
    }
}
