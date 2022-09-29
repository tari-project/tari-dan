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

use std::{collections::HashMap, convert::TryFrom, time::Duration};

use lazy_static::lazy_static;
use tari_common_types::types::PrivateKey;
use tari_dan_common_types::ShardId;
use tari_dan_engine::{
    instruction::{Instruction, InstructionProcessor, TransactionBuilder},
    packager::PackageBuilder,
    runtime::{RuntimeInterfaceImpl, StateTracker},
    state_store::memory::MemoryStateStore,
    wasm::compile::compile_str,
};
use tari_shutdown::Shutdown;
use tari_utilities::ByteArray;
use tokio::{
    sync::{
        broadcast,
        mpsc::{channel, Receiver, Sender},
    },
    task::JoinHandle,
    time::timeout,
};

use crate::{
    models::{
        vote_message::VoteMessage,
        HotStuffMessage,
        ObjectPledge,
        Payload,
        QuorumCertificate,
        QuorumDecision,
        TariDanPayload,
    },
    services::{
        epoch_manager::{EpochManager, RangeEpochManager},
        infrastructure_services::NodeAddressable,
        leader_strategy::{AlwaysFirstLeader, LeaderStrategy},
    },
    workers::hotstuff_waiter::HotStuffWaiter,
    DigitalAssetError,
};

pub struct PayloadProcessorListener<TPayload> {
    receiver: broadcast::Receiver<(TPayload, HashMap<ShardId, Vec<ObjectPledge>>)>,
    sender: broadcast::Sender<(TPayload, HashMap<ShardId, Vec<ObjectPledge>>)>,
}

impl<TPayload: Payload> PayloadProcessorListener<TPayload> {
    pub fn new() -> Self {
        let (sender, receiver) = broadcast::channel(100);
        Self { receiver, sender }
    }
}

impl<TPayload: Payload> PayloadProcessor<TPayload> for PayloadProcessorListener<TPayload> {
    fn process_payload(
        &self,
        payload: &TPayload,
        pledges: HashMap<ShardId, Vec<ObjectPledge>>,
    ) -> Result<(), DigitalAssetError> {
        self.sender
            .send((payload.clone(), pledges))
            .map_err(|_e| DigitalAssetError::SendError {
                context: "Sending process payload".to_string(),
            })?;
        Ok(())
    }
}

pub struct NullPayloadProcessor {}

impl<TPayload: Payload> PayloadProcessor<TPayload> for NullPayloadProcessor {
    fn process_payload(
        &self,
        _payload: &TPayload,
        _pledges: HashMap<ShardId, Vec<ObjectPledge>>,
    ) -> Result<(), DigitalAssetError> {
        Ok(())
    }
}

pub trait Consensus<TPayload: Payload> {
    fn execute_transaction(
        &mut self,
        payload: TPayload,
        inputs: Vec<ObjectPledge>,
        outputs: Vec<ObjectPledge>,
    ) -> Result<(), String>;
}

