use std::{
    collections::HashMap,
    ops::{Index, Range, RangeBounds},
    time::Duration,
};

use async_recursion::async_recursion;
use clap::command;
use digest::{Digest, FixedOutput};
use lazy_static::lazy_static;
use tari_common_types::types::{FixedHash, PrivateKey};
use tari_crypto::{hash::blake2::Blake256, keys::SecretKey};
use tari_dan_common_types::ShardId;
use tari_dan_engine::{
    instruction::{Instruction, Transaction, TransactionBuilder},
    packager::PackageBuilder,
    wasm::{compile::compile_str, WasmModule},
};
use tari_shutdown::{Shutdown, ShutdownSignal};
use tari_utilities::ByteArray;
use tokio::{
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
    time::timeout,
};

use crate::{
    models::{
        vote_message::VoteMessage,
        Committee,
        Epoch,
        HotStuffMessage,
        HotStuffMessageType,
        HotStuffMessageType::Commit,
        HotStuffTreeNode,
        NodeHeight,
        ObjectPledge,
        Payload,
        QuorumCertificate,
        QuorumDecision,
        SubstateState,
        TariDanPayload,
        TreeNodeHash,
        ValidatorSignature,
        ViewId,
    },
    services::{
        epoch_manager::{EpochManager, RangeEpochManager},
        infrastructure_services::NodeAddressable,
        leader_strategy::{AlwaysFirstLeader, LeaderStrategy},
    },
    workers::hotstuff_waiter::HotStuffWaiter,
};

pub trait Consensus<TPayload: Payload> {
    fn execute_transaction(
        &mut self,
        payload: TPayload,
        inputs: Vec<ObjectPledge>,
        outputs: Vec<ObjectPledge>,
    ) -> Result<(), String>;
}

pub struct HsTestHarness<TPayload: Payload + 'static, TAddr: NodeAddressable + 'static> {
    identity: TAddr,
    tx_new: Sender<(TPayload, ShardId)>,
    tx_hs_messages: Sender<(TAddr, HotStuffMessage<TPayload, TAddr>)>,
    rx_leader: Receiver<HotStuffMessage<TPayload, TAddr>>,
    shutdown: Shutdown,
    rx_broadcast: Receiver<(HotStuffMessage<TPayload, TAddr>, Vec<TAddr>)>,
    rx_vote_message: Receiver<(VoteMessage, TAddr)>,
    tx_votes: Sender<(TAddr, VoteMessage)>,
    rx_execute: Receiver<TPayload>,
    hs_waiter: Option<JoinHandle<Result<(), String>>>,
}
impl<TPayload: Payload, TAddr: NodeAddressable> HsTestHarness<TPayload, TAddr> {
    pub fn new<
        TEpochManager: EpochManager<TAddr> + Send + Sync + 'static,
        TLeader: LeaderStrategy<TAddr, TPayload> + Send + Sync + 'static,
    >(
        identity: TAddr,
        epoch_manager: TEpochManager,
        leader: TLeader,
    ) -> Self {
        let (tx_new, rx_new) = channel(1);
        let (tx_hs_messages, rx_hs_messages) = channel(1);
        let (tx_leader, mut rx_leader) = channel(1);
        let (tx_broadcast, rx_broadcast) = channel(1);
        let (tx_vote_message, rx_vote_message) = channel(1);
        let (tx_votes, rx_votes) = channel(1);
        let (tx_execute, rx_execute) = channel(1);
        let shutdown = Shutdown::new();

        let hs_waiter = Some(HotStuffWaiter::<_, _, _, _>::spawn(
            identity.clone(),
            epoch_manager,
            leader,
            rx_new,
            rx_hs_messages,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_vote_message,
            tx_execute,
            shutdown.to_signal(),
        ));
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
            hs_waiter,
        }
    }

    fn identity(&self) -> TAddr {
        self.identity.clone()
    }

    async fn assert_shuts_down_safely(&mut self) {
        // send might fail if it's already shutdown
        let _ = self.shutdown.trigger();
        self.hs_waiter.take().unwrap().await.expect("did not end cleanly");
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

    async fn recv_execute(&mut self) -> TPayload {
        if let Some(msg) = timeout(Duration::from_secs(10), self.rx_execute.recv())
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
    static ref shard0: ShardId = ShardId(FixedHash::zero());
    static ref shard1: ShardId = ShardId(FixedHash::from([1u8; 32]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_receives_new_payload_starts_new_chain() {
    let node1 = "node1".to_string();

    let epoch_manager = RangeEpochManager::new(*shard0..*shard1, vec![node1.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});

    let new_payload = ("Hello world".to_string(), vec![*shard0]);
    instance.tx_new.send((new_payload, *shard0)).await.unwrap();
    let leader_message = instance.rx_leader.recv().await.expect("Did not receive leader message");
    dbg!(leader_message);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_proposes() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let epoch_manager = RangeEpochManager::new(*shard0..*shard1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![*shard0]);

    dbg!(payload.to_id());
    // Send a new view message
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *shard0, Some(payload));

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
    let epoch_manager = RangeEpochManager::new(*shard0..*shard1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![*shard0]);
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *shard0, Some(payload));

    // Node 2 sends new view to node 1
    instance
        .tx_hs_messages
        .send((node2, new_view_message.clone()))
        .await
        .unwrap();

    // Should receive a proposal
    let (proposal_message, broadcast_group) = instance.recv_broadcast().await;

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
    let epoch_manager = RangeEpochManager::new(*shard0..*shard1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![*shard0]);

    // Start a new view
    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *shard0, Some(payload));
    instance
        .tx_hs_messages
        .send((node2.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal_message, broadcast_group) = instance.recv_broadcast().await;

    // tx_hs_messages
    //     .send((node1.clone(), proposal_message))
    //     .await
    //     .expect("Should not error");

    let vote_hash = proposal_message.node().unwrap().hash().clone();

    // Create some votes
    let mut vote = VoteMessage::new(vote_hash.clone(), *shard0, QuorumDecision::Accept, Default::default());

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
    let mut vote = VoteMessage::new(vote_hash.clone(), *shard0, QuorumDecision::Accept, Default::default());
    vote.sign();
    instance.tx_votes.send((node2, vote)).await.unwrap();

    // should get a proposal

    let (proposal2, broadcast_group) = instance.recv_broadcast().await;

    let proposed_node = proposal2.node().expect("Should have a node attached");

    assert_eq!(proposed_node.justify().local_node_hash(), vote_hash);

    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_execute_called_when_consensus_reached() {
    let node1 = "node1".to_string();
    let epoch_manager = RangeEpochManager::new(*shard0..*shard1, vec![node1.clone()]);
    let mut instance = HsTestHarness::new(node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = ("Hello World".to_string(), vec![*shard0]);

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *shard0, Some(payload.clone()));
    instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal1, broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal1)).await.unwrap();
    let (vote, _) = instance.recv_vote_message().await;
    // loopback the vote
    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();
    let (proposal2, broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal2)).await.unwrap();
    let (vote, _) = instance.recv_vote_message().await;

    // No execute yet
    instance.assert_no_execute().await;

    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal3, broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal3)).await.unwrap();
    let (vote, _) = instance.recv_vote_message().await;

    // No execute yet
    instance.assert_no_execute().await;
    // loopback the vote
    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal4, broadcast_group) = instance.recv_broadcast().await;

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

    assert_eq!(executed_payload, payload);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_multishard_votes() {
    let node1 = "node1".to_string();
    let node2 = "node2".to_string();
    let shard0_committee = vec![node1.clone()];
    let shard1_committee = vec![node2.clone()];
    let epoch_manager = RangeEpochManager::new_with_multiple(&vec![
        (*shard0..*shard1, shard0_committee),
        (*shard1..ShardId(FixedHash::from([2u8; 32])), shard1_committee),
    ]);
    let mut node1_instance = HsTestHarness::new(node1.clone(), epoch_manager.clone(), AlwaysFirstLeader {});
    let mut node2_instance = HsTestHarness::new(node2.clone(), epoch_manager, AlwaysFirstLeader {});

    let payload = ("Hello World".to_string(), vec![*shard0, *shard1]);

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *shard0, Some(payload.clone()));
    node1_instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(), *shard1, Some(payload.clone()));
    node2_instance
        .tx_hs_messages
        .send((node2.clone(), new_view_message.clone()))
        .await
        .unwrap();

    let (proposal1_n1, broadcast_group) = node1_instance.recv_broadcast().await;
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
    let (proposal1_n2, broadcast_group) = node2_instance.recv_broadcast().await;
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
    let (proposal2_n1, broadcast_group) = node1_instance.recv_broadcast().await;
    let (proposal2_n2, broadcast_group) = node2_instance.recv_broadcast().await;

    node1_instance.assert_shuts_down_safely().await;
    node2_instance.assert_shuts_down_safely().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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
