//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_dan_core::{models::Epoch, services::epoch_manager::EpochManager};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::{mpsc::Receiver, oneshot},
    task::JoinHandle,
};

use crate::{
    grpc::services::base_node_client::GrpcBaseNodeClient,
    p2p::services::epoch_manager::base_layer_epoch_manager::BaseLayerEpochManager,
};
// const LOG_TARGET: &str = "tari::validator_node::epoch_manager";

pub struct EpochManagerService {
    rx_request: Receiver<(
        EpochManagerRequest,
        oneshot::Sender<Result<EpochManagerResponse, String>>,
    )>,
    inner: BaseLayerEpochManager,
}

#[derive(Debug, Clone)]
pub enum EpochManagerRequest {
    CurrentEpoch,
}

pub enum EpochManagerResponse {
    CurrentEpoch { epoch: Epoch },
}

impl EpochManagerService {
    pub fn spawn(
        rx_request: Receiver<(
            EpochManagerRequest,
            oneshot::Sender<Result<EpochManagerResponse, String>>,
        )>,
        shutdown: ShutdownSignal,
        base_node_client: GrpcBaseNodeClient,
    ) -> JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            EpochManagerService {
                rx_request,
                inner: BaseLayerEpochManager { base_node_client },
            }
            .run(shutdown)
            .await
        })
    }

    pub async fn run(&mut self, mut shutdown: ShutdownSignal) -> Result<(), String> {
        loop {
            tokio::select! {
                Some((req, reply)) = self.rx_request.recv() => {
                    let _ignore = reply.send(self.handle_request(req).await);
                },
                _ = shutdown.wait() => {
                    dbg!("Shutting down epoch manager");
                    break;
                }
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, req: EpochManagerRequest) -> Result<EpochManagerResponse, String> {
        match req {
            EpochManagerRequest::CurrentEpoch => Ok(EpochManagerResponse::CurrentEpoch {
                epoch: self.inner.current_epoch().await,
            }),
        }
    }
}
