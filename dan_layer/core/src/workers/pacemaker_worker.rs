// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use log::*;
use tari_dan_common_types::{PayloadId, ShardId};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
    time::timeout,
};

use super::hotstuff_error::HotStuffError;
use crate::models::{
    pacemaker::{PacemakerSignal, PacemakerWaitStatus, WaitOver},
    HotstuffPhase,
};

const LOG_TARGET: &str = "tari::dan_layer::pacemaker_worker";

#[derive(Debug)]
pub struct LeaderFailurePacemaker {
    rx_waiter_signal: Receiver<PacemakerSignal>,
    tx_waiter_status: Sender<(WaitOver, PacemakerWaitStatus)>,
    max_timeout: u64,
    pub wait_over_set: HashSet<WaitOver>,
}

impl LeaderFailurePacemaker {
    pub fn spawn(
        rx_waiter_signal: Receiver<PacemakerSignal>,
        tx_waiter_status: Sender<(WaitOver, PacemakerWaitStatus)>,
        max_timeout: u64,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<Result<(), HotStuffError>> {
        let pacemaker = Self::new(rx_waiter_signal, tx_waiter_status, max_timeout);
        tokio::spawn(pacemaker.run(shutdown))
    }

    pub fn new(
        rx_waiter_signal: Receiver<PacemakerSignal>,
        tx_waiter_status: Sender<(WaitOver, PacemakerWaitStatus)>,
        max_timeout: u64,
    ) -> Self {
        Self {
            rx_waiter_signal,
            tx_waiter_status,
            max_timeout,
            wait_over_set: HashSet::new(),
        }
    }

    async fn send_timeout_msg(&mut self, wait_over: WaitOver) -> Result<(), HotStuffError> {
        if self.wait_over_set.contains(&wait_over) {
            self.tx_waiter_status
                .send((wait_over, PacemakerWaitStatus::WaitTimeOut))
                .await
                .map_err(|_| HotStuffError::SendError)?;
            // TODO: do we need to remove the wait over data ?
            self.wait_over_set.remove(&wait_over);
        }
        Ok(())
    }

    pub async fn run(mut self, mut shutdown: ShutdownSignal) -> Result<(), HotStuffError> {
        let mut timeout_handle = HashMap::new();
        loop {
            tokio::select! {
                msg = self.rx_waiter_signal.recv() => {
                    if let Some(msg) = msg {
                        match msg {
                            PacemakerSignal::StartWait(wait_over) => {
                                if self.wait_over_set.contains(&wait_over) {
                                    // we already start a waiting process for this
                                    // payload and shard id
                                    continue;
                                }
                                self.wait_over_set.insert(wait_over.clone());
                                tokio::time::sleep(Duration::from_secs(self.max_timeout)).await;
                                // only sends a wait time out message if the waiting process
                                // is still on
                                timeout_handle.insert(wait_over.clone(), timeout(Duration::from_secs(self.max_timeout), async {
                                    info!(target: LOG_TARGET,
                                        "Waiting possible leader failure for payload_id = {}, shard_id = {}, hotstuff_phase = {:?}",
                                        wait_over.0, wait_over.1, wait_over.2
                                );
                                }));
                                self.send_timeout_msg(wait_over).await?;
                            },
                            PacemakerSignal::StopWait(wait_over) => {
                                // simply remove the wait over data from the pacemaker
                                if let Some(&timeout) = timeout_handle.get(&wait_over) {
                                    timeout_handle.remove(&wait_over);
                                    timeout.into_inner();
                                    self.wait_over_set.remove(&wait_over);
                                }
                            },
                        }
                    }
                },
                _ = shutdown.wait() => {
                    info!(target: LOG_TARGET, "💤 Shutting down");
                    break;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_shutdown::Shutdown;
    use tokio::sync::mpsc::channel;

    use super::*;

    struct Tester {
        tx_waiter_signal: Sender<PacemakerSignal>,
        rx_waiter_status: Receiver<(WaitOver, PacemakerWaitStatus)>,
    }

    #[tokio::test]
    async fn test_wait_timeout_pacemaker() {
        let (tx_waiter_signal, rx_waiter_signal) = channel::<PacemakerSignal>(10);
        let (tx_waiter_status, rx_waiter_status) = channel::<(WaitOver, PacemakerWaitStatus)>(10);

        let mut tester = Tester {
            rx_waiter_status,
            tx_waiter_signal,
        };

        let shutdown = Shutdown::new();
        LeaderFailurePacemaker::spawn(rx_waiter_signal, tx_waiter_status, 3_u64, shutdown.to_signal());

        let payload = PayloadId::new([
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31,
        ]);
        let shard_id = ShardId::zero();
        let phase = HotstuffPhase::Prepare;

        tester
            .tx_waiter_signal
            .send(PacemakerSignal::StartWait((payload, shard_id, phase)))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(3)).await;

        let msg = tester.rx_waiter_status.recv().await.unwrap();
        assert_eq!(msg.0 .0, payload);
        assert_eq!(msg.0 .1, shard_id);
        assert_eq!(msg.0 .2, phase);
    }

    #[tokio::test]
    async fn test_shutdown_pacemaker() {
        let (tx_waiter_signal, rx_waiter_signal) = channel::<PacemakerSignal>(10);
        let (tx_waiter_status, rx_waiter_status) = channel::<(WaitOver, PacemakerWaitStatus)>(10);

        let mut tester = Tester {
            rx_waiter_status,
            tx_waiter_signal,
        };

        let shutdown = Shutdown::new();
        LeaderFailurePacemaker::spawn(rx_waiter_signal, tx_waiter_status, 3_u64, shutdown.to_signal());

        let payload = PayloadId::new([
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31,
        ]);
        let shard_id = ShardId::zero();
        let phase = HotstuffPhase::Prepare;

        // send start waiting signal to wait over
        tester
            .tx_waiter_signal
            .send(PacemakerSignal::StartWait((payload, shard_id, phase)))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(1)).await;
        // send shutdown signal for pacemaker
        tester
            .tx_waiter_signal
            .send(PacemakerSignal::StopWait((payload, shard_id, phase)))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(2)).await;

        let msg = tester.rx_waiter_status.recv().await.unwrap();
        assert_eq!(msg.0 .0, payload);
        assert_eq!(msg.0 .1, shard_id);
        assert_eq!(msg.0 .2, phase);
    }
}