pub struct HsTestHarness<TPayload, TAddr> {
    identity: TAddr,
    tx_new: Sender<(TPayload, ShardId)>,
    tx_hs_messages: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    rx_leader: Receiver<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    shutdown: Shutdown,
    rx_broadcast: Receiver<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
    rx_vote_message: Receiver<(VoteMessage, TAddr)>,
    tx_votes: Sender<(TAddr, VoteMessage)>,
    rx_execute: broadcast::Receiver<(TPayload, HashMap<ShardId, Vec<ObjectPledge>>)>,
    hs_waiter: Option<JoinHandle<Result<(), HotStuffError>>>,
}
impl<TPayload, TAddr> HsTestHarness<TPayload, TAddr>
where
    TPayload: Payload + 'static,
    TAddr: NodeAddressable + 'static,
{
    pub fn new<TEpochManager, TLeader>(identity: TAddr, epoch_manager: TEpochManager, leader: TLeader) -> Self
    where
        TEpochManager: EpochManager<TAddr> + Send + Sync + 'static,
        TLeader: LeaderStrategy<TAddr> + Send + Sync + 'static,
    {
        let (tx_new, rx_new) = channel(1);
        let (tx_hs_messages, rx_hs_messages) = channel(1);
        let (tx_leader, rx_leader) = channel(1);
        let (tx_broadcast, rx_broadcast) = channel(1);
        let (tx_vote_message, rx_vote_message) = channel(1);
        let (tx_votes, rx_votes) = channel(1);
        let payload_processor = PayloadProcessorListener::<TPayload>::new();
        let rx_execute = payload_processor.receiver.resubscribe();
        let shutdown = Shutdown::new();

        let shard_store = MemoryShardStoreFactory::new();
        let hs_waiter = HotStuffWaiter::spawn(
            identity.clone(),
            epoch_manager,
            leader,
            rx_new,
            rx_hs_messages,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_vote_message,
            payload_processor,
            shard_store,
            shutdown.to_signal(),
        );
        Self {
            identity,
            tx_new,
            tx_hs_messages,
            rx_leader,
            shutdown,
            rx_broadcast,
            rx_vote_message,
            tx_votes,
            rx_execute,
            hs_waiter: Some(hs_waiter),
        }
    }

    fn identity(&self) -> TAddr {
        self.identity.clone()
    }

    async fn assert_shuts_down_safely(&mut self) {
        self.shutdown.trigger();
        self.hs_waiter
            .take()
            .unwrap()
            .await
            .expect("did not end cleanly")
            .unwrap();
    }

    async fn recv_broadcast(&mut self) -> (HotStuffMessage<TPayload, TAddr>, Vec<TAddr>) {
        if let Some(msg) = timeout(Duration::from_secs(10), self.rx_broadcast.recv())
            .await
            .expect("timed out")
        {
            msg
        } else {
            // Otherwise there are no senders, meaning the main loop has shut down,
            // so try shutdown to get the actual error
            self.assert_shuts_down_safely().await;
            panic!("Shut down safely, but still received none");
        }
    }

    async fn recv_vote_message(&mut self) -> (VoteMessage, TAddr) {
        if let Some(msg) = timeout(Duration::from_secs(10), self.rx_vote_message.recv())
            .await
            .expect("timed out")
        {
            msg
        } else {
            // Otherwise there are no senders, meaning the main loop has shut down,
            // so try shutdown to get the actual error
            self.assert_shuts_down_safely().await;
            panic!("Shut down safely, but still received none");
        }
    }

    async fn recv_execute(&mut self) -> (TPayload, HashMap<ShardId, Vec<ObjectPledge>>) {
        if let Ok(msg) = timeout(Duration::from_secs(10), self.rx_execute.recv())
            .await
            .expect("timed out")
        {
            msg
        } else {
            // Otherwise there are no senders, meaning the main loop has shut down,
            // so try shutdown to get the actual error
            self.assert_shuts_down_safely().await;
            panic!("Shut down safely, but still received none");
        }
    }

    async fn assert_no_execute(&mut self) {
        assert!(
            timeout(Duration::from_secs(1), self.rx_execute.recv()).await.is_err(),
            "received an execute when we weren't expecting it"
        )
    }
}

