//  Copyright 2021. The Tari Project
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

use log::*;
use tari_common::exit_codes::ExitError;
use tari_dan_core::workers::events::HotStuffEvent;
use tari_shutdown::ShutdownSignal;
use tari_template_lib::Hash;

use crate::{p2p::services::networking::NetworkingService, Services};

const LOG_TARGET: &str = "tari::validator_node::dan_node";

pub struct DanNode {
    services: Services,
}

impl DanNode {
    pub fn new(services: Services) -> Self {
        Self { services }
    }

    pub async fn start(mut self, mut shutdown: ShutdownSignal) -> Result<(), ExitError> {
        self.services.networking.announce().await?;

        let mut hotstuff_events = self.services.hotstuff_events.subscribe();

        loop {
            tokio::select! {
                // Wait until killed
                _ = shutdown.wait() => {
                     break;
                },

                Ok(event) = hotstuff_events.recv() => {
                    if let HotStuffEvent::OnFinalized(qc, _) = event {
                        let transaction_hash = Hash::from(qc.payload_id().into_array());
                        info!(target: LOG_TARGET, "🏁 Removing finalized transaction {} from mempool", transaction_hash);
                        if let Err(err) = self.services.mempool.remove_transaction(transaction_hash).await {
                            error!(target: LOG_TARGET, "Failed to remove transaction from mempool: {}", err);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
