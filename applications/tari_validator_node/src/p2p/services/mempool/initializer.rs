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

use std::sync::Arc;

use tari_comms::NodeIdentity;
use tari_dan_app_utilities::epoch_manager::EpochManagerHandle;
use tari_dan_common_types::ShardId;
use tari_transaction::Transaction;
use tokio::{
    sync::{broadcast, mpsc},
    task,
    task::JoinHandle,
};

use crate::{
    dry_run_transaction_processor::DryRunTransactionProcessor,
    p2p::services::{
        mempool::{handle::MempoolHandle, service::MempoolService, MempoolError, Validator},
        messaging::OutboundMessaging,
    },
};

pub fn spawn<TValidator>(
    new_transactions: mpsc::Receiver<Transaction>,
    outbound: OutboundMessaging,
    epoch_manager: EpochManagerHandle,
    node_identity: Arc<NodeIdentity>,
    validator: TValidator,
    dry_run_processor: DryRunTransactionProcessor,
    tx_new_valid_transaction: mpsc::Sender<(Transaction, ShardId)>,
) -> (MempoolHandle, JoinHandle<anyhow::Result<()>>)
where
    TValidator: Validator<Transaction, Error = MempoolError> + Send + Sync + 'static,
{
    let (tx_mempool_request, rx_mempool_request) = mpsc::channel(1);

    let mempool = MempoolService::new(
        new_transactions,
        rx_mempool_request,
        outbound,
        epoch_manager,
        node_identity,
        validator,
        dry_run_processor,
        tx_new_valid_transaction,
    );
    let handle = MempoolHandle::new(tx_mempool_request);

    let join_handle = task::spawn(mempool.run());

    (handle, join_handle)
}
