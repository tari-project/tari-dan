//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause
use std::{cmp, time::Duration};

use log::*;
use tari_dan_common_types::NodeHeight;
use tari_shutdown::ShutdownSignal;
use tokio::sync::mpsc;

use crate::hotstuff::{
    on_beat::OnBeat,
    on_force_beat::OnForceBeat,
    on_leader_timeout::OnLeaderTimeout,
    pacemaker_handle::{PaceMakerHandle, PacemakerRequest},
    HotStuffError,
};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::pacemaker";
const MAX_DELTA: Duration = Duration::from_secs(300);

pub struct PaceMaker {
    pace_maker_handle: PaceMakerHandle,
    handle_receiver: mpsc::Receiver<PacemakerRequest>,
    shutdown: ShutdownSignal,
    block_time: Duration,
    current_height: NodeHeight,
    current_high_qc_height: NodeHeight,
}

impl PaceMaker {
    pub fn new(shutdown: ShutdownSignal) -> Self {
        let (sender, receiver) = mpsc::channel(100);
        Self {
            handle_receiver: receiver,
            pace_maker_handle: PaceMakerHandle::new(sender),
            // TODO: make network constant. We're starting slow with 10s but should be 1s in the future
            block_time: Duration::from_secs(10),
            shutdown,
            current_height: NodeHeight(0),
            current_high_qc_height: NodeHeight(0),
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
        let leader_timeout = tokio::time::sleep(Duration::MAX);
        let empty_block_deadline = tokio::time::sleep(Duration::MAX);
        tokio::pin!(leader_timeout);
        tokio::pin!(empty_block_deadline);
        let mut started = false;
        loop {
            tokio::select! {
                // biased;
                Some(event) = self.handle_receiver.recv() => {
                    match event {
                       PacemakerRequest::ResetLeaderTimeout { last_seen_height, high_qc_height } => {
                            self.current_height = cmp::max(self.current_height, last_seen_height);
                            assert!(self.current_high_qc_height <= high_qc_height, "high_qc_height must be monotonically increasing");
                            self.current_high_qc_height = high_qc_height;

                            leader_timeout.as_mut().reset(tokio::time::Instant::now() + self.delta_time());
                            // set a timer for when we must send an empty block...
                            empty_block_deadline.as_mut().reset(tokio::time::Instant::now() + self.block_time);

                            started = true;
                       },
                        PacemakerRequest::TriggerBeat {is_forced} => {
                            if is_forced{
                                on_force_beat.beat();
                            } else {
                               on_beat.beat();
                            }
                        }
                        PacemakerRequest::Start { current_height, high_qc_height } => {
                            if !started {
                                self.current_height = current_height;
                                self.current_high_qc_height = high_qc_height;
                                leader_timeout.as_mut().reset(tokio::time::Instant::now() + self.delta_time());
                                empty_block_deadline.as_mut().reset(tokio::time::Instant::now() + self.block_time);
                                started = true;
                            }
                        }
                    }
                },
                () = &mut empty_block_deadline => {
                    empty_block_deadline.as_mut().reset(tokio::time::Instant::now() + self.block_time);
                    on_force_beat.beat();
                }
                () = &mut leader_timeout => {
                    debug!(target: LOG_TARGET, "⚠️ Leader timeout! Current height: {}", self.current_height);
                    self.current_height =  self.current_height + NodeHeight(1);
                    on_leader_timeout.leader_timed_out(self.current_height);
                    leader_timeout.as_mut().reset(tokio::time::Instant::now() + self.delta_time());
                    empty_block_deadline.as_mut().reset(tokio::time::Instant::now() + self.block_time);
                },

                _ = self.shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Current delta time defined as 2^n where n is the difference in height between the last seen block height and the
    /// high QC height. This is always greater than the block time.
    /// Ensure that current_height and current_high_qc_height are set before calling this function.
    fn delta_time(&self) -> Duration {
        let exp = u32::try_from(cmp::min(
            u64::from(u32::MAX),
            self.current_height.saturating_sub(self.current_high_qc_height).as_u64(),
        ))
        .unwrap_or(u32::MAX);
        let delta = cmp::min(
            MAX_DELTA,
            2u64.checked_pow(exp).map(Duration::from_secs).unwrap_or(MAX_DELTA),
        );
        self.block_time + delta
    }
}
