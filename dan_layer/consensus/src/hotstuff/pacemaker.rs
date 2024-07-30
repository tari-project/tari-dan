//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp,
    time::{Duration, Instant},
};

use log::*;
use tari_dan_common_types::NodeHeight;
use tokio::sync::mpsc;

use crate::hotstuff::{
    current_view::CurrentView,
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
    current_view: CurrentView,
    current_high_qc_height: NodeHeight,
    block_time: Duration,
}

impl PaceMaker {
    pub fn new(max_base_time: Duration) -> Self {
        let (sender, receiver) = mpsc::channel(100);

        let on_beat = OnBeat::new();
        let on_force_beat = OnForceBeat::new();
        let on_leader_timeout = OnLeaderTimeout::new();
        let current_height = CurrentView::new();

        Self {
            handle_receiver: receiver,
            pace_maker_handle: PaceMakerHandle::new(
                sender,
                on_beat,
                on_force_beat,
                on_leader_timeout,
                current_height.clone(),
            ),
            current_view: current_height,
            current_high_qc_height: NodeHeight(0),
            block_time: max_base_time,
        }
    }

    pub fn clone_handle(&self) -> PaceMakerHandle {
        self.pace_maker_handle.clone()
    }

    pub fn spawn(mut self) {
        let handle = self.clone_handle();
        let on_beat = handle.get_on_beat();
        let on_force_beat = handle.get_on_force_beat();
        let on_leader_timeout = handle.get_on_leader_timeout();

        tokio::spawn(async move {
            if let Err(e) = self.run(on_beat, on_force_beat, on_leader_timeout).await {
                error!(target: LOG_TARGET, "Error (run): {}", e);
            }
        });
    }

    pub async fn run(
        &mut self,
        on_beat: OnBeat,
        on_force_beat: OnForceBeat,
        on_leader_timeout: OnLeaderTimeout,
    ) -> Result<(), HotStuffError> {
        // Don't start the timer until we start the pacemaker
        let leader_timeout = tokio::time::sleep(Duration::MAX);
        let block_timer = tokio::time::sleep(Duration::MAX);
        tokio::pin!(leader_timeout);
        tokio::pin!(block_timer);

        let mut started = false;

        loop {
            tokio::select! {
                // biased;
                maybe_req = self.handle_receiver.recv() => {
                    if let Some(req) = maybe_req {
                        match req {
                           PacemakerRequest::ResetLeaderTimeout { high_qc_height } => {
                                if !started {
                                    continue;
                                }

                                if let Some(height) = high_qc_height {
                                    self.current_high_qc_height =  height;
                                }
                                let delta = self.delta_time();
                                info!(target: LOG_TARGET, "Reset! Current height: {}, Delta: {:.2?}", self.current_view, delta);
                                leader_timeout.as_mut().reset(tokio::time::Instant::now() + delta);
                                // set a timer for when we must send a block...
                                block_timer.as_mut().reset(tokio::time::Instant::now() + self.block_time);
                           },
                            PacemakerRequest::Start { high_qc_height } => {
                                info!(target: LOG_TARGET, "🚀 Starting pacemaker at leaf height {} and high QC: {}", self.current_view, high_qc_height);
                                if started {
                                    continue;
                                }
                                self.current_high_qc_height = high_qc_height;
                                let delta = self.delta_time();
                                info!(target: LOG_TARGET, "Reset! Current height: {}, Delta: {:.2?}", self.current_view, delta);
                                leader_timeout.as_mut().reset(tokio::time::Instant::now() + delta);
                                block_timer.as_mut().reset(tokio::time::Instant::now() + self.block_time);
                                on_beat.beat();
                                started = true;
                            }
                            PacemakerRequest::Stop => {
                                info!(target: LOG_TARGET, "💤 Stopping pacemaker");
                                started = false;
                                // TODO: we could use futures-rs Either
                                leader_timeout.as_mut().reset(far_future());
                                block_timer.as_mut().reset(far_future());
                            }
                        }
                    } else{
                        info!(target: LOG_TARGET, "💤 All pacemaker handles dropped");
                        break;
                    }
                },
                () = &mut block_timer => {
                    block_timer.as_mut().reset(tokio::time::Instant::now() + self.block_time);
                    on_force_beat.beat(None);
                }
                () = &mut leader_timeout => {
                    block_timer.as_mut().reset(tokio::time::Instant::now() + self.block_time);

                    let delta = self.delta_time();
                    leader_timeout.as_mut().reset(tokio::time::Instant::now() + delta);
                    info!(target: LOG_TARGET, "⚠️ Leader timeout! Current view: {}, Delta: {:.2?}", self.current_view, delta);
                    self.current_view.set_next_height();
                    on_leader_timeout.leader_timed_out(self.current_view.get_height());
                },

            }
        }

        Ok(())
    }

    /// Current delta time defined as 2^n where n is the difference in height between the last seen block height and the
    /// high QC height. This is always greater than the block time.
    /// Ensure that current_height and current_high_qc_height are set before calling this function.
    fn delta_time(&self) -> Duration {
        let current_height = self.current_view.get_height();
        if current_height.is_zero() || self.current_high_qc_height.is_zero() {
            // Allow extra time for the first block
            return self.block_time * 2;
        }
        let exp = u32::try_from(cmp::min(
            u64::from(u32::MAX),
            cmp::max(1, current_height.saturating_sub(self.current_high_qc_height).as_u64()),
        ))
        .unwrap_or(u32::MAX);
        let delta = cmp::min(
            MAX_DELTA,
            2u64.checked_pow(exp).map(Duration::from_secs).unwrap_or(MAX_DELTA),
        );
        // TODO: get real avg latency
        let avg_latency = Duration::from_secs(2);
        self.block_time + delta + avg_latency
    }
}

fn far_future() -> tokio::time::Instant {
    // Taken verbatim from the tokio library:
    // Roughly 30 years from now.
    // API does not provide a way to obtain max `Instant`
    // or convert specific date in the future to instant.
    // 1000 years overflows on macOS, 100 years overflows on FreeBSD.
    tokio::time::Instant::from_std(Instant::now() + Duration::from_secs(86400 * 365 * 30))
}