lazy_static! {
    static ref SHARD0: ShardId = ShardId::zero();
    static ref SHARD1: ShardId = ShardId([1u8; 32]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_receives_new_payload_starts_new_chain() {
    let node1 = "node1".to_string();

    let epoch_manager = RangeEpochManager::new(*SHARD0..*SHARD1, vec![node1.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});

    let new_payload = ("Hello world".to_string(), vec![*SHARD0]);
    instance.tx_new.send((new_payload, *SHARD0)).await.unwrap();
    let leader_message = instance.rx_leader.recv().await.expect("Did not receive leader message");
    dbg!(leader_message);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_proposes() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let epoch_manager = RangeEpochManager::new(*SHARD0..*SHARD1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![*SHARD0]);

    dbg!(payload.to_id());
    // Send a new view message
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *SHARD0, Some(payload));

    instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message))
        .await
        .unwrap();

    let (_, broadcast_group) = instance.recv_broadcast().await;

    assert_eq!(broadcast_group, vec![node1, node2]);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_replica_sends_vote_for_proposal() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let epoch_manager = RangeEpochManager::new(*SHARD0..*SHARD1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![*SHARD0]);
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *SHARD0, Some(payload));

    // Node 2 sends new view to node 1
    instance
        .tx_hs_messages
        .send((node2, new_view_message.clone()))
        .await
        .unwrap();

    // Should receive a proposal
    let (proposal_message, _broadcast_group) = instance.recv_broadcast().await;

    // Forward the proposal back to itself
    instance
        .tx_hs_messages
        .send((node1.clone(), proposal_message))
        .await
        .expect("Should not error");

    let (vote, from) = instance.recv_vote_message().await;

    dbg!(vote);
    assert_eq!(from, node1);
    // todo!("assert values");
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_sends_new_proposal_when_enough_votes_are_received() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let epoch_manager = RangeEpochManager::new(*SHARD0..*SHARD1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![*SHARD0]);

    // Start a new view
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *SHARD0, Some(payload));
    instance
        .tx_hs_messages
        .send((node2.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal_message, _broadcast_group) = instance.recv_broadcast().await;

    // tx_hs_messages
    //     .send((node1.clone(), proposal_message))
    //     .await
    //     .expect("Should not error");

    let vote_hash = proposal_message.node().unwrap().hash();

    // Create some votes
    let mut vote = VoteMessage::new(*vote_hash, *SHARD0, QuorumDecision::Accept, Default::default());

    vote.sign();
    instance.tx_votes.send((node1, vote.clone())).await.unwrap();

    // Should get no proposal
    assert!(
        timeout(Duration::from_secs(1), instance.rx_broadcast.recv())
            .await
            .is_err(),
        "received a proposal when we weren't expecting it"
    );

    // Send another vote
    let mut vote = VoteMessage::new(*vote_hash, *SHARD0, QuorumDecision::Accept, Default::default());
    vote.sign();
    instance.tx_votes.send((node2, vote)).await.unwrap();

    // should get a proposal

    let (proposal2, _broadcast_group) = instance.recv_broadcast().await;

    let proposed_node = proposal2.node().expect("Should have a node attached");

    assert_eq!(proposed_node.justify().local_node_hash(), *vote_hash);

    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_execute_called_when_consensus_reached() {
    let node1 = "node1".to_string();
    let epoch_manager = RangeEpochManager::new(*SHARD0..*SHARD1, vec![node1.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![*SHARD0]);

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *SHARD0, Some(payload.clone()));
    instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal1, _broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal1)).await.unwrap();
    let (vote, _) = instance.recv_vote_message().await;
    // loopback the vote
    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();
    let (proposal2, _broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal2)).await.unwrap();
    let (vote, _) = instance.recv_vote_message().await;

    // No execute yet
    instance.assert_no_execute().await;

    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal3, _broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal3)).await.unwrap();
    let (vote, _) = instance.recv_vote_message().await;

    // No execute yet
    instance.assert_no_execute().await;
    // loopback the vote
    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal4, _broadcast_group) = instance.recv_broadcast().await;

    dbg!(&proposal4);
    instance.tx_hs_messages.send((node1.clone(), proposal4)).await.unwrap();
    let (vote, _) = instance.recv_vote_message().await;
    dbg!(&vote);

    // // No execute yet
    // assert!(
    //     timeout(Duration::from_secs(1), rx_execute.recv()).await.is_err(),
    //     "received an execute when we weren't expecting it"
    // );
    //
    // tx_votes.send((node1, vote.clone())).await.unwrap();
    //
    let executed_payload = timeout(Duration::from_secs(10), instance.rx_execute.recv())
        .await
        .expect("timed out")
        .expect("Should not be None");
    // executed_payload.2.send(HashMap::new()).unwrap();

    assert_eq!(executed_payload.0, payload);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_multishard_votes() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let shard0_committee = vec![node1.clone()];
    let shard1_committee = vec![node2.clone()];
    let epoch_manager = RangeEpochManager::new_with_multiple(&[
        (*SHARD0..*SHARD1, shard0_committee),
        (*SHARD1..ShardId([2u8; 32]), shard1_committee),
    ]);
    let mut node1_instance = HsTestHarness::new(node1.clone(), epoch_manager.clone(), AlwaysFirstLeader {});
    let mut node2_instance = HsTestHarness::new(node2.clone(), epoch_manager, AlwaysFirstLeader {});

    let payload = ("Hello World".to_string(), vec![*SHARD0, *SHARD1]);

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *SHARD0, Some(payload.clone()));
    node1_instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *SHARD1, Some(payload.clone()));
    node2_instance
        .tx_hs_messages
        .send((node2.clone(), new_view_message.clone()))
        .await
        .unwrap();

    let (proposal1_n1, _broadcast_group) = node1_instance.recv_broadcast().await;
    // loopback the proposal to all nodes
    node1_instance
        .tx_hs_messages
        .send((node1.clone(), proposal1_n1.clone()))
        .await
        .unwrap();
    node2_instance
        .tx_hs_messages
        .send((node1.clone(), proposal1_n1))
        .await
        .unwrap();

    // Node 2 also proposes
    let (proposal1_n2, _broadcast_group) = node2_instance.recv_broadcast().await;
    // loopback the proposal to all nodes
    node1_instance
        .tx_hs_messages
        .send((node1.clone(), proposal1_n2.clone()))
        .await
        .unwrap();
    node2_instance
        .tx_hs_messages
        .send((node1.clone(), proposal1_n2))
        .await
        .unwrap();

    // Should get a vote from n1 and n2
    let (vote1_n1, _) = node1_instance.recv_vote_message().await;
    let (vote1_n2, _) = node2_instance.recv_vote_message().await;

    // Loop back the votes to each leader
    node1_instance
        .tx_votes
        .send((node1.clone(), vote1_n1.clone()))
        .await
        .unwrap();
    node2_instance
        .tx_votes
        .send((node2.clone(), vote1_n2.clone()))
        .await
        .unwrap();

    // get a proposal from each
    let (_proposal2_n1, _broadcast_group) = node1_instance.recv_broadcast().await;
    let (_proposal2_n2, _broadcast_group) = node2_instance.recv_broadcast().await;

    node1_instance.assert_shuts_down_safely().await;
    node2_instance.assert_shuts_down_safely().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Test may be implemented in future"]
