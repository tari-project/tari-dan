//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{committee::Committee, optional::Optional, Epoch, NodeHeight};
use tari_dan_storage::{
    consensus_models::{HighQc, LastSentVote, LeafBlock},
    StateStore,
};

use crate::{
    hotstuff::{get_next_block_height_and_leader, pacemaker_handle::PaceMakerHandle, HotStuffError},
    messages::{HotstuffMessage, NewViewMessage, VoteMessage},
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_next_sync_view";

pub struct OnNextSyncViewHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    pacemaker: PaceMakerHandle,
}

impl<TConsensusSpec: ConsensusSpec> OnNextSyncViewHandler<TConsensusSpec> {
    pub fn new(
        store: TConsensusSpec::StateStore,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        pacemaker: PaceMakerHandle,
    ) -> Self {
        Self {
            store,
            outbound_messaging,
            leader_strategy,
            pacemaker,
        }
    }

    pub async fn handle(
        &mut self,
        epoch: Epoch,
        current_height: NodeHeight,
        local_committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        // info!(target: LOG_TARGET, "⚠️ Leader failure: NEXTSYNCVIEW for epoch {} and current height {}", epoch,
        // current_height);

        let (new_height, next_leader, leaf_block, high_qc, last_sent_vote) = self.store.with_read_tx(|tx| {
            let leaf_block = LeafBlock::get(tx, epoch)?;
            let (next_height, next_leader, _) = get_next_block_height_and_leader(
                tx,
                local_committee,
                &self.leader_strategy,
                leaf_block.block_id(),
                // Leader failure at current height, so we use the next height
                current_height + NodeHeight(1),
            )?;
            let high_qc = HighQc::get(tx, epoch)?.get_quorum_certificate(tx)?;
            let last_sent_vote = LastSentVote::get(tx)
                .optional()?
                .filter(|vote| high_qc.block_height() < vote.block_height);
            Ok::<_, HotStuffError>((next_height, next_leader, leaf_block, high_qc, last_sent_vote))
        })?;

        if leaf_block.height() == new_height {
            info!(target: LOG_TARGET, "❓️ Leader failure occurred just before we completed processing of the leaf block {leaf_block}. Ignoring.");
            return Ok(());
        }

        self.pacemaker
            .update_view(epoch, new_height, high_qc.block_height())
            .await?;

        let last_vote = last_sent_vote.map(VoteMessage::from);
        info!(
            target: LOG_TARGET,
            "🌟 Send NEWVIEW {new_height} Vote[{}] HighQC: {high_qc} to {next_leader}",
            last_vote.as_ref().map(|v| format!("{}", v.unverified_block_height)).unwrap_or_else(|| "None".to_string()),
        );
        let message = NewViewMessage {
            high_qc,
            new_height,
            last_vote,
        };

        self.outbound_messaging
            .send(next_leader.clone(), HotstuffMessage::NewView(message))
            .await?;

        Ok(())
    }
}
