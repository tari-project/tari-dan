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

use futures::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use log::*;
use tari_shutdown::ShutdownSignal;
use tokio::sync::{
    mpsc::{channel, Receiver, Sender},
    oneshot,
};

use super::hotstuff_error::HotStuffError;

const LOG_TARGET: &str = "tari::dan_layer::pacemaker_worker";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacemakerWaitStatus {
    WaitTimeOut,
}

/// A pacemaker service that is responsible for:
///   1. Receiving [`start_wait`] messages parametrized over a type `T` and start wait up to a max_timeout value.
///   2. Receiving [`stop_wait`] messages for an already existing [`start_wait`].
///   3. If it receives a stop message then it should stop the waiting and do nothing else;
///   4. Otherwise, it should notify the sender of [`start_wait`] message that the max_timeout duration has passed.
#[derive(Debug)]
pub struct Pacemaker<T> {
    /// Receiver for start signal. Whenever the service receives a new instance of (`T`, `Duration`)
    /// it starts a waiting time process for the T-value with timeout `Duration`
    rx_start_signal: Receiver<(T, Duration)>,
    /// Receiver of stop/shutdown signal. It is assumed that whenever a new value `T` is
    /// received, that we are already waiting for timeout on that value (i.e. rx_start_signal,
    /// received that same value first). If such value is received, then we stop the waiting process
    rx_stop_signal: Receiver<T>,
    /// Keeps track of waiting timers, parametrized by values of `T`
    waiting_futures: FuturesUnordered<BoxFuture<'static, Option<T>>>,
    /// For each pending timeout, we keep track of inner oneshot channels for communication
    pending_timeouts: HashMap<T, oneshot::Sender<()>>,
    /// Sender which sends Timeout status to the other end of the channel
    tx_waiter_status: Sender<T>,
}