async fn test_hs_waiter_leader_starts_view_with_n_minus_f_new_view() {
    // TODO: I don't know if this is a requirement, the prepare step might actually be fine
    // let (tx_new, rx_new) = channel(1);
    // let (tx_hs_messages, rx_hs_messages) = channel(1);
    // let (tx_leader, mut rx_leader) = channel(1);
    // let (tx_broadcast, mut rx_broadcast) = channel(1);
    // let (tx_shutdown, rx_shutdown) = channel(1);
    // let node1 = "node1".to_string();
    // let node2 = "node2".to_string();
    // let node3 = "node3".to_string();
    // let node4 = "node4".to_string();
    // let committee = Committee::new(vec![node1.clone(), node2.clone(), node3.clone(), node4.clone()]);
    //
    // let instance = HotStuffWaiter::<String, String>::spawn(
    //     node3.clone(),
    //     committee,
    //     rx_new,
    //     rx_hs_messages,
    //     tx_leader,
    //     tx_broadcast,
    //     rx_shutdown,
    // );
    // let payload = "Hello World".to_string();
    //
    // Send a new view message
    // let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(0), 0, Some(payload));
    //
    // tx_hs_messages.send((node1, new_view_message.clone())).await.unwrap();
    // tx_hs_messages.send((node2, new_view_message.clone())).await.unwrap();

    // should receive a broadcast proposal
    // let proposal_message = rx_broadcast.try_recv().expect("Did not receive proposal");
    // assert!(
    //     timeout(Duration::from_secs(1), rx_broadcast.recv()).await.is_err(),
    //     "Leader should not have proposed until it's received 3 messages"
    // );

    // Technically the leader will send this to themselves
    // tx_hs_messages.send((node3, new_view_message)).await.unwrap();

    // Now we should receive the proposal
    // let proposal_message = timeout(Duration::from_secs(10), rx_broadcast.recv())
    //     .await
    //     .expect("timed out");

    // tx_shutdown.send(()).await.unwrap();
    // instance.await.expect("did not end cleanly");
    todo!()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Test may be implemented in future"]
async fn test_hs_waiter_non_committee_member_does_not_start_new_view() {
    todo!()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Test may be implemented in future"]
async fn test_hs_waiter_validate_qc_for_incorrect_committee_fails() {
    todo!()
}

// async fn recv_timeout<'a, T>(channel: &'a mut Receiver<T>, duration: Duration<>) -> Result<Option<T>, String> {
//     let timeout = tokio::time::timeout(duration);
//     tokio::select! {
//         msg = channel.recv() => {
//             Ok(msg)
//         },
//         _ = timeout => {
//             Err("Timed out".to_string())
//         }
//     }
// }

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Test may be implemented in future"]
async fn test_hs_waiter_cannot_spend_until_it_is_proven_committed() {
    // You must provide a valid 4 chain proof in order to spend or exist an output
    todo!()
}

use tari_template_lib::{args::Arg, Hash};

use crate::{
    services::PayloadProcessor,
    storage::shard_store::MemoryShardStoreFactory,
    workers::hotstuff_error::HotStuffError,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_kitchen_sink() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let shard0_committee = vec![node1.clone()];
    let shard1_committee = vec![node2.clone()];

    let package = PackageBuilder::new()
        .add_wasm_module(
            compile_str(
                r#"
    use tari_template_macros::template;

#[template]
mod hello_world {
    pub struct HelloWorld {
    }

    impl HelloWorld {

        pub fn new() -> Self {
            Self {}
        }

        pub fn greet() -> String {
            "Hello World!".to_string()
        }

    }
}

    "#,
                &[],
            )
            .unwrap(),
        )
        .build()
        .unwrap();

    let instruction = Instruction::CallFunction {
        package_address: package.address(),
        template: "HelloWorld".to_string(),
        function: "new".to_string(),
        args: vec![Arg::Literal(b"Kitchen Sink".to_vec())],
    };
    let secret_key = PrivateKey::from_bytes(&[1; 32]).unwrap();

    let mut builder = TransactionBuilder::new();
    builder.add_instruction(instruction);
    // Only creating a single component
    // This tells us which shards are involved in the transaction
    // Because there are no inputs, we need to say that there are 2 components
    // being created, so that two shards are involved, not just one.
    builder.with_new_components(2);
    let transaction = builder.sign(&secret_key).build();

    let involved_shards = transaction.meta().involved_shards();
    let s1;
    let s2;
    // Sort the shards so that we can create a range epoch manager
    if involved_shards[0].0 < involved_shards[1].0 {
        s1 = involved_shards[0];
        s2 = involved_shards[1];
    } else {
        s1 = involved_shards[1];
        s2 = involved_shards[0];
    }
    let epoch_manager = RangeEpochManager::new_with_multiple(&[
        (s1..s2, shard0_committee),
        (s2..ShardId([255u8; 32]), shard1_committee),
    ]);
    // Create 2x hotstuff waiters
    let node1_instance = HsTestHarness::new(node1.clone(), epoch_manager.clone(), AlwaysFirstLeader {});
    let node2_instance = HsTestHarness::new(node2.clone(), epoch_manager, AlwaysFirstLeader {});

    let payload = TariDanPayload::new(transaction);

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), s1, Some(payload.clone()));
    node1_instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), s2, Some(payload.clone()));
    node2_instance
        .tx_hs_messages
        .send((node2.clone(), new_view_message.clone()))
        .await
        .unwrap();

    let mut nodes = vec![node1_instance, node2_instance];
    do_rounds_of_hotstuff(&mut nodes, 4).await;

    // [n0(prep)]->[n1(pre-commit)]->[n2(commit)]->[n3(decide)] -> [n4] tell everyone that we have decided

    // should get an execute message
    for node in &mut nodes {
        let (ex_transaction, shard_pledges) = node.recv_execute().await;

        dbg!(&shard_pledges);
        let mut pre_state = vec![];
        for (k, v) in shard_pledges {
            pre_state.push((k.0.to_vec(), v[0].current_state.clone()));
        }
        let state_db = MemoryStateStore::load(pre_state);
        // state_db.allow_creation_of_non_existent_shards = false;
        let state_tracker = StateTracker::new(
            state_db,
            Hash::try_from(ex_transaction.transaction().hash().as_slice()).unwrap(),
        );
        let runtime_interface = RuntimeInterfaceImpl::new(state_tracker);
        // Process the instruction
        let processor = InstructionProcessor::new(runtime_interface, package.clone());
        let result = processor.execute(ex_transaction.transaction().clone()).unwrap();

        // TODO: Save the changes substates back to shard db

        // reply_tx.s   end(HashMap::new()).unwrap();

        dbg!(&result);
        result.result.expect("Did not execute successfully");
    }

    for n in &mut nodes {
        n.assert_shuts_down_safely().await;
    }
    // let executor = ConsensusExecutor::new();
    //
    // let execute_msg = node1_instance.recv_execute().await;
}

