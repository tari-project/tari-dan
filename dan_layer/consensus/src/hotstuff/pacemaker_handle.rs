//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::NodeHeight;
use tari_dan_storage::consensus_models::LeafBlock;
use tokio::sync::mpsc;

use crate::hotstuff::{
    current_height::CurrentHeight,
    on_beat::OnBeat,
    on_force_beat::OnForceBeat,
    on_leader_timeout::OnLeaderTimeout,
    HotStuffError,
};

pub enum PacemakerRequest {
    ResetLeaderTimeout { high_qc_height: NodeHeight },
    Start { high_qc_height: NodeHeight },
    Stop,
}

#[derive(Debug, Clone)]
pub struct PaceMakerHandle {
    sender: mpsc::Sender<PacemakerRequest>,
    on_beat: OnBeat,
    on_force_beat: OnForceBeat,
    on_leader_timeout: OnLeaderTimeout,
    current_height: CurrentHeight,
}

impl PaceMakerHandle {
    pub(super) fn new(
        sender: mpsc::Sender<PacemakerRequest>,
        on_beat: OnBeat,
        on_force_beat: OnForceBeat,
        on_leader_timeout: OnLeaderTimeout,
        current_height: CurrentHeight,
    ) -> Self {
        Self {
            sender,
            on_beat,
            on_force_beat,
            on_leader_timeout,
            current_height,
        }
    }

    /// Start the pacemaker if it hasn't already been started. If it has, this is a no-op
    pub async fn start(&self, current_height: NodeHeight, high_qc_height: NodeHeight) -> Result<(), HotStuffError> {
        self.current_height.update(current_height);
        self.sender
            .send(PacemakerRequest::Start { high_qc_height })
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }

    /// Stop the pacemaker. If it hasn't been started, this is a no-op
    pub async fn stop(&self) -> Result<(), HotStuffError> {
        self.sender
            .send(PacemakerRequest::Stop)
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }

    /// Signal the pacemaker trigger a beat. If the pacemaker has not been started, this is a no-op
    pub fn beat(&self) {
        self.on_beat.beat();
    }

    /// Signal the pacemaker trigger a forced beat. If the pacemaker has not been started, this is a no-op
    pub fn force_beat(&self, parent_block: LeafBlock) {
        self.on_force_beat.beat(Some(parent_block));
    }

    pub fn get_on_beat(&self) -> OnBeat {
        self.on_beat.clone()
    }

    pub fn get_on_force_beat(&self) -> OnForceBeat {
        self.on_force_beat.clone()
    }

    pub fn get_on_leader_timeout(&self) -> OnLeaderTimeout {
        self.on_leader_timeout.clone()
    }

    /// Reset the leader timeout. This should be called when a valid leader proposal is received.
    pub async fn update_view(
        &self,
        last_seen_height: NodeHeight,
        high_qc_height: NodeHeight,
    ) -> Result<(), HotStuffError> {
        // Update current height here to prevent possibility of race conditions
        self.current_height.update(last_seen_height);
        self.sender
            .send(PacemakerRequest::ResetLeaderTimeout { high_qc_height })
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }

    pub fn current_height(&self) -> NodeHeight {
        self.current_height.get()
    }
}
