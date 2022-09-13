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
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    message::DanMessage,
    models::{vote_message::VoteMessage, Committee, HotStuffMessage, Payload, TariDanPayload},
    services::{
        infrastructure_services::OutboundService,
        leader_strategy::{AlwaysFirstLeader, LeaderStrategy},
        TariDanPayloadProcessor,
    },
    workers::hotstuff_waiter::HotStuffWaiter,
};
use tari_dan_engine::instruction::Transaction;
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
};

use crate::p2p::services::{
    epoch_manager::handle::EpochManagerHandle,
    mempool::MempoolHandle,
    messaging::OutboundMessaging,
};

const LOG_TARGET: &str = "tari::validator_node::hotstuff_service";

pub struct HotstuffService {
    node_public_key: CommsPublicKey,
    mempool: MempoolHandle,
    outbound: OutboundMessaging,
    /// New incoming transaction from mempool
    tx_new: Sender<(TariDanPayload, ShardId)>,
    /// Outgoing leader new-view messages
    rx_leader: Receiver<HotStuffMessage<TariDanPayload, CommsPublicKey>>,
    /// Outgoing proposal messages to be broadcast by the leader to all replicas
    rx_broadcast: Receiver<(HotStuffMessage<TariDanPayload, CommsPublicKey>, Vec<CommsPublicKey>)>,
    /// Outgoing vote messages to be sent to the leader
    rx_vote_message: Receiver<(VoteMessage, CommsPublicKey)>,
    shutdown: ShutdownSignal, // waiter: HotstuffWaiter,
}

impl HotstuffService {
    pub fn spawn(
        node_public_key: CommsPublicKey,
        epoch_manager: EpochManagerHandle,
        mempool: MempoolHandle,
        outbound: OutboundMessaging,
        payload_processor: TariDanPayloadProcessor,
        rx_hotstuff_messages: Receiver<(CommsPublicKey, HotStuffMessage<TariDanPayload, CommsPublicKey>)>,
        rx_vote_messages: Receiver<(CommsPublicKey, VoteMessage)>,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<Result<(), anyhow::Error>> {
        dbg!("Hotstuff starting");
        let (tx_new, rx_new) = channel(1);
        let (tx_leader, rx_leader) = channel(1);
        let (tx_broadcast, rx_broadcast) = channel(1);
        let (tx_vote_message, rx_vote_message) = channel(1);

        let leader_strategy = AlwaysFirstLeader {};
        HotStuffWaiter::spawn(
            node_public_key.clone(),
            epoch_manager,
            leader_strategy,
            rx_new,
            rx_hotstuff_messages,
            rx_vote_messages,
            tx_leader,
            tx_broadcast,
            tx_vote_message,
            payload_processor,
            shutdown.clone(),
        );

        tokio::spawn(
            Self {
                node_public_key,
                mempool,
                outbound,
                tx_new,
                rx_leader,
                rx_broadcast,
                rx_vote_message,
                shutdown,
            }
            .run(),
        )
    }

    async fn handle_leader_message(
        &mut self,
        msg: HotStuffMessage<TariDanPayload, CommsPublicKey>,
    ) -> Result<(), anyhow::Error> {
        // TODO: who should decide the leader?
        let leader_strategy = AlwaysFirstLeader {};
        let committee = Committee::<CommsPublicKey>::new(vec![]);
        let leader = leader_strategy.get_leader(
            &committee,
            msg.new_view_payload().as_ref().unwrap().to_id(),
            msg.shard(),
            0, // round?
        );
        self.outbound
            .send(
                self.node_public_key.clone(),
                leader.clone(),
                DanMessage::HotStuffMessage(msg),
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
            .broadcast(self.node_public_key.clone(), &nodes, DanMessage::HotStuffMessage(msg))
            .await?;
        Ok(())
    }

    async fn handle_new_valid_transaction(&mut self, tx: Transaction, shard: ShardId) -> Result<(), anyhow::Error> {
        self.tx_new.send((TariDanPayload::new(tx), shard)).await?;
        Ok(())
    }

    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        dbg!("Main loop starting");
        loop {
            tokio::select! {
                // Inbound
               Some((tx, shard_id)) = self.mempool.next_valid_transaction() => log(self.handle_new_valid_transaction(tx, shard_id).await),

               // Outbound
               Some(msg) = self.rx_leader.recv() => log(self.handle_leader_message(msg).await),
               Some((msg, leader)) = self.rx_vote_message.recv() => log(self.handle_vote_message(leader, msg).await),
               Some((msg, dest_nodes)) = self.rx_broadcast.recv() => log(self.handle_broadcast_message(dest_nodes, msg).await),

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

fn log(result: Result<(), anyhow::Error>) {
    if let Err(e) = result {
        error!(target: LOG_TARGET, "Error in hotstuff service: {}", e);
    }
}