async fn do_rounds_of_hotstuff<TPayload: Payload + 'static, TAddr: NodeAddressable + 'static>(
    nodes: &mut [HsTestHarness<TPayload, TAddr>],
    rounds: usize,
) {
    let mut node_map = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        node_map.insert(n.identity(), i);
    }
    for i in 0..rounds {
        dbg!(i);
        let mut proposals = vec![];
        for node in nodes.iter_mut() {
            let (proposal1_n1, _broadcast_group) = node.recv_broadcast().await;
            proposals.push((node.identity(), proposal1_n1));
        }

        for other_node in nodes.iter() {
            for (addr, msg) in &proposals {
                other_node
                    .tx_hs_messages
                    .send((addr.clone(), msg.clone()))
                    .await
                    .unwrap();
            }
        }

        let mut votes = HashMap::new();
        for node in nodes.iter_mut() {
            let (vote1, leader) = node.recv_vote_message().await;
            votes.entry(leader).or_insert(Vec::new()).push((vote1, node.identity()));
        }
        for leader in votes.keys() {
            for vote in votes.get(leader).unwrap() {
                let node_index = node_map.get(leader).unwrap();
                nodes
                    .get_mut(*node_index)
                    .unwrap()
                    .tx_votes
                    .send((vote.1.clone(), vote.0.clone()))
                    .await
                    .unwrap();
            }
        }
    }
}
