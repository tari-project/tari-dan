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

use std::{fmt::Debug, time::Duration};

use futures::stream::FuturesUnordered;
use log::*;
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
};

use super::hotstuff_error::HotStuffError;
use crate::models::pacemaker::PacemakerWaitStatus;

const LOG_TARGET: &str = "tari::dan_layer::pacemaker_worker";

#[derive(Debug)]
pub struct Pacemaker<T: Debug + PartialEq + Send> {
    rx_start_signal: Receiver<T>,
    rx_shutdown_signal: Receiver<T>,
    tx_waiter_status: Sender<(T, PacemakerWaitStatus)>,
    max_timeout: u64,
}

impl<T: Debug + PartialEq + Send + 'static> Pacemaker<T> {
    pub fn spawn(
        rx_start_signal: Receiver<T>,
        rx_shutdown_signal: Receiver<T>,
        tx_waiter_status: Sender<(T, PacemakerWaitStatus)>,
        max_timeout: u64,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<Result<(), HotStuffError>> {
        let pacemaker = Self::new(rx_start_signal, rx_shutdown_signal, tx_waiter_status, max_timeout);
        tokio::spawn(pacemaker.run(shutdown))
    }

    fn new(
        rx_start_signal: Receiver<T>,
        rx_shutdown_signal: Receiver<T>,
        tx_waiter_status: Sender<(T, PacemakerWaitStatus)>,
        max_timeout: u64,
    ) -> Self {
        Self {
            rx_start_signal,
            rx_shutdown_signal,
            tx_waiter_status,
            max_timeout,
        }
    }

    async fn send_shutdown_msg(&mut self, wait_over: T) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "Sending shutdown message for value: {:?}", wait_over
        );
        self.tx_waiter_status
            .send((wait_over, PacemakerWaitStatus::ShutDown))
            .await
            .map_err(|_| HotStuffError::SendError)?;
        Ok(())
    }

    async fn send_timeout_msg(&mut self, wait_over: T) -> Result<(), HotStuffError> {
        info!(target: LOG_TARGET, "Sending timeout message for value: {:?}", wait_over);
        self.tx_waiter_status
            .send((wait_over, PacemakerWaitStatus::WaitTimeOut))
            .await
            .map_err(|_| HotStuffError::SendError)?;

        Ok(())
    }

    pub async fn run(mut self, mut shutdown: ShutdownSignal) -> Result<(), HotStuffError> {
        let wait_messages = FuturesUnordered::new();
        loop {
            tokio::select! {
                msg = self.rx_start_signal.recv() => {
                    if let Some(wait_over) = msg {
                        info!(
                            target: LOG_TARGET,
                            "Received start wait signal for value: {:?}", wait_over
                        );

                        wait_messages.push(tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(self.max_timeout)) => {
                                info!(
                                    target: LOG_TARGET,
                                    "Waiter has timed out for value {:?}", wait_over
                                );
                                self.send_timeout_msg(wait_over).await.map_err(|_| HotStuffError::SendError)?;
                            },
                            msg = self.rx_shutdown_signal.recv() => {
                                if let Some(wo) = msg {
                                    if wo == wait_over {
                                        info!(
                                            target: LOG_TARGET,
                                            "Waiter has received a shutdown signal for value: {:?}",
                                            wait_over
                                        );
                                        self.send_shutdown_msg(wait_over).await.map_err(|_| HotStuffError::SendError)?;
                                    }
                                }
                            }
                        })
                    }
                },
                _ = shutdown.wait() => {
                    info!("Shutting down pacemaker service..");
                    break;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_dan_common_types::{PayloadId, ShardId};
    use tari_shutdown::Shutdown;
    use tokio::sync::mpsc::channel;

    use super::*;
    use crate::models::{pacemaker::WaitOver, HotstuffPhase};

    struct Tester {
        tx_start_waiter_signal: Sender<WaitOver>,
        tx_shutdown_waiter_signal: Sender<WaitOver>,
        rx_waiter_status: Receiver<(WaitOver, PacemakerWaitStatus)>,
    }

    #[tokio::test]
    async fn test_wait_timeout_pacemaker() {
        let (tx_start_waiter_signal, rx_start_waiter_signal) = channel::<WaitOver>(10);
        let (tx_waiter_status, rx_waiter_status) = channel::<(WaitOver, PacemakerWaitStatus)>(10);
        let (tx_shutdown_waiter_signal, rx_shutdown_waiter_signal) = channel::<WaitOver>(10);

        let mut tester = Tester {
            rx_waiter_status,
            tx_shutdown_waiter_signal,
            tx_start_waiter_signal,
        };

        let shutdown = Shutdown::new();
        Pacemaker::spawn(
            rx_start_waiter_signal,
            rx_shutdown_waiter_signal,
            tx_waiter_status,
            3_u64,
            shutdown.to_signal(),
        );

        let payload = PayloadId::new([
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31,
        ]);
        let shard_id = ShardId::zero();
        let phase = HotstuffPhase::Prepare;

        tester
            .tx_start_waiter_signal
            .send((payload, shard_id, phase))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(3)).await;

        let msg = tester.rx_waiter_status.recv().await.unwrap();
        assert_eq!(msg.0 .0, payload);
        assert_eq!(msg.0 .1, shard_id);
        assert_eq!(msg.0 .2, phase);
        assert_eq!(msg.1, PacemakerWaitStatus::WaitTimeOut);
    }

    #[tokio::test]
    async fn test_shutdown_pacemaker() {
        let (tx_start_waiter_signal, rx_start_waiter_signal) = channel::<WaitOver>(10);
        let (tx_waiter_status, rx_waiter_status) = channel::<(WaitOver, PacemakerWaitStatus)>(10);
        let (tx_shutdown_waiter_signal, rx_shutdown_waiter_signal) = channel::<WaitOver>(10);

        let mut tester = Tester {
            rx_waiter_status,
            tx_start_waiter_signal,
            tx_shutdown_waiter_signal,
        };

        let shutdown = Shutdown::new();
        Pacemaker::spawn(
            rx_start_waiter_signal,
            rx_shutdown_waiter_signal,
            tx_waiter_status,
            10_u64,
            shutdown.to_signal(),
        );

        let payload = PayloadId::new([
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31,
        ]);
        let shard_id = ShardId::zero();
        let phase = HotstuffPhase::Prepare;

        // send start waiting signal to wait over
        tester
            .tx_start_waiter_signal
            .send((payload, shard_id, phase))
            .await
            .unwrap();

        // tokio::time::sleep(Duration::from_secs(1)).await;
        // send shutdown signal for pacemaker
        tester
            .tx_shutdown_waiter_signal
            .send((payload, shard_id, phase))
            .await
            .unwrap();

        // tokio::time::sleep(Duration::from_secs(2)).await;

        let msg = tester.rx_waiter_status.recv().await.unwrap();
        assert_eq!(msg.0 .0, payload);
        assert_eq!(msg.0 .1, shard_id);
        assert_eq!(msg.0 .2, phase);
        assert_eq!(msg.1, PacemakerWaitStatus::ShutDown);
    }

    #[tokio::test]
    async fn test_wait_timeout_one_out_of_three_pacemaker() {
        let (tx_start_waiter_signal, rx_start_waiter_signal) = channel::<WaitOver>(10);
        let (tx_waiter_status, rx_waiter_status) = channel::<(WaitOver, PacemakerWaitStatus)>(10);
        let (tx_shutdown_waiter_signal, rx_shutdown_waiter_signal) = channel::<WaitOver>(10);

        let mut tester = Tester {
            rx_waiter_status,
            tx_shutdown_waiter_signal,
            tx_start_waiter_signal,
        };

        let shutdown = Shutdown::new();
        Pacemaker::spawn(
            rx_start_waiter_signal,
            rx_shutdown_waiter_signal,
            tx_waiter_status,
            3_u64,
            shutdown.to_signal(),
        );

        let mut data = vec![
            0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31,
        ];

        let payload_0 = PayloadId::new(data.clone());
        let shard_id_0 = ShardId::zero();
        let phase_0 = HotstuffPhase::Prepare;

        tester
            .tx_start_waiter_signal
            .send((payload_0, shard_id_0, phase_0))
            .await
            .unwrap();

        data[0] = 100_u8;

        let payload_1 = PayloadId::new(data.clone());
        let shard_id_1 = ShardId::zero();
        let phase_1 = HotstuffPhase::PreCommit;

        tester
            .tx_start_waiter_signal
            .send((payload_1, shard_id_1, phase_1))
            .await
            .unwrap();

        data[0] = 255_u8;

        let payload_2 = PayloadId::new(data);
        let shard_id_2 = ShardId::zero();
        let phase_2 = HotstuffPhase::Commit;

        tester
            .tx_start_waiter_signal
            .send((payload_2, shard_id_2, phase_2))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_secs(1)).await;

        // stop the middle waiter, on time
        tester
            .tx_shutdown_waiter_signal
            .send((payload_1, shard_id_1, phase_1))
            .await
            .unwrap();

        let msg = tester.rx_waiter_status.recv().await.unwrap();
        assert_eq!(msg.0 .0, payload_1);
        assert_eq!(msg.0 .1, shard_id_1);
        assert_eq!(msg.0 .2, phase_1);

        assert_eq!(msg.1, PacemakerWaitStatus::ShutDown);

        tokio::time::sleep(Duration::from_secs(2)).await;

        let msg = tester.rx_waiter_status.recv().await.unwrap();
        assert_eq!(msg.0 .0, payload_0);
        assert_eq!(msg.0 .1, shard_id_0);
        assert_eq!(msg.0 .2, phase_0);
        assert_eq!(msg.1, PacemakerWaitStatus::WaitTimeOut);

        let msg = tester.rx_waiter_status.recv().await.unwrap();
        assert_eq!(msg.0 .0, payload_2);
        assert_eq!(msg.0 .1, shard_id_2);
        assert_eq!(msg.0 .2, phase_2);

        assert_eq!(msg.1, PacemakerWaitStatus::WaitTimeOut);
    }
}
