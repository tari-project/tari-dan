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

use std::{collections::HashMap, fmt::Debug, hash::Hash, time::Duration};

use log::*;
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
};

use super::hotstuff_error::HotStuffError;
use crate::models::pacemaker::PacemakerWaitStatus;

const LOG_TARGET: &str = "tari::dan_layer::pacemaker_worker";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacemakerWaitStatus {
    WaitTimeOut,
}

async fn send_timeout_message<T: Debug + Send>(
    wait_over: T,
    tx_waiter_status: Sender<(T, PacemakerWaitStatus)>,
) -> Result<(), HotStuffError> {
    info!(target: LOG_TARGET, "Sending timeout message for value: {:?}", wait_over);
    tx_waiter_status
        .send((wait_over, PacemakerWaitStatus::WaitTimeOut))
        .await
        .map_err(|_| HotStuffError::SendError)?;
    Ok(())
}

/// A pacemaker service that is responsible for:
///   1. Receiving [`start_wait`] messages parametrized over a type `T` and start wait up to a max_timeout value.
///   2. Receiving [`stop_wait`] messages for an already existing [`start_wait`].
///   3. If it receives a stop message then it should stop the waiting and do nothing else;
///   4. Otherwise, it should notify the sender of [`start_wait`] message that the max_timeout duration has passed.
#[derive(Debug)]
pub struct Pacemaker<T> {
    rx_start_signal: Receiver<T>,
    rx_shutdown_signal: Receiver<T>,
    tx_inner_map: HashMap<T, Sender<bool>>,
    tx_waiter_status: Sender<(T, PacemakerWaitStatus)>,
    max_timeout: u64,
}

impl<T: Copy + Debug + PartialEq + Eq + Hash + Send + Sync + 'static> Pacemaker<T> {
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
            tx_inner_map: HashMap::new(),
            tx_waiter_status,
            max_timeout,
        }
    }

    pub async fn run(mut self, mut shutdown: ShutdownSignal) -> Result<(), HotStuffError> {
        let max_timeout = self.max_timeout;
        loop {
            tokio::select! {
                msg = self.rx_start_signal.recv() => {
                    if let Some(wait_over) = msg {
                        let (tx, mut rx) = channel::<bool>(1);
                        self.tx_inner_map.insert(wait_over, tx);
                        info!(
                            target: LOG_TARGET,
                            "Received start wait signal for value: {:?}", wait_over
                        );
                        let tx_waiter_status = self.tx_waiter_status.clone();
                        let _join = tokio::spawn(async move {
                            tokio::select! {
                                _ = tokio::time::sleep(Duration::from_millis(max_timeout)) => {
                                    if let Err(e) = send_timeout_message(wait_over, tx_waiter_status).await {
                                        error!(target: LOG_TARGET, "failed to send timeout status message for value = {:?} with error = {}", wait_over, e);
                                    }
                                },
                                _ = rx.recv() => {
                                    info!("The wait signal for wait_over = {:?} has been shutted down", wait_over);
                                }
                            }
                        });
                    }
                },
                msg = self.rx_shutdown_signal.recv() => {
                    if let Some(wait_over) = msg {
                        if let Some(tx) = self.tx_inner_map.get(&wait_over) {
                            tx.send(true).await.unwrap();
                            // remove the entry from the tx_inner_map
                            self.tx_inner_map.remove(&wait_over);
                        }
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
    use tari_shutdown::Shutdown;
    use tokio::sync::mpsc::channel;

    use super::*;

    struct Tester {
        tx_start_waiter_signal: Sender<u32>,
        tx_shutdown_waiter_signal: Sender<u32>,
        rx_waiter_status: Receiver<(u32, PacemakerWaitStatus)>,
    }

    #[tokio::test]
    async fn test_wait_timeout_pacemaker() {
        let (tx_start_waiter_signal, rx_start_waiter_signal) = channel::<u32>(10);
        let (tx_waiter_status, rx_waiter_status) = channel::<(u32, PacemakerWaitStatus)>(10);
        let (tx_shutdown_waiter_signal, rx_shutdown_waiter_signal) = channel::<u32>(10);

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
            10,
            shutdown.to_signal(),
        );

        tester.tx_start_waiter_signal.send(0_u32).await.unwrap();

        tokio::time::sleep(Duration::from_millis(11)).await;

        let msg = tester.rx_waiter_status.recv().await.unwrap();
        assert_eq!(msg.0, 0_u32);
        assert_eq!(msg.1, PacemakerWaitStatus::WaitTimeOut);
    }

    #[tokio::test]
    async fn test_shutdown_pacemaker() {
        let (tx_start_waiter_signal, rx_start_waiter_signal) = channel::<u32>(10);
        let (tx_waiter_status, rx_waiter_status) = channel::<(u32, PacemakerWaitStatus)>(10);
        let (tx_shutdown_waiter_signal, rx_shutdown_waiter_signal) = channel::<u32>(10);

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
            10,
            shutdown.to_signal(),
        );

        // send start waiting signal to wait over
        tester.tx_start_waiter_signal.send(1).await.unwrap();

        tokio::time::sleep(Duration::from_millis(1)).await;

        // send shutdown signal for pacemaker
        tester.tx_shutdown_waiter_signal.send(1).await.unwrap();

        assert!(
            tokio::time::timeout(Duration::from_millis(10), tester.rx_waiter_status.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_wait_timeout_one_out_of_three_pacemaker() {
        let (tx_start_waiter_signal, rx_start_waiter_signal) = channel::<u32>(10);
        let (tx_waiter_status, rx_waiter_status) = channel::<(u32, PacemakerWaitStatus)>(10);
        let (tx_shutdown_waiter_signal, rx_shutdown_waiter_signal) = channel::<u32>(10);

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
            10,
            shutdown.to_signal(),
        );

        // send three wait signals
        tester.tx_start_waiter_signal.send(0).await.unwrap();
        tester.tx_start_waiter_signal.send(1).await.unwrap();
        tester.tx_start_waiter_signal.send(2).await.unwrap();

        tokio::time::sleep(Duration::from_millis(1)).await;

        // stop the middle waiter
        tester.tx_shutdown_waiter_signal.send(1).await.unwrap();

        // we should receive two WaitTimeOut status, for the first and last messages
        // the middle one was stopped, so we don't expect any further status messages
        // to be received
        let msg = tester.rx_waiter_status.recv().await.unwrap();

        assert_eq!(msg.0, 0);
        assert_eq!(msg.1, PacemakerWaitStatus::WaitTimeOut);

        let msg = tester.rx_waiter_status.recv().await.unwrap();

        assert_eq!(msg.0, 2);
        assert_eq!(msg.1, PacemakerWaitStatus::WaitTimeOut);

        assert!(
            tokio::time::timeout(Duration::from_millis(10), tester.rx_waiter_status.recv())
                .await
                .is_err()
        );
    }
}
