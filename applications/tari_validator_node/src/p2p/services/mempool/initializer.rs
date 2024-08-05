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

use log::*;
use tari_dan_common_types::{NumPreshards, PeerAddress};
use tari_epoch_manager::base_layer::EpochManagerHandle;
use tari_state_store_sqlite::SqliteStateStore;
use tari_transaction::Transaction;
use tokio::{sync::mpsc, task, task::JoinHandle};

#[cfg(feature = "metrics")]
use super::metrics::PrometheusMempoolMetrics;
use crate::{
    consensus::ConsensusHandle,
    p2p::services::{
        mempool::{handle::MempoolHandle, service::MempoolService},
        messaging::Gossip,
    },
    transaction_validators::TransactionValidationError,
    validator::Validator,
};

const LOG_TARGET: &str = "tari::dan::validator_node::mempool";

pub fn spawn<TValidator>(
    num_preshards: NumPreshards,
    gossip: Gossip,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    transaction_validator: TValidator,
    state_store: SqliteStateStore<PeerAddress>,
    consensus_handle: ConsensusHandle,
    #[cfg(feature = "metrics")] metrics_registry: &prometheus::Registry,
) -> (MempoolHandle, JoinHandle<anyhow::Result<()>>)
where
    TValidator: Validator<Transaction, Context = (), Error = TransactionValidationError> + Send + Sync + 'static,
{
    // This channel only needs to be size 1, because each mempool request must wait for a reply and the mempool is
    // running on a single task and so there is no benefit to buffering multiple requests.
    let (tx_mempool_request, rx_mempool_request) = mpsc::channel(1);

    #[cfg(feature = "metrics")]
    let metrics = PrometheusMempoolMetrics::new(metrics_registry);
    let mempool = MempoolService::new(
        num_preshards,
        rx_mempool_request,
        gossip,
        epoch_manager,
        transaction_validator,
        state_store,
        consensus_handle,
        #[cfg(feature = "metrics")]
        metrics,
    );
    let handle = MempoolHandle::new(tx_mempool_request);

    let join_handle = task::spawn(mempool.run());
    debug!(target: LOG_TARGET, "Spawning mempool service (task: {:?})", join_handle);

    (handle, join_handle)
}
