//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_consensus_types::{HighPc, LeafBlock, ProposalCertificate, Vote};
use tari_ootle_common_types::{NodeHeight, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreWriteTransaction,
    consensus_models::{Block, BookkeepingModel},
};

use super::vote_collector::{ProposalVoteCollector, TimeoutVoteCollector};
use crate::{
    hotstuff::{
        ProposalValidationError,
        epoch_state::EpochState,
        error::HotStuffError,
        pacemaker_handle::PaceMakerHandle,
    },
    messages::NewViewMessage,
    tracing::TraceTimer,
    traits::{ConsensusSpec, LeaderStrategy},
    validations::check_quorum_certificate_signatures,
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_receive_new_view";

pub struct OnReceiveNewViewHandler<TConsensusSpec: ConsensusSpec> {
    local_validator_addr: TConsensusSpec::Addr,
    store: TConsensusSpec::StateStore,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    pacemaker: PaceMakerHandle,
    proposal_vote_collector: ProposalVoteCollector<TConsensusSpec>,
    timeout_vote_collector: TimeoutVoteCollector<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveNewViewHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        local_validator_addr: TConsensusSpec::Addr,
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        pacemaker: PaceMakerHandle,
        proposal_vote_collector: ProposalVoteCollector<TConsensusSpec>,
        timeout_vote_collector: TimeoutVoteCollector<TConsensusSpec>,
    ) -> Self {
        Self {
            local_validator_addr,
            store,
            leader_strategy,
            pacemaker,
            proposal_vote_collector,
            timeout_vote_collector,
        }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle(
        &mut self,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        current_height: NodeHeight,
        from: TConsensusSpec::Addr,
        message: NewViewMessage,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "OnReceiveNewView");

        let NewViewMessage {
            high_pc,
            last_vote,
            timeout,
        } = message;
        let timeout_height = timeout.height;
        info!(
            target: LOG_TARGET,
            "🌟 NEWVIEW from {from} with timeout height {timeout_height} with qc {high_pc}",
        );
        if high_pc.epoch() != epoch_state.epoch() {
            warn!(target: LOG_TARGET, "❌ NEWVIEW from {from} with epoch {} but current epoch is {}", high_pc.epoch(), epoch_state.epoch());
            return Ok(());
        }

        if timeout_height < current_height {
            warn!(target: LOG_TARGET, "❌ Ignoring NEWVIEW for {timeout_height} less than the current {current_height}.");
            return Ok(());
        }

        let is_qc_valid = self.store.with_read_tx(|tx| {
            let local_high_qc = HighPc::get(tx, epoch_state.epoch())?;
            // Only accept a higher QC than the local one
            if local_high_qc.block_height > high_pc.height() {
                return Ok(false);
            }

            if let Err(err) = self.validate_qc(&high_pc, epoch_state, self.proposal_vote_collector.signing_service()) {
                warn!(target: LOG_TARGET, "❌ NEWVIEW: Invalid QC: {}", err);
                return Ok(false);
            }

            if !Block::record_exists(tx, &high_pc.calculate_block_id())? {
                // Sync if we do not have the block for this valid QC
                let local_height = LeafBlock::get(tx, epoch_state.epoch())
                    .optional()?
                    .map(|leaf| leaf.height())
                    .unwrap_or_default();
                return Err(HotStuffError::FallenBehind {
                    local_epoch: epoch_state.epoch(),
                    local_height,
                    qc_epoch: high_pc.epoch(),
                    qc_height: high_pc.height(),
                });
            }

            Ok(true)
        })?;

        if !is_qc_valid {
            return Ok(());
        }

        // Check if we are the leader for the view after new_height. We'll set our local view height to the new_height
        // if quorum is reached and propose a block at new_height.
        let (leader, _) = self
            .leader_strategy
            .get_leader(epoch_state.local_committee(), timeout_height);

        if *leader != self.local_validator_addr {
            warn!(target: LOG_TARGET, "❌ NEWVIEW failed, leader is {} at {}. Our address is {}", leader, timeout_height, self.local_validator_addr);
            return Ok(());
        }

        if let Some(vote) = last_vote {
            debug!(
                target: LOG_TARGET,
                "🔥 Receive VOTE with NEWVIEW for node {} {} from {}", vote.height(), vote.block_id, from,
            );
            // HighPc is updated if a quorum is reached in the collector, and will be used if we propose
            if let Err(err) = self
                .proposal_vote_collector
                .check_and_collect_vote(from.clone(), current_height, epoch_state, vote)
                .await
            {
                warn!(target: LOG_TARGET, "❌ Error handling vote: {}", err);
            }
        }

        // Take note of unique NEWVIEWs so that we can count them
        let Some((timeout_certificate, high_tc)) = self
            .timeout_vote_collector
            .check_and_collect_vote(from, current_height, epoch_state, timeout)
            .await?
        else {
            debug!(target: LOG_TARGET, "🌟 Received NEWVIEW but quorum is not yet reached.");
            return Ok(());
        };

        let threshold = epoch_state.local_committee_info().quorum_threshold();

        info!(target: LOG_TARGET, "🌟✅ NEWVIEW height {} (high_tc: {}) has reached quorum ({}/{})", timeout_height, high_tc, timeout_certificate.signatures().len(), threshold.value());
        if timeout_certificate.calculate_id() == *high_tc.id() {
            info!(target: LOG_TARGET, "🕒️ New HIGH TC {}", timeout_certificate);
            // Clear the last sent new view since we have a new certificate
            self.store.with_write_tx(|tx| tx.last_sent_new_view_clear())?;
            self.pacemaker.force_beat(high_tc.height());
        } else {
            info!(target: LOG_TARGET, "❓️ New TC from votes {} but it is not the highest TC {}", timeout_certificate, high_tc);
        }

        Ok(())
    }

    fn validate_qc(
        &self,
        qc: &ProposalCertificate,
        epoch_state: &EpochState<TConsensusSpec::Addr>,
        vote_signing_service: &TConsensusSpec::SignerService,
    ) -> Result<(), ProposalValidationError> {
        if qc.epoch() != epoch_state.epoch() {
            return Err(ProposalValidationError::InvalidEpochInQc {
                block_id: qc.calculate_block_id(),
                qc_id: qc.calculate_id(),
                qc_epoch: qc.epoch(),
                current_epoch: epoch_state.epoch(),
            });
        }
        check_quorum_certificate_signatures::<TConsensusSpec>(
            self.proposal_vote_collector.network(),
            qc.into(),
            epoch_state.local_committee(),
            vote_signing_service,
        )?;
        Ok(())
    }
}
