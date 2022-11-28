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
use tari_dan_core::{
    models::{vote_message::VoteMessage, HotStuffMessage, TariDanPayload},
    workers::events::{EventSubscription, HotStuffEvent},
};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStoreFactory;
use tari_shutdown::ShutdownSignal;
use tokio::sync::mpsc;

use crate::{
    p2p::services::{
        epoch_manager::handle::EpochManagerHandle,
        hotstuff::hotstuff_service::HotstuffService,
        mempool::MempoolHandle,
        messaging::OutboundMessaging,
        template_manager::TemplateManager,
    },
    payload_processor::TariDanPayloadProcessor,
    ValidatorNodeConfig,
};

pub fn try_spawn(
    node_identity: Arc<NodeIdentity>,
    config: &ValidatorNodeConfig,
    outbound: OutboundMessaging,
    epoch_manager: EpochManagerHandle,
    mempool: MempoolHandle,
    payload_processor: TariDanPayloadProcessor<TemplateManager>,
    rx_consensus_message: mpsc::Receiver<(CommsPublicKey, HotStuffMessage<TariDanPayload, CommsPublicKey>)>,
    rx_vote_message: mpsc::Receiver<(CommsPublicKey, VoteMessage)>,
    shutdown: ShutdownSignal,
) -> Result<(EventSubscription<HotStuffEvent>, SqliteShardStoreFactory), anyhow::Error> {
    let db = SqliteShardStoreFactory::try_create(config.state_db_path())?;

    let events = HotstuffService::spawn(
        node_identity,
        epoch_manager,
        mempool,
        outbound,
        payload_processor,
        db.clone(),
        rx_consensus_message,
        rx_vote_message,
        shutdown,
    );
    Ok((events, db))
}