async fn test_hs_waiter_non_committee_member_does_not_start_new_view() {
    todo!()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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
async fn test_hs_waiter_cannot_spend_until_it_is_proven_committed() {
    // You must provide a valid 4 chain proof in order to spend or exist an output
    todo!()
}

use tari_template_lib::args::Arg;

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
    use tari_template_lib::prelude::*;
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
    builder.add_outputs(2);
    let transaction = builder.sign(&secret_key).build();

    let involved_shards = transaction.meta().involved_shards();
    dbg!(&involved_shards);
    let s1;
    let s2;
    if involved_shards[0].0 < involved_shards[1].0 {
        s1 = involved_shards[0];
        s2 = involved_shards[1];
    } else {
        s1 = involved_shards[1];
        s2 = involved_shards[0];
    }
    let epoch_manager = RangeEpochManager::new_with_multiple(&vec![
        (s1..s2, shard0_committee),
        (s2..ShardId(FixedHash::from([255u8; 32])), shard1_committee),
    ]);
    let mut node1_instance = HsTestHarness::new(node1.clone(), epoch_manager.clone(), AlwaysFirstLeader {});
    let mut node2_instance = HsTestHarness::new(node2.clone(), epoch_manager, AlwaysFirstLeader {});

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

    // should get an execute message
    for node in nodes.iter_mut() {
        let execute_message = node.recv_execute().await;
        dbg!(&node.identity, execute_message);
    }

    for n in nodes.iter_mut() {
        n.assert_shuts_down_safely().await;
    }
    // let executor = ConsensusExecutor::new();
    //
    // let execute_msg = node1_instance.recv_execute().await;
}

async fn do_rounds_of_hotstuff<TPayload: Payload, TAddr: NodeAddressable>(
    nodes: &mut Vec<HsTestHarness<TPayload, TAddr>>,
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
            let (proposal1_n1, broadcast_group) = node.recv_broadcast().await;
            proposals.push((node.identity(), proposal1_n1));
        }

        for other_node in nodes.iter() {
            for proposal in proposals.iter() {
                other_node
                    .tx_hs_messages
                    .send((proposal.0.clone(), proposal.1.clone()))
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
