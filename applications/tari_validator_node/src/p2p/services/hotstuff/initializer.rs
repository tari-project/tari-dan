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

use std::sync::Arc;

use tari_comms::{types::CommsPublicKey, NodeIdentity};
use tari_dan_app_utilities::epoch_manager::EpochManagerHandle;
use tari_dan_core::{
    models::{vote_message::VoteMessage, HotStuffMessage, TariDanPayload},
    workers::hotstuff_waiter::RecoveryMessage,
};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_shutdown::ShutdownSignal;
use tokio::sync::mpsc;

use super::hotstuff_service::HotstuffServiceSpawnOutput;
use crate::{
    p2p::services::{
        hotstuff::hotstuff_service::HotstuffService, mempool::MempoolHandle, messaging::OutboundMessaging,
        template_manager::TemplateManager,
    },
    payload_processor::TariDanPayloadProcessor,
};

pub fn try_spawn(
    node_identity: Arc<NodeIdentity>,
    shard_store: SqliteShardStore,
    outbound: OutboundMessaging,
    epoch_manager: EpochManagerHandle,
    mempool: MempoolHandle,
    payload_processor: TariDanPayloadProcessor<TemplateManager>,
    rx_consensus_message: mpsc::Receiver<(CommsPublicKey, HotStuffMessage<TariDanPayload, CommsPublicKey>)>,
    rx_recovery_message: mpsc::Receiver<(CommsPublicKey, RecoveryMessage)>,
    rx_vote_message: mpsc::Receiver<(CommsPublicKey, VoteMessage)>,
    shutdown: ShutdownSignal,
) -> HotstuffServiceSpawnOutput {
    HotstuffService::spawn(
        node_identity,
        epoch_manager,
        mempool,
        outbound,
        payload_processor,
        shard_store,
        rx_consensus_message,
        rx_recovery_message,
        rx_vote_message,
        shutdown,
    )
}
