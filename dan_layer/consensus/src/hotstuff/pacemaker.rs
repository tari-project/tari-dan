//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
use std::time::Duration;

use log::*;
use tari_shutdown::ShutdownSignal;
use tokio::sync::{mpsc, watch};

use crate::hotstuff::{
    on_beat::OnBeat,
    on_leader_timeout::OnLeaderTimeout,
    pacemaker_handle::{PaceMakerHandle, PacemakerEvent},
    HotStuffError,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::pacemaker";
const MAX_DELTA: Duration = Duration::from_secs(180); // 3 minutes

pub struct PaceMaker {
    pace_maker_handle: PaceMakerHandle,
    handle_receiver: mpsc::Receiver<PacemakerEvent>,
    shutdown: ShutdownSignal,
    current_delta: Duration,
}

impl PaceMaker {
    pub fn new(shutdown: ShutdownSignal) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self {
            handle_receiver: receiver,
            pace_maker_handle: PaceMakerHandle::new(sender),
            current_delta: Duration::from_millis(3000),
            shutdown,
        }
    }

    pub fn clone_handle(&self) -> PaceMakerHandle {
        self.pace_maker_handle.clone()
    }

    pub fn spawn(self) -> (OnBeat, OnLeaderTimeout) {
        // let (tx_on_beat, rx_on_beat) = watch::channel(());
        // let (tx_on_leader_timeout, rx_on_leader_timeout) = watch::channel(());
        let on_beat = OnBeat::new();
        let on_beat2 = on_beat.clone();
        let on_leader_timeout = OnLeaderTimeout::new();
        let on_leader_timeout2 = on_leader_timeout.clone();
        tokio::spawn(async move {
            if let Err(e) = self.run(on_beat2, on_leader_timeout2).await {
                error!(target: LOG_TARGET, "Error (run): {}", e);
            }
        });
        (on_beat, on_leader_timeout)
    }

    pub async fn run(mut self, on_beat: OnBeat, on_leader_timeout: OnLeaderTimeout) -> Result<(), HotStuffError> {
        let sleep = tokio::time::sleep(self.current_delta);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                // biased;
                Some(event) = self.handle_receiver.recv() => {
                    match event {
                       PacemakerEvent::ResetLeaderTimeout => {
                           sleep.as_mut().reset(tokio::time::Instant::now() + self.current_delta);
                       },
                        PacemakerEvent::Beat => {
                           on_beat.beat();
                        }
                    }
                    // if let Err(e) = self.on_beat().await {
                    //     error!(target: LOG_TARGET, "Error (on_beat): {}", e);
                    // }

                },
                () = &mut sleep => {
                    // tx_on_leader_timeout.send(()).map_err(|e| HotStuffError::PacemakerChannelDropped{ details: e.to_string()})?;
                    on_leader_timeout.leader_timed_out();
                    self.current_delta *= 2;
                    if self.current_delta > MAX_DELTA {
                        self.current_delta = MAX_DELTA;
                    }
                },
                // _ = self.on_leader_timeout.wait() => {
                //     if let Err(e) = self.on_leader_timeout().await {
                //         error!(target: LOG_TARGET, "Error (on_leader_timeout): {}", e);
                //     }
                // }
                _ = self.shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }

        Ok(())
    }
}
