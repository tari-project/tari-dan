//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
use std::time::{Duration, Instant};

use log::*;
use tari_dan_common_types::NodeHeight;
use tari_shutdown::ShutdownSignal;
use tokio::sync::mpsc;

use crate::hotstuff::{
    on_beat::OnBeat,
    on_force_beat::OnForceBeat,
    on_leader_timeout::OnLeaderTimeout,
    pacemaker_handle::{PaceMakerHandle, PacemakerEvent},
    HotStuffError,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::pacemaker";
const MAX_DELTA: Duration = Duration::from_secs(90);
const MIN_DELTA: Duration = Duration::from_millis(60000);

pub struct PaceMaker {
    pace_maker_handle: PaceMakerHandle,
    handle_receiver: mpsc::Receiver<PacemakerEvent>,
    shutdown: ShutdownSignal,
    current_delta: Duration,
    current_height: NodeHeight,
}

impl PaceMaker {
    pub fn new(shutdown: ShutdownSignal) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        Self {
            handle_receiver: receiver,
            pace_maker_handle: PaceMakerHandle::new(sender),
            current_delta: Duration::from_millis(3000),
            shutdown,
            current_height: NodeHeight(0),
        }
    }

    pub fn clone_handle(&self) -> PaceMakerHandle {
        self.pace_maker_handle.clone()
    }

    pub fn spawn(self) -> (OnBeat, OnForceBeat, OnLeaderTimeout) {
        // let (tx_on_beat, rx_on_beat) = watch::channel(());
        // let (tx_on_leader_timeout, rx_on_leader_timeout) = watch::channel(());
        let on_beat = OnBeat::new();
        let on_beat2 = on_beat.clone();
        let on_force_beat = OnForceBeat::new();
        let on_force_beat2 = on_force_beat.clone();
        let on_leader_timeout = OnLeaderTimeout::new();
        let on_leader_timeout2 = on_leader_timeout.clone();
        tokio::spawn(async move {
            if let Err(e) = self.run(on_beat2, on_force_beat2, on_leader_timeout2).await {
                error!(target: LOG_TARGET, "Error (run): {}", e);
            }
        });
        (on_beat, on_force_beat, on_leader_timeout)
    }

    pub async fn run(
        mut self,
        on_beat: OnBeat,
        on_force_beat: OnForceBeat,
        on_leader_timeout: OnLeaderTimeout,
    ) -> Result<(), HotStuffError> {
        // Don't start the timer until we receive a reset event
        let sleep = tokio::time::sleep(Duration::from_secs(31_000_000));
        let empty_block_deadline = tokio::time::sleep(Duration::from_secs(31_000_000));
        tokio::pin!(sleep);
        tokio::pin!(empty_block_deadline);
        let mut last_reset = Instant::now();
        loop {
            tokio::select! {
                // biased;
                Some(event) = self.handle_receiver.recv() => {
                    match event {
                       PacemakerEvent::ResetLeaderTimeout { last_seen_height } => {
                            error!(target: LOG_TARGET, "XX Resetting leader timeout. Last seen height: {}", last_seen_height);
                            self.current_height = last_seen_height + NodeHeight(1);
                           sleep.as_mut().reset(tokio::time::Instant::now() + self.current_delta);
                            // set a timer for when we must send an empty block...
                            empty_block_deadline.as_mut().reset(tokio::time::Instant::now() + self.current_delta / 2);

                            // if the last time we reset was less than half delta, then we reduce delta
                            if last_reset.elapsed() < self.current_delta / 2 {
                                self.current_delta = self.current_delta * 9 / 10;
                                if self.current_delta < MIN_DELTA {
                                    self.current_delta = MIN_DELTA;
                                }
                            }

                            last_reset = Instant::now();
                       },
                        PacemakerEvent::Beat => {
                            error!(target: LOG_TARGET, "XX Beat");
                           on_beat.beat();
                        }
                    }
                    // if let Err(e) = self.on_beat().await {
                    //     error!(target: LOG_TARGET, "Error (on_beat): {}", e);
                    // }

                },
                () = &mut empty_block_deadline => {
                    error!(target: LOG_TARGET, "XX Empty block deadline: {}", self.current_delta.as_millis());
                    empty_block_deadline.as_mut().reset(tokio::time::Instant::now() + self.current_delta / 2);
                    on_force_beat.beat();
                }
                () = &mut sleep => {
                    error!(target: LOG_TARGET, "XX Leader timed out delta: {}, setting new height: {}", self.current_delta.as_millis(), self.current_height + NodeHeight(1) );
                    // tx_on_leader_timeout.send(()).map_err(|e| HotStuffError::PacemakerChannelDropped{ details: e.to_string()})?;
                    on_leader_timeout.leader_timed_out(self.current_height);
                    self.current_height =  self.current_height + NodeHeight(1);
                    self.current_delta *= 2;
                    if self.current_delta > MAX_DELTA {
                        self.current_delta = MAX_DELTA;
                    }
                    // TODO: perhaps we should track the height
                    sleep.as_mut().reset(tokio::time::Instant::now() + self.current_delta);
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
