//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_dan_common_types::{committee::CommitteeInfo, ShardGroup};
use tari_dan_storage::{
    consensus_models::{Block, ForeignProposal, ForeignReceiveCounters},
    StateStore,
};
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{error::HotStuffError, pacemaker_handle::PaceMakerHandle, ProposalValidationError},
    messages::ForeignProposalMessage,
    tracing::TraceTimer,
    traits::ConsensusSpec,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_foreign_proposal";

#[derive(Clone)]
pub struct OnReceiveForeignProposalHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    pacemaker: PaceMakerHandle,
}

impl<TConsensusSpec> OnReceiveForeignProposalHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        pacemaker: PaceMakerHandle,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            pacemaker,
        }
    }

    pub async fn handle(
        &mut self,
        message: ForeignProposalMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "OnReceiveForeignProposal");
        let foreign_committee_info = self
            .epoch_manager
            .get_committee_info_by_validator_public_key(message.block.epoch(), message.block.proposed_by())
            .await?;
        self.validate_and_save(message, local_committee_info, &foreign_committee_info)?;
        Ok(())
    }

    pub fn validate_and_save(
        &mut self,
        message: ForeignProposalMessage,
        local_committee_info: &CommitteeInfo,
        foreign_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let proposal = ForeignProposal::from(message);

        if self.store.with_read_tx(|tx| proposal.exists(tx))? {
            // This is expected behaviour, we may receive the same foreign proposal multiple times
            debug!(
                target: LOG_TARGET,
                "FOREIGN PROPOSAL: Already received proposal for block {}",
                proposal.block().id(),
            );
            return Ok(());
        }

        // TODO: validate justify_qc
        let mut foreign_receive_counter = self
            .store
            .with_read_tx(|tx| ForeignReceiveCounters::get_or_default(tx))?;

        if let Err(err) = self.validate_proposed_block(
            proposal.block(),
            foreign_committee_info.shard_group(),
            local_committee_info.shard_group(),
            &foreign_receive_counter,
        ) {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Invalid proposal: {}. Ignoring.",
                err
            );
            // Invalid blocks should not cause the state machine to transition to Error
            return Ok(());
        }

        foreign_receive_counter.increment_group(foreign_committee_info.shard_group());

        // Justify QC must justify the block
        if proposal.justify_qc().block_id() != proposal.block().id() {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Justify QC block id does not match the block id. Justify QC block id: {}, block id: {}",
                proposal.justify_qc().block_id(),
                proposal.block().id(),
            );
            return Ok(());
        }

        info!(
            target: LOG_TARGET,
            "🧩 Receive FOREIGN PROPOSAL for block {}, justify_qc: {}",
            proposal.block(),
            proposal.justify_qc(),
        );

        self.store.with_write_tx(|tx| {
            foreign_receive_counter.save(tx)?;
            proposal.upsert(tx, None)
        })?;

        // Foreign proposals to propose
        self.pacemaker.on_beat();

        Ok(())
    }

    fn validate_proposed_block(
        &self,
        candidate_block: &Block,
        _foreign_shard: ShardGroup,
        _local_shard: ShardGroup,
        _foreign_receive_counter: &ForeignReceiveCounters,
    ) -> Result<(), ProposalValidationError> {
        // TODO: ignoring for now because this is currently broken
        // let Some(incoming_count) = candidate_block.get_foreign_counter(&local_shard) else {
        //     debug!(target:LOG_TARGET, "Our bucket {local_shard:?} is missing reliability index in the proposed block
        // {candidate_block:?}");     return Err(ProposalValidationError::MissingForeignCounters {
        //         proposed_by: from.to_string(),
        //         hash: *candidate_block.id(),
        //     });
        // };
        // let current_count = foreign_receive_counter.get_count(&foreign_shard);
        // if current_count + 1 != incoming_count {
        //     debug!(target:LOG_TARGET, "We were expecting the index to be {expected_count}, but the index was
        // {incoming_count}", expected_count = current_count + 1);     return
        // Err(ProposalValidationError::InvalidForeignCounters {         proposed_by: from.to_string(),
        //         hash: *candidate_block.id(),
        //         details: format!(
        //             "Expected foreign receive count to be {} but it was {}",
        //             current_count + 1,
        //             incoming_count
        //         ),
        //     });
        // }
        if candidate_block.is_genesis() {
            return Err(ProposalValidationError::ProposingGenesisBlock {
                proposed_by: candidate_block.proposed_by().to_string(),
                hash: *candidate_block.id(),
            });
        }

        let calculated_hash = candidate_block.calculate_hash().into();
        if calculated_hash != *candidate_block.id() {
            return Err(ProposalValidationError::NodeHashMismatch {
                proposed_by: candidate_block.proposed_by().to_string(),
                hash: *candidate_block.id(),
                calculated_hash,
            });
        }

        // TODO: validate justify signatures
        // self.validate_qc(candidate_block.justify(), committee)?;

        Ok(())
    }
}
