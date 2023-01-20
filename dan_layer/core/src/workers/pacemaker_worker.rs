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

use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use tari_dan_common_types::{PayloadId, ShardId};
use tari_shutdown::{Shutdown, ShutdownSignal};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task::JoinHandle,
};

use super::hotstuff_error::HotStuffError;
use crate::{
    models::HotstuffPhase,
    services::pacemaker::{Pacemaker, PacemakerSignal, PacemakerWaitStatus, WaitOver},
};

const LOG_TARGET: &str = "tari::dan_layer::pacemaker_worker";

pub struct Timer {}

impl Timer {
    fn new() -> Self {
        Self {}
    }

    async fn start_wait(&self, max_timeout: u64) {
        tokio::time::sleep(Duration::from_secs(max_timeout)).await;
    }
}

#[derive(Debug)]
pub struct LeaderFailurePacemaker {
    tx_waiter_status: Sender<(WaitOver, PacemakerWaitStatus)>,
    max_timeout: u64,
    status: HashMap<WaitOver, PacemakerWaitStatus>,
}

impl LeaderFailurePacemaker {
    pub fn spawn(
        tx_waiter_status: Sender<(WaitOver, PacemakerWaitStatus)>,
        wait_over: WaitOver,
        max_timeout: u64,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<Result<(), HotStuffError>> {
        let pacemaker = Self::new(tx_waiter_status, max_timeout);
        tokio::spawn(pacemaker.start_timer(wait_over, shutdown))
    }

    pub fn new(tx_waiter_status: Sender<(WaitOver, PacemakerWaitStatus)>, max_timeout: u64) -> Self {
        let status = HashMap::new();
        Self {
            tx_waiter_status,
            max_timeout,
            status,
        }
    }

    pub async fn start_timer(
        &mut self,
        wait_over: (PayloadId, ShardId, HotStuffPhase),
        shutdown: ShutdownSignal,
    ) -> Result<(), HotStuffError> {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from(self.max_timeout)).await => {
                let status = PacemakerWaitStatus::WaitTimeOut;
                self.status.insert(wait_over.clone(), status);
                self.tx_waiter_status.send((wait_over, status)).await;
            }
            _ = shutdown.wait() => {
                let status = PacemakerWaitStatus::ShutDown;
                self.status.insert(wait_over.clone(), status).await;
            }
        }
        Ok(())
    }
}