impl<T> Pacemaker<T>
where T: Clone + Debug + PartialEq + Eq + Hash + Send + Sync + 'static
{
    pub fn spawn(shutdown: ShutdownSignal) -> PacemakerHandle<T> {
        let (tx_timeout_status, rx_timeout_status) = channel(100);
        let (tx_start_waiter_signal, rx_start_waiter_signal) = channel(100);
        let (tx_stop_waiter_signal, rx_stop_waiter_signal) = channel(100);

        let pacemaker = Self::new(rx_start_waiter_signal, rx_stop_waiter_signal, tx_timeout_status);
        tokio::spawn(pacemaker.run(shutdown));

        PacemakerHandle {
            rx_timeout_status,
            tx_start_waiter_signal,
            tx_stop_waiter_signal,
        }
    }

    fn new(rx_start_signal: Receiver<(T, Duration)>, rx_stop_signal: Receiver<T>, tx_waiter_status: Sender<T>) -> Self {
        Self {
            rx_start_signal,
            rx_stop_signal,
            waiting_futures: FuturesUnordered::new(),
            pending_timeouts: HashMap::new(),
            tx_waiter_status,
        }
    }

    fn handle_stop_signal(&mut self, t: T) {
        if let Some(signal) = self.pending_timeouts.remove(&t) {
            let _ = signal.send(());
        }
    }

    pub async fn run(mut self, mut shutdown: ShutdownSignal) -> Result<(), HotStuffError> {
        loop {
            tokio::select! {
                Some((wait_over, duration_timeout)) = self.rx_start_signal.recv() => {
                    info!(
                        target: LOG_TARGET,
                        "Received start wait signal for value: {:?}", wait_over
                    );
                    let (tx, rx_stop_signal) = oneshot::channel();
                    if self.pending_timeouts.insert(wait_over.clone(), tx).is_none() {
                        self.waiting_futures.push(Box::pin(async move {
                            tokio::select! {
                                _ = tokio::time::sleep(duration_timeout) => {
                                    info!("The wait signal for value = {:?} has timeout", wait_over);
                                    Some(wait_over)
                                },
                                _ = rx_stop_signal => {
                                    info!("The wait signal for wait_over = {:?} has been shut down", wait_over);
                                    None
                                }
                            }
                        }));
                    } else {
                        info!(target: LOG_TARGET, "Already received an existing wait timer for value = {:?}", wait_over);
                    }
                },
                Some(msg) = self.waiting_futures.next() => {
                    if let Some(t) = msg {
                        // at this point it is safe to remove the wait_over from pending_timeouts
                        // so that this data structure doesn't grow every time
                        self.pending_timeouts.remove(&t);
                        // send status
                        let send_status = self.tx_waiter_status.send(t);
                        if send_status.await.is_err() {
                            error!(target: LOG_TARGET, "Hotstuff waiter has shut down");
                        }
                    }
                },
                Some(t) = self.rx_stop_signal.recv() => {
                    self.handle_stop_signal(t);
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

pub struct PacemakerHandle<T> {
    tx_start_waiter_signal: Sender<(T, Duration)>,
    tx_stop_waiter_signal: Sender<T>,
    rx_timeout_status: Receiver<T>,
}

impl<T: Debug> PacemakerHandle<T> {
    pub async fn start_timer(&self, wait_over: T, duration_timeout: Duration) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "Pacemaker: start wait timer for value = {:?}", wait_over
        );
        self.tx_start_waiter_signal
            .send((wait_over, duration_timeout))
            .await
            .map_err(|_| HotStuffError::SendError)
    }

    pub async fn stop_timer(&self, wait_over: T) -> Result<(), HotStuffError> {
        info!(
            target: LOG_TARGET,
            "Pacemaker: stop wait timer for value = {:?}", wait_over
        );
        self.tx_stop_waiter_signal
            .send(wait_over)
            .await
            .map_err(|_| HotStuffError::SendError)
    }

    pub async fn on_timeout(&mut self) -> Option<T> {
        self.rx_timeout_status.recv().await
    }
}

#[cfg(test)]
mod tests {
    use tari_shutdown::Shutdown;

    use super::*;

    #[tokio::test]
    async fn test_wait_timeout_pacemaker() {
        let shutdown = Shutdown::new();
        let mut handle = Pacemaker::spawn(shutdown.to_signal());

        handle.start_timer(0_u32, Duration::from_millis(10)).await.unwrap();
        let msg = handle.on_timeout().await.unwrap();
        handle.stop_timer(0_u32).await.unwrap();

        tokio::time::sleep(Duration::from_millis(11)).await;

        assert_eq!(msg, 0_u32);
    }

    #[tokio::test]
    async fn test_shutdown_pacemaker() {
        let shutdown = Shutdown::new();
        let mut handle = Pacemaker::spawn(shutdown.to_signal());

        // send start waiting signal to wait over
        handle.start_timer(1_u32, Duration::from_millis(10)).await.unwrap();

        // wait 1 millisecond
        tokio::time::sleep(Duration::from_millis(1)).await;

        // send shutdown signal for pacemaker
        handle.stop_timer(1).await.unwrap();

        assert!(tokio::time::timeout(Duration::from_millis(10), handle.on_timeout())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_wait_timeout_one_out_of_three_pacemaker() {
        let shutdown = Shutdown::new();
        let mut handle = Pacemaker::spawn(shutdown.to_signal());

        // send three wait signals
        handle.start_timer(0_u32, Duration::from_millis(10)).await.unwrap();
        handle.start_timer(1_u32, Duration::from_millis(10)).await.unwrap();
        handle.start_timer(2_u32, Duration::from_millis(10)).await.unwrap();

        tokio::time::sleep(Duration::from_millis(1)).await;

        // stop the middle waiter
        handle.stop_timer(1_u32).await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;

        // we should receive two WaitTimeOut status, for the first and last messages
        // the middle one was stopped, so we don't expect any further status messages
        // to be received
        let msg = handle.on_timeout().await.unwrap();
        assert_eq!(msg, 0);

        let msg = handle.on_timeout().await.unwrap();
        assert_eq!(msg, 2);

        // assert that we don't receive any further timeout message
        assert!(tokio::time::timeout(Duration::from_millis(10), handle.on_timeout())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn test_multiple_messages() {
        let shutdown = Shutdown::new();
        let mut handle = Pacemaker::spawn(shutdown.to_signal());

        // loop over start wait messages
        for i in 0..100 {
            handle.start_timer(i, Duration::from_millis(100)).await.unwrap();
        }

        tokio::time::sleep(Duration::from_millis(1)).await;

        // stop waiting messages that are indexed by even numbers
        for i in (0..100).filter(|i| i % 2 == 0) {
            handle.stop_timer(i).await.unwrap();
        }

        // assert that timeouts occur if and only if messages are indexed by odd numbers
        tokio::time::sleep(Duration::from_millis(100)).await;
        for i in (0..100).filter(|i| i % 2 == 1) {
            let val = handle.on_timeout().await.unwrap();
            assert_eq!(i, val)
        }

        // no more messages are received
        assert!(tokio::time::timeout(Duration::from_millis(100), handle.on_timeout())
            .await
            .is_err());
    }
}
