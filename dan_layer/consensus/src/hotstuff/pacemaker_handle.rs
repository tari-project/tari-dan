//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::NodeHeight;
use tokio::sync::mpsc;

use crate::hotstuff::HotStuffError;

pub enum PacemakerRequest {
    ResetLeaderTimeout {
        last_seen_height: NodeHeight,
        high_qc_height: NodeHeight,
    },
    TriggerBeat {
        is_forced: bool,
    },
    Start {
        current_height: NodeHeight,
        high_qc_height: NodeHeight,
    },
}

#[derive(Debug, Clone)]
pub struct PaceMakerHandle {
    sender: mpsc::Sender<PacemakerRequest>,
}

impl PaceMakerHandle {
    pub fn new(sender: mpsc::Sender<PacemakerRequest>) -> Self {
        Self { sender }
    }

    /// Start the pacemaker if it hasn't already been started. If it has, this is a no-op
    pub async fn start(&self, current_height: NodeHeight, high_qc_height: NodeHeight) -> Result<(), HotStuffError> {
        self.sender
            .send(PacemakerRequest::Start {
                current_height,
                high_qc_height,
            })
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }

    pub async fn beat(&self) -> Result<(), HotStuffError> {
        self.sender
            .send(PacemakerRequest::TriggerBeat { is_forced: false })
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }

    pub async fn force_beat(&self) -> Result<(), HotStuffError> {
        self.sender
            .send(PacemakerRequest::TriggerBeat { is_forced: true })
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }

    pub async fn reset_leader_timeout(
        &self,
        last_seen_height: NodeHeight,
        high_qc_height: NodeHeight,
    ) -> Result<(), HotStuffError> {
        self.sender
            .send(PacemakerRequest::ResetLeaderTimeout {
                last_seen_height,
                high_qc_height,
            })
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }
}
