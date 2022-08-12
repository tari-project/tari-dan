use std::collections::HashMap;

use tari_common_types::types::FixedHash;
use tari_dan_engine::instruction::Instruction;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
};

use crate::{
    models::{HotStuffMessage, HotStuffMessageType, Payload, QuorumCertificate, TreeNodeHash, ViewId},
    services::infrastructure_services::NodeAddressable,
};

pub struct ShardDb {
    shard_qcs: HashMap<u32, QuorumCertificate>,
}

impl ShardDb {
    pub fn new() -> Self {
        ShardDb {
            shard_qcs: HashMap::new(),
        }
    }

    pub fn get_high_qc_for(&self, shard: u32) -> QuorumCertificate {
        if let Some(qc) = self.shard_qcs.get(&shard) {
            qc.clone()
        } else {
            QuorumCertificate::genesis(TreeNodeHash::zero())
        }
    }
}

pub struct HotStuffWaiter<TPayload: Payload, TAddr: NodeAddressable> {
    rx_new: Receiver<TPayload>,
    rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload>)>,
    tx_leader: Sender<HotStuffMessage<TPayload>>,
    tx_broadcast: Sender<(HotStuffMessage<TPayload>, Vec<TAddr>)>,
    shard_db: ShardDb,
}

impl<TPayload: Payload + 'static, TAddr: NodeAddressable + 'static> HotStuffWaiter<TPayload, TAddr> {
    pub fn spawn(
        rx_new: Receiver<TPayload>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload>)>,
        tx_leader: Sender<HotStuffMessage<TPayload>>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload>, Vec<TAddr>)>,
        rx_shutdown: Receiver<()>,
    ) -> JoinHandle<Result<(), String>> {
        tokio::spawn(async move {
            HotStuffWaiter::<TPayload, TAddr>::new(rx_new, rx_hs_message, tx_leader, tx_broadcast)
                .run(rx_shutdown)
                .await
        })
    }

    pub fn new(
        rx_new: Receiver<TPayload>,
        rx_hs_message: Receiver<(TAddr, HotStuffMessage<TPayload>)>,
        tx_leader: Sender<HotStuffMessage<TPayload>>,
        tx_broadcast: Sender<(HotStuffMessage<TPayload>, Vec<TAddr>)>,
    ) -> Self {
        Self {
            rx_new,
            rx_hs_message,
            tx_leader,
            tx_broadcast,
            shard_db: ShardDb::new(),
        }
    }

    fn get_highest_qc(&self, state_key: u32) -> QuorumCertificate {
        self.shard_db.get_high_qc_for(state_key)
    }

    fn on_receive_new_view(&mut self, from: TAddr, qc: &QuorumCertificate) -> Result<(), String> {
        // TODO: Validate who message is from
        self.validate_from_for_new_view(from);
        self.validate_qc(qc);
        dbg!("update qc");
        Ok(())
    }

    fn validate_from_for_new_view(&self, from: TAddr) -> Result<(), String> {
        // Validate that from is in the correct committee
        todo!()
    }

    fn validate_qc(&self, qc: &QuorumCertificate) -> Result<(), String> {
        todo!()
    }

    pub async fn run(mut self, mut rx_shutdown: Receiver<()>) -> Result<(), String> {
        loop {
            tokio::select! {
                                msg = self.rx_new.recv() => {
                                    dbg!("new payload received");

                                     // get state
                                     let high_qc = self.get_highest_qc(0);
                                    // send to leader

                                    let new_view = HotStuffMessage::new_view(high_qc, ViewId(0), 1);

                                    self.tx_leader.send(new_view).await.map_err(|e| e.to_string())?;
                                },
                            msg = self.rx_hs_message.recv() => {
                                 if let Some((from, msg) ) = msg {
                            dbg!("Hotstuff received");
                            dbg!(&msg);

                        match msg.message_type() {
                            HotStuffMessageType::NewView => {
                        self.on_receive_new_view(from, msg.high_qc().as_ref().unwrap());
                    },
                            _ => todo!()
                        }

            }
                            },
                                           _ = rx_shutdown.recv() => {
                                        dbg!("Exiting");
                                        break;
                                    }
                                }
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_receives_new_payload_starts_new_chain() {
    let (tx_new, rx_new) = channel(1);
    let (tx_hs_messages, rx_hs_messages) = channel(1);
    let (tx_leader, mut rx_leader) = channel(1);
    let (tx_shutdown, rx_shutdown) = channel(1);
    let (tx_broadcast, rx_broadcast) = channel(1);
    let instance =
        HotStuffWaiter::<String, String>::spawn(rx_new, rx_hs_messages, tx_leader, tx_broadcast, rx_shutdown);

    let new_payload = "Hello world".to_string();
    tx_new.send(new_payload).await.unwrap();
    let leader_message = rx_leader.recv().await.expect("Did not receive leader message");
    dbg!(leader_message);
    tx_shutdown.send(()).await.unwrap();
    //     let leader_message = rx_leader.recv().await;
    //     dbg!(leader_message);
    instance.await.expect("did not end cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_proposes() {
    let (tx_new, rx_new) = channel(1);
    let (tx_hs_messages, rx_hs_messages) = channel(1);
    let (tx_leader, mut rx_leader) = channel(1);
    let (tx_broadcast, mut rx_broadcast) = channel(1);
    let (tx_shutdown, rx_shutdown) = channel(1);
    let instance =
        HotStuffWaiter::<String, String>::spawn(rx_new, rx_hs_messages, tx_leader, tx_broadcast, rx_shutdown);

    let node1 = "node1".to_string();

    // Send a new view message
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(TreeNodeHash::zero()), ViewId(0), 1);

    tx_hs_messages.send((node1, new_view_message)).await.unwrap();

    // should receive a broadcast proposal
    let proposal_message = rx_broadcast.try_recv().expect("Did not receive proposal");
    tx_shutdown.send(()).await.unwrap();
    //     let leader_message = rx_leader.recv().await;
    //     dbg!(leader_message);
    instance.await.expect("did not end cleanly");
}
