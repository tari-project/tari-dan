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

use std::{fmt::Display, sync::Arc};

use log::*;
use tari_comms::{types::CommsPublicKey, NodeIdentity};
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    consensus_constants::ConsensusConstants,
    message::DanMessage,
    models::{vote_message::VoteMessage, HotStuffMessage, TariDanPayload},
    services::{
        infrastructure_services::OutboundService,
        leader_strategy::PayloadSpecificLeaderStrategy,
        NodeIdentitySigningService,
    },
    workers::{
        events::{EventSubscription, HotStuffEvent},
        hotstuff_waiter::{HotStuffWaiter, RecoveryMessage, NETWORK_LATENCY},
        pacemaker_worker::Pacemaker,
    },
};
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_shutdown::ShutdownSignal;
use tari_transaction::Transaction;
use tokio::sync::{
    broadcast,
    mpsc::{channel, Receiver, Sender},
};

use crate::{
    p2p::services::{
        epoch_manager::handle::EpochManagerHandle,
        mempool::MempoolHandle,
        messaging::OutboundMessaging,
        template_manager::TemplateManager,
    },
    payload_processor::TariDanPayloadProcessor,
};

const LOG_TARGET: &str = "tari::validator_node::hotstuff_service";

pub struct HotstuffService {
    node_public_key: CommsPublicKey,
    mempool: MempoolHandle,
    outbound: OutboundMessaging,
    /// New incoming transaction from mempool
    tx_new: Sender<(TariDanPayload, ShardId)>,
    /// Outgoing leader new-view messages
    rx_leader: Receiver<(CommsPublicKey, HotStuffMessage<TariDanPayload, CommsPublicKey>)>,
    /// Outgoing proposal messages to be broadcast by the leader to all replicas
    rx_broadcast: Receiver<(HotStuffMessage<TariDanPayload, CommsPublicKey>, Vec<CommsPublicKey>)>,
    /// Outgoing replica recovery response
    rx_recovery: Receiver<(RecoveryMessage, CommsPublicKey)>,
    /// Outgoing recovery request to be broadcast by all replicas to f+1 replicas in foreign committee
    rx_recovery_broadcast: Receiver<(RecoveryMessage, Vec<CommsPublicKey>)>,
    /// Outgoing vote messages to be sent to the leader
    rx_vote_message: Receiver<(VoteMessage, CommsPublicKey)>,
    shutdown: ShutdownSignal, // waiter: HotstuffWaiter,
}

impl HotstuffService {
    pub fn spawn(
        node_identity: Arc<NodeIdentity>,
        epoch_manager: EpochManagerHandle,
        mempool: MempoolHandle,
        outbound: OutboundMessaging,
        payload_processor: TariDanPayloadProcessor<TemplateManager>,
        shard_store_factory: SqliteShardStore,
        rx_hotstuff_messages: Receiver<(CommsPublicKey, HotStuffMessage<TariDanPayload, CommsPublicKey>)>,
        rx_recovery_messages: Receiver<(CommsPublicKey, RecoveryMessage)>,
        rx_vote_messages: Receiver<(CommsPublicKey, VoteMessage)>,
        shutdown: ShutdownSignal,
    ) -> EventSubscription<HotStuffEvent> {
        dbg!("Hotstuff starting");
        let (tx_new, rx_new) = channel(100);
        let (tx_leader, rx_leader) = channel(100);
        let (tx_broadcast, rx_broadcast) = channel(100);
        let (tx_recovery, rx_recovery) = channel(100);
        let (tx_recovery_broadcast, rx_recovery_broadcast) = channel(100);
        let (tx_vote_message, rx_vote_message) = channel(100);
        let (tx_events, _) = broadcast::channel(100);

        let leader_strategy = PayloadSpecificLeaderStrategy {};
        let consensus_constants = ConsensusConstants::devnet();
        let node_public_key = node_identity.public_key().clone();
        let pacemaker = Pacemaker::spawn(shutdown.clone());

        HotStuffWaiter::spawn(
            NodeIdentitySigningService::new(node_identity),
            // TODO: we still need this because The signing service is not generic. Abstracting signatures and public
            // keys may add a lot of type complexity.
            node_public_key.clone(),
            epoch_manager,
            leader_strategy,
            rx_new,
            rx_hotstuff_messages,
            rx_recovery_messages,
            rx_vote_messages,
            tx_leader,
            tx_broadcast,
            tx_recovery,
            tx_recovery_broadcast,
            tx_vote_message,
            tx_events.clone(),
            pacemaker,
            payload_processor,
            shard_store_factory,
            shutdown.clone(),
            consensus_constants,
            NETWORK_LATENCY,
        );

        tokio::spawn(
            Self {
                node_public_key,
                mempool,
                outbound,
                tx_new,
                rx_leader,
                rx_broadcast,
                rx_recovery,
                rx_recovery_broadcast,
                rx_vote_message,
                shutdown,
            }
            .run(),
        );

        EventSubscription::new(tx_events)
    }

