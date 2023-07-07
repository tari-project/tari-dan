//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{HashMap, HashSet};

use log::*;
use tari_dan_storage::{
    consensus_models::{BlockId, QuorumCertificate},
    StateStore,
};

use crate::{
    hotstuff::{common::update_high_qc, error::HotStuffError, on_beat::OnBeat},
    messages::NewViewMessage,
    traits::{ConsensusSpec, EpochManager},
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_new_view";

pub struct OnReceiveNewViewHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    _leader_strategy: TConsensusSpec::LeaderStrategy,
    epoch_manager: TConsensusSpec::EpochManager,
    newview_message_counts: HashMap<BlockId, HashSet<TConsensusSpec::Addr>>,
    on_beat: OnBeat,
}

impl<TConsensusSpec> OnReceiveNewViewHandler<TConsensusSpec>
where
    TConsensusSpec: ConsensusSpec,
    HotStuffError: From<<TConsensusSpec::EpochManager as EpochManager>::Error>,
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        epoch_manager: TConsensusSpec::EpochManager,
        on_beat: OnBeat,
    ) -> Self {
        Self {
            store,
            _leader_strategy: leader_strategy,
            epoch_manager,
            newview_message_counts: HashMap::default(),
            on_beat,
        }
    }

    pub async fn handle(&mut self, from: TConsensusSpec::Addr, message: NewViewMessage) -> Result<(), HotStuffError> {
        let NewViewMessage { high_qc } = message;
        debug!(
            target: LOG_TARGET,
            "🔥 Receive NEWVIEW for block {}",
            high_qc.block_id(),
        );

        if !self
            .epoch_manager
            .is_validator_in_local_committee(&from, high_qc.epoch())
            .await?
        {
            return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
                epoch: high_qc.epoch(),
                sender: from.to_string(),
                context: format!("Received NEWVIEW from {}", from),
            });
        }

        self.validate_qc(&high_qc)?;

        self.store
            .with_write_tx(|tx| update_high_qc::<TConsensusSpec::StateStore>(tx, &high_qc))?;

        // Take note of unique NEWVIEWs so that we can count them
        let entry = self.newview_message_counts.entry(*high_qc.block_id()).or_default();
        entry.insert(from);

        self.on_beat.beat();

        Ok(())
    }

    fn validate_qc(&self, _qc: &QuorumCertificate) -> Result<(), HotStuffError> {
        // TODO
        Ok(())
    }
}
