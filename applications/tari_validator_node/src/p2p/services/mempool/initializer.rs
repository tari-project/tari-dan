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
use tari_transaction::Transaction;
use tokio::{
    sync::{broadcast, mpsc},
    task,
    task::JoinHandle,
};

use crate::p2p::services::{
    epoch_manager::handle::EpochManagerHandle,
    mempool::{handle::MempoolHandle, service::MempoolService, validator::MempoolTransactionValidator},
    messaging::OutboundMessaging,
    template_manager::TemplateManager,
};

pub fn spawn(
    new_transactions: mpsc::Receiver<Transaction>,
    outbound: OutboundMessaging,
    epoch_manager: EpochManagerHandle,
    node_identity: Arc<NodeIdentity>,
    template_manager: TemplateManager,
) -> (MempoolHandle, JoinHandle<anyhow::Result<()>>) {
    let (tx_valid_transactions, rx_valid_transactions) = broadcast::channel(100);
    let (tx_mempool_request, rx_mempool_request) = mpsc::channel(1);

    let validator = MempoolTransactionValidator::new(template_manager);
    let mempool = MempoolService::new(
        new_transactions,
        rx_mempool_request,
        outbound,
        tx_valid_transactions,
        epoch_manager,
        node_identity,
        validator,
    );
    let handle = MempoolHandle::new(rx_valid_transactions, tx_mempool_request);

    let join_handle = task::spawn(mempool.run());

    (handle, join_handle)
}