    async fn handle_leader_message(
        &mut self,
        to: CommsPublicKey,
        msg: HotStuffMessage<TariDanPayload, CommsPublicKey>,
    ) -> Result<(), anyhow::Error> {
        trace!(target: LOG_TARGET, "Sending leader message to {}", to);
        self.outbound
            .send(
                self.node_public_key.clone(),
                to,
                DanMessage::HotStuffMessage(Box::new(msg)),
            )
            .await?;
        Ok(())
    }

    async fn handle_vote_message(&mut self, leader: CommsPublicKey, msg: VoteMessage) -> Result<(), anyhow::Error> {
        self.outbound
            .send(self.node_public_key.clone(), leader, DanMessage::VoteMessage(msg))
            .await?;
        Ok(())
    }

    async fn handle_broadcast_message(
        &mut self,
        nodes: Vec<CommsPublicKey>,
        msg: HotStuffMessage<TariDanPayload, CommsPublicKey>,
    ) -> Result<(), anyhow::Error> {
        self.outbound
            .broadcast(
                self.node_public_key.clone(),
                &nodes,
                DanMessage::HotStuffMessage(Box::new(msg)),
            )
            .await?;
        Ok(())
    }

    async fn handle_recovery_message(&mut self, msg: RecoveryMessage, to: CommsPublicKey) -> Result<(), anyhow::Error> {
        trace!(target: LOG_TARGET, "Sending leader message to {}", to);
        self.outbound
            .send(
                self.node_public_key.clone(),
                to,
                DanMessage::RecoveryMessage(Box::new(msg)),
            )
            .await?;
        Ok(())
    }

    async fn handle_recovery_broadcast_message(
        &mut self,
        msg: RecoveryMessage,
        nodes: Vec<CommsPublicKey>,
    ) -> Result<(), anyhow::Error> {
        self.outbound
            .broadcast(
                self.node_public_key.clone(),
                &nodes,
                DanMessage::RecoveryMessage(Box::new(msg)),
            )
            .await?;
        Ok(())
    }

    async fn handle_new_valid_transaction(&mut self, tx: Transaction, shard: ShardId) -> Result<(), anyhow::Error> {
        self.tx_new.send((TariDanPayload::new(tx), shard)).await?;
        Ok(())
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        loop {
            tokio::select! {
                // Inbound
                res = self.mempool.next_valid_transaction() => {
                    if let Some((tx, shard_id)) = log(res, "new valid transaction") {
                        debug!(target: LOG_TARGET, "Received new transaction {} for shard {}", tx.hash(), shard_id);
                        log(self.handle_new_valid_transaction(tx, shard_id).await, "new valid transaction");
                    }
                }
                // Outbound
                Some((to, msg)) = self.rx_leader.recv() => {
                    debug!(target: LOG_TARGET, "Received leader message: {}", &msg);
                    log(self.handle_leader_message(to, msg).await, "leader message");
                }
                Some((msg, leader)) = self.rx_vote_message.recv() => {
                    debug!(target: LOG_TARGET, "Received vote message");
                    log(self.handle_vote_message(leader, msg).await, "vote message");
                }
                Some((msg, dest_nodes)) = self.rx_broadcast.recv() => {
                    debug!(target: LOG_TARGET, "Received broadcast message: {}", &msg);
                    log(self.handle_broadcast_message(dest_nodes, msg).await, "broadcast message");
                }
                Some((msg, replica)) = self.rx_recovery.recv() =>{
                    debug!(target: LOG_TARGET, "Received replica recovery response: {:?}", &msg);
                    log(self.handle_recovery_message(msg, replica).await, "replice recovery response");
                }
                Some((msg, replicas)) = self.rx_recovery_broadcast.recv() =>{
                    debug!(target: LOG_TARGET, "Received broadcast recovery request: {:?}", &msg);
                    log(self.handle_recovery_broadcast_message(msg, replicas).await, "broadcast recovery request");
                }
                // Shutdown
                _ = self.shutdown.wait() => {
                    dbg!("Shutting down hs service");
                    break;
                }
            }
        }
        Ok(())
    }
}

fn log<T, E: Display>(result: Result<T, E>, area: &str) -> Option<T> {
    match result {
        Ok(t) => Some(t),
        Err(e) => {
            error!(target: LOG_TARGET, "Error in hotstuff service: {} [{}]", e, area);
            None
        },
    }
}
