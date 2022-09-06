use log::*;
use tari_comms::types::CommsPublicKey;
use tari_dan_common_types::ShardId;
use tari_dan_core::{
    models::{vote_message::VoteMessage, HotStuffMessage, TariDanPayload},
    services::{leader_strategy::AlwaysFirstLeader, mempool::service::MempoolServiceHandle},
    workers::hotstuff_waiter::HotStuffWaiter,
};
use tari_shutdown::ShutdownSignal;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
};

use crate::p2p::services::epoch_manager::handle::EpochManagerHandle;

const LOG_TARGET: &str = "tari::validator_node::hotstuff_service";

pub struct HotstuffService {
    mempool: MempoolServiceHandle,
    tx_new: Sender<(TariDanPayload, ShardId)>,
    tx_hs_messages: Sender<(CommsPublicKey, HotStuffMessage<TariDanPayload, CommsPublicKey>)>,
    tx_votes: Sender<(CommsPublicKey, VoteMessage)>,
    rx_leader: Receiver<HotStuffMessage<TariDanPayload, CommsPublicKey>>,
    rx_broadcast: Receiver<(HotStuffMessage<TariDanPayload, CommsPublicKey>, Vec<CommsPublicKey>)>,
    rx_vote_message: Receiver<(VoteMessage, CommsPublicKey)>,
    rx_execute: Receiver<TariDanPayload>,
    shutdown: ShutdownSignal, // waiter: HotstuffWaiter,
}

impl HotstuffService {
    pub fn spawn(
        node_identity: CommsPublicKey,
        epoch_manager: EpochManagerHandle,
        mempool: MempoolServiceHandle,
        shutdown: ShutdownSignal,
    ) -> JoinHandle<Result<(), String>> {
        dbg!("Hotstuff starting");
        let (tx_new, rx_new) = channel(1);
        let (tx_hs_messages, rx_hs_messages) = channel(1);
        let (tx_votes, rx_votes) = channel(1);
        let (tx_leader, rx_leader) = channel(1);
        let (tx_broadcast, rx_broadcast) = channel(1);
        let (tx_vote_message, rx_vote_message) = channel(1);
        let (tx_execute, rx_execute) = channel(1);
        tokio::spawn(async move {
            let leader_strategy = AlwaysFirstLeader {};
            HotStuffWaiter::<TariDanPayload, _, _, _>::spawn(
                node_identity.clone(),
                epoch_manager,
                leader_strategy,
                rx_new,
                rx_hs_messages,
                rx_votes,
                tx_leader,
                tx_broadcast,
                tx_vote_message,
                tx_execute,
                shutdown.clone(),
            );

            let s = Self {
                mempool,
                tx_new,
                tx_hs_messages,
                tx_votes,
                rx_leader,
                rx_broadcast,
                rx_vote_message,
                rx_execute,
                shutdown,
            }
            .run()
            .await;
            Ok(())
        })
    }

    pub async fn run(mut self) -> Result<(), String> {
        dbg!("Main loop starting");
        loop {
            tokio::select! {
                new_tx = self.mempool.new_payload().recv() => {
                    match new_tx {
                       Ok(tx) => self.tx_new.send(tx).await.map_err(|e| format!("Could not send new tx:{}", e))?,
                        Err(e) => {
                            error!(target: LOG_TARGET, "Mempool event lagged:{}", e);
                        }
                    }
                },


                _ = self.shutdown.wait() => {
                    dbg!("Shutting down hs service");
                    break;
                }
            }
        }
        Ok(())
    }
}
