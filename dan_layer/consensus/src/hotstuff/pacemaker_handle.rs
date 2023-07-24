//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tokio::sync::mpsc;

use crate::hotstuff::HotStuffError;

pub enum PacemakerEvent {
    ResetLeaderTimeout,
    Beat,
}

#[derive(Debug, Clone)]
pub struct PaceMakerHandle {
    // receiver: mpsc::Receiver<PacemakerEvent>,
    sender: mpsc::Sender<PacemakerEvent>,
}

impl PaceMakerHandle {
    pub fn new(sender: mpsc::Sender<PacemakerEvent>) -> Self {
        // let (sender, receiver) = mpsc::channel();
        Self {
            // receiver,
            sender,
        }
    }

    // pub async fn wait(&mut self) {
    //     self.
    // }

    pub async fn beat(&self) -> Result<(), HotStuffError> {
        self.sender
            .send(PacemakerEvent::Beat)
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }

    pub async fn reset_leader_timeout(&self) -> Result<(), HotStuffError> {
        self.sender
            .send(PacemakerEvent::ResetLeaderTimeout)
            .await
            .map_err(|e| HotStuffError::PacemakerChannelDropped { details: e.to_string() })
    }
}
