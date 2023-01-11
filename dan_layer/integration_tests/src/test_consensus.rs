//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{collections::HashMap, sync::Arc, time::Duration};

use lazy_static::lazy_static;
use rand::rngs::OsRng;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_comms::{multiaddr::Multiaddr, peer_manager::PeerFeatures, NodeIdentity};
use tari_core::ValidatorNodeMmr;
use tari_crypto::keys::PublicKey as PublicKeyT;
use tari_dan_common_types::{vn_mmr_node_hash, Epoch, QuorumCertificate, QuorumDecision, ShardId};
use tari_dan_core::{
    models::{vote_message::VoteMessage, HotStuffMessage, Payload, TariDanPayload},
    services::{epoch_manager::RangeEpochManager, leader_strategy::AlwaysFirstLeader, NodeIdentitySigningService},
    storage::shard_store::{ShardStore, ShardStoreTransaction},
};
use tari_dan_engine::transaction::{Transaction, TransactionBuilder};
use tari_engine_types::instruction::Instruction;
use tari_template_lib::{args, models::TemplateAddress};
use tari_utilities::ByteArray;
use tokio::time::timeout;

use crate::harness::HsTestHarness;

fn create_test_default_qc(
    committee_keys: Vec<(PublicKey, PrivateKey)>,
    all_vn_keys: Vec<PublicKey>,
    payload: &TariDanPayload,
) -> QuorumCertificate {
    create_test_qc(ShardId::zero(), committee_keys, all_vn_keys, payload)
}

fn create_test_qc(
    shard_id: ShardId,
    committee_keys: Vec<(PublicKey, PrivateKey)>,
    all_vn_keys: Vec<PublicKey>,
    payload: &TariDanPayload,
) -> QuorumCertificate {
    let qc = QuorumCertificate::genesis(Epoch(0), payload.to_id(), shard_id);
    let vote = VoteMessage::new(qc.node_hash(), *qc.decision(), qc.all_shard_pledges().to_vec());

    let mut vn_mmr = ValidatorNodeMmr::new(Vec::new());
    for pk in &all_vn_keys {
        vn_mmr
            .push(vn_mmr_node_hash(pk, &ShardId::zero()).to_vec())
            .expect("Could not build the merkle mountain range of the VN set");
    }

    let validators_metadata: Vec<_> = committee_keys
        .into_iter()
        .map(|(_, secret)| {
            let mut node_vote = vote.clone();
            let node_identity = Arc::new(NodeIdentity::new(
                secret,
                Multiaddr::empty(),
                PeerFeatures::COMMUNICATION_NODE,
            ));
            node_vote
                .sign_vote(
                    &NodeIdentitySigningService::new(node_identity),
                    ShardId::zero(),
                    &vn_mmr,
                )
                .unwrap();
            node_vote.validator_metadata().clone()
        })
        .collect();

    QuorumCertificate::new(
        qc.payload_id(),
        qc.payload_height(),
        qc.node_hash(),
        qc.node_height(),
        shard_id,
        qc.epoch(),
        *qc.decision(),
        qc.all_shard_pledges().to_vec(),
        validators_metadata,
    )
}

lazy_static! {
    static ref SHARD0: ShardId = ShardId::zero();
    static ref SHARD1: ShardId = ShardId([1u8; 32]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_receives_new_payload_starts_new_chain() {
    // let node1 = "node1".to_string()
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let registered_vn_keys = vec![node1.clone()];
    let epoch_manager = RangeEpochManager::new(registered_vn_keys, *SHARD0..*SHARD1, vec![node1.clone()]);
    let mut instance = HsTestHarness::new(node1_pk.clone(), node1.clone(), epoch_manager, AlwaysFirstLeader {});

    let new_payload = TariDanPayload::new(Transaction::builder().sign(&node1_pk).clone().build());
    instance.tx_new.send((new_payload, *SHARD0)).await.unwrap();
    let leader_message = instance.rx_leader.recv().await.expect("Did not receive leader message");
    dbg!(leader_message);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_leader_proposes() {
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let (node2_pk, node2) = PublicKey::random_keypair(&mut OsRng);
    let registered_vn_keys = vec![node1.clone(), node2.clone()];
    let epoch_manager =
        RangeEpochManager::new(registered_vn_keys, *SHARD0..*SHARD1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1_pk.clone(), node1.clone(), epoch_manager, AlwaysFirstLeader {});
    // let payload = ("Hello World".to_string(), vec![*SHARD0]);
    let payload = TariDanPayload::new(
        Transaction::builder()
            .add_input(*SHARD0)
            .sign(&node1_pk)
            .clone()
            .build(),
    );

    let qc = create_test_default_qc(
        vec![(node1.clone(), node1_pk), (node2.clone(), node2_pk)],
        vec![node1.clone(), node2.clone()],
        &payload,
    );
    let new_view_message = HotStuffMessage::new_view(qc, *SHARD0, payload);

    instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();
    instance
        .tx_hs_messages
        .send((node2.clone(), new_view_message))
        .await
        .unwrap();

    let (_, mut broadcast_group) = instance.recv_broadcast().await;

    broadcast_group.sort();
    let mut all_nodes = vec![node1, node2];
    all_nodes.sort();
    assert_eq!(broadcast_group, all_nodes);
    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_replica_sends_vote_for_proposal() {
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let (node2_pk, node2) = PublicKey::random_keypair(&mut OsRng);
    let registered_vn_keys = vec![node1.clone(), node2.clone()];
    let epoch_manager =
        RangeEpochManager::new(registered_vn_keys, *SHARD0..*SHARD1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1_pk.clone(), node1.clone(), epoch_manager, AlwaysFirstLeader {});
    // let payload = ("Hello World".to_string(), vec![*SHARD0]);
    let payload = TariDanPayload::new(
        Transaction::builder()
            .add_input(*SHARD0)
            .sign(&node1_pk)
            .clone()
            .build(),
    );
    let qc = create_test_default_qc(
        vec![(node1.clone(), node1_pk), (node2.clone(), node2_pk)],
        vec![node1.clone(), node2.clone()],
        &payload,
    );
    let new_view_message = HotStuffMessage::new_view(qc, *SHARD0, payload);

    // Node 2 sends new view to node 1
    instance
        .tx_hs_messages
        .send((node2, new_view_message.clone()))
        .await
        .unwrap();
    instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
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
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let (node2_pk, node2) = PublicKey::random_keypair(&mut OsRng);
    let registered_vn_keys = vec![node1.clone(), node2.clone()];

    // create the VN set mmr
    let mut vn_mmr = ValidatorNodeMmr::new(Vec::new());
    vn_mmr
        .push(vn_mmr_node_hash(&node1, &ShardId::zero()).to_vec())
        .unwrap();
    vn_mmr
        .push(vn_mmr_node_hash(&node2, &ShardId::zero()).to_vec())
        .unwrap();

    let epoch_manager =
        RangeEpochManager::new(registered_vn_keys, *SHARD0..*SHARD1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(node1_pk.clone(), node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = TariDanPayload::new(
        Transaction::builder()
            .add_input(*SHARD0)
            .add_output(*SHARD1)
            .sign(&node1_pk)
            .clone()
            .build(),
    );

    // Start a new view
    let qc = create_test_default_qc(
        vec![(node1.clone(), node1_pk.clone()), (node2.clone(), node2_pk.clone())],
        vec![node1.clone(), node2.clone()],
        &payload,
    );
    let new_view_message = HotStuffMessage::new_view(qc, *SHARD0, payload.clone());
    instance
        .tx_hs_messages
        .send((node2.clone(), new_view_message.clone()))
        .await
        .unwrap();
    instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    // Get the node hash from the proposal
    let (proposal_message, _broadcast_group) = instance.recv_broadcast().await;

    let vote_hash = proposal_message.node().unwrap().hash();
    instance
        .state_store()
        .with_write_tx(|tx| tx.save_node(proposal_message.node().unwrap().clone()))
        .unwrap();

    // Create some votes
    let mut vote = VoteMessage::new(
        *vote_hash,
        QuorumDecision::Accept,
        new_view_message.high_qc().unwrap().all_shard_pledges().to_vec(),
    );
    vote.sign_vote(instance.signing_service(), *SHARD0, &vn_mmr).unwrap();
    instance.tx_votes.send((node1, vote.clone())).await.unwrap();

    // Should get no proposal
    assert!(
        timeout(Duration::from_secs(1), instance.rx_broadcast.recv())
            .await
            .is_err(),
        "received a proposal when we weren't expecting it"
    );

    // Send another vote
    let mut vote = VoteMessage::new(*vote_hash, QuorumDecision::Accept, Default::default());
    vote.sign_vote(instance.signing_service(), ShardId::zero(), &vn_mmr)
        .unwrap();
    instance.tx_votes.send((node2, vote)).await.unwrap();

    // should get a proposal
    let (proposal2, _broadcast_group) = instance.recv_broadcast().await;
    let proposed_node = proposal2.node().expect("Should have a node attached");
    assert_eq!(proposed_node.justify().node_hash(), *vote_hash);

    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_execute_called_at_prepare_phase_only() {
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let registered_vn_keys = vec![node1.clone()];
    let epoch_manager = RangeEpochManager::new(registered_vn_keys, *SHARD0..*SHARD1, vec![node1.clone()]);
    let mut instance = HsTestHarness::new(node1_pk.clone(), node1.clone(), epoch_manager, AlwaysFirstLeader {});
    let payload = TariDanPayload::new(
        Transaction::builder()
            .add_input(*SHARD0)
            .sign(&node1_pk)
            .clone()
            .build(),
    );

    let qc = create_test_default_qc(vec![(node1.clone(), node1_pk.clone())], vec![node1.clone()], &payload);
    let new_view_message = HotStuffMessage::new_view(qc, *SHARD0, payload.clone());
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

    // Execute at h=0
    let (executed_payload, _) = instance.recv_execute().await;
    assert_eq!(executed_payload.transaction(), payload.transaction());

    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal3, _broadcast_group) = instance.recv_broadcast().await;

    // loopback the proposal
    instance.tx_hs_messages.send((node1.clone(), proposal3)).await.unwrap();
    let (vote, _) = instance.recv_vote_message().await;

    instance.assert_no_execute().await;

    // loopback the vote
    instance.tx_votes.send((node1.clone(), vote.clone())).await.unwrap();

    let (proposal4, _broadcast_group) = instance.recv_broadcast().await;

    dbg!(&proposal4);
    instance.tx_hs_messages.send((node1.clone(), proposal4)).await.unwrap();

    // Does not execute again
    instance.assert_no_execute().await;

    instance.assert_shuts_down_safely().await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_waiter_multishard_votes() {
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let (node2_pk, node2) = PublicKey::random_keypair(&mut OsRng);
    let shard0_committee = vec![node1.clone()];
    let shard1_committee = vec![node2.clone()];
    let registered_vn_keys = vec![node1.clone(), node2.clone()];
    let epoch_manager = RangeEpochManager::new_with_multiple(registered_vn_keys, &[
        (*SHARD0..*SHARD1, shard0_committee),
        (*SHARD1..ShardId([2u8; 32]), shard1_committee),
    ]);
    let mut node1_instance = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager.clone(),
        AlwaysFirstLeader {},
    );
    let mut node2_instance = HsTestHarness::new(node2_pk.clone(), node2.clone(), epoch_manager, AlwaysFirstLeader {});

    let payload = TariDanPayload::new(
        Transaction::builder()
            .with_inputs(vec![*SHARD0, *SHARD1])
            .sign(&node1_pk)
            .clone()
            .build(),
    );

    let qc_shard0 = create_test_qc(
        *SHARD0,
        vec![(node1.clone(), node1_pk.clone())],
        vec![node1.clone(), node2.clone()],
        &payload,
    );
    let new_view_message = HotStuffMessage::new_view(qc_shard0, *SHARD0, payload.clone());
    node1_instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    let qc_shard1 = create_test_qc(
        *SHARD1,
        vec![(node2.clone(), node2_pk.clone())],
        vec![node1.clone(), node2.clone()],
        &payload,
    );
    let new_view_message = HotStuffMessage::new_view(qc_shard1, *SHARD1, payload.clone());
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
    // let new_view_message = HotStuffMessage::new_view(QuorumCertificate::genesis(0), 0, payload);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Implement test"]
async fn test_kitchen_sink() {
    // TODO: Implement a test harness that can connect many nodes together
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let (node2_pk, node2) = PublicKey::random_keypair(&mut OsRng);
    let shard0_committee = vec![node1.clone()];
    let shard1_committee = vec![node2.clone()];

    let template_address =
        TemplateAddress::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap();
    // let package = PackageBuilder::new()
    //     .add_template(
    //         template_address,
    //         compile_str(
    //             r#"
    //         use tari_template_lib::prelude::*;
    //
    //     #[template]
    //     mod hello_world {
    //         pub struct HelloWorld { }
    //
    //         impl HelloWorld {
    //             pub fn new() -> Self {
    //                 Self {}
    //             }
    //
    //             pub fn greet() -> String {
    //                 "Hello World!".to_string()
    //             }
    //         }
    //     }
    //         "#,
    //             &[],
    //         )
    //         .unwrap()
    //         .load_template()
    //         .unwrap(),
    //     )
    //     .build();

    let instruction = Instruction::CallFunction {
        template_address,
        function: "new".to_string(),
        args: args![b"Kitchen Sink"],
    };
    let secret_key = PrivateKey::from_bytes(&[1; 32]).unwrap();

    let mut builder = TransactionBuilder::new();
    builder.add_instruction(instruction);
    // Only creating a single component
    // This tells us which shards are involved in the transaction
    // Because there are no inputs, we need to say that there are 2 components
    // being created, so that two shards are involved, not just one.
    builder.with_new_outputs(2).sign(&secret_key);
    let transaction = builder.build();

    let mut involved_shards = transaction.meta().involved_shards();
    // Sort the shards so that we can create a range epoch manager
    involved_shards.sort();
    let s1 = involved_shards[0];
    let s2 = involved_shards[1];

    let registered_vn_keys = vec![node1.clone(), node2.clone()];
    let epoch_manager = RangeEpochManager::new_with_multiple(registered_vn_keys, &[
        (s1..s2, shard0_committee),
        (s2..ShardId([255u8; 32]), shard1_committee),
    ]);
    // Create 2x hotstuff waiters
    let node1_instance = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager.clone(),
        AlwaysFirstLeader {},
    );
    // node1_instance.add_package(package);
    let node2_instance = HsTestHarness::new(node2_pk.clone(), node2.clone(), epoch_manager, AlwaysFirstLeader {});
    // TODO: Create a task that connects the two instances via their channels so that they can do the hotstuff rounds
    // node1_instance.connect(&node2_instance);

    let payload = TariDanPayload::new(transaction);

    let qc_s1 = create_test_qc(
        s1,
        vec![(node1.clone(), node1_pk.clone())],
        vec![node1.clone(), node2.clone()],
        &payload,
    );
    let new_view_message = HotStuffMessage::new_view(qc_s1, s1, payload.clone());
    node1_instance
        .tx_hs_messages
        .send((node1.clone(), new_view_message.clone()))
        .await
        .unwrap();

    let qc_s2 = create_test_qc(
        s2,
        vec![(node2.clone(), node2_pk.clone())],
        vec![node1.clone(), node2.clone()],
        &payload,
    );
    let new_view_message = HotStuffMessage::new_view(qc_s2, s2, payload.clone());
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
        let (_ex_transaction, shard_pledges) = node.recv_execute().await;

        dbg!(&shard_pledges);
        // TODO: Not sure why this is failing
        // let mut pre_state = vec![];
        // for (k, v) in shard_pledges {
        //     pre_state.push((k.0.to_vec(), v[0].current_state.clone()));
        // }
        // let state_db = MemoryStateStore::load(pre_state);
        // // state_db.allow_creation_of_non_existent_shards = false;
        // let state_tracker = StateTracker::new(state_db, *ex_transaction.transaction().hash());
        // let runtime_interface = RuntimeInterfaceImpl::new(state_tracker);
        // // Process the instruction
        // let processor = TransactionProcessor::new(runtime_interface, package.clone());
        // let result = processor.execute(ex_transaction.transaction().clone()).unwrap();

        // TODO: Save the changes substates back to shard db

        // reply_tx.s   end(HashMap::new()).unwrap();

        // dbg!(&result);
        // result.result.expect("Did not execute successfully");
    }

    for n in &mut nodes {
        n.assert_shuts_down_safely().await;
    }
    // let executor = ConsensusExecutor::new();
    //
    // let execute_msg = node1_instance.recv_execute().await;
}

async fn do_rounds_of_hotstuff(validator_instances: &mut [HsTestHarness], rounds: usize) {
    #[allow(clippy::mutable_key_type)]
    let mut node_map = HashMap::new();
    for (i, n) in validator_instances.iter().enumerate() {
        node_map.insert(n.identity(), i);
    }
    for i in 0..rounds {
        dbg!(i);
        let mut proposals = vec![];
        for node in validator_instances.iter_mut() {
            let (proposal, _broadcast_group) = node.recv_broadcast().await;
            proposals.push((node.identity(), proposal));
        }

        for other_node in validator_instances.iter() {
            for (addr, msg) in &proposals {
                other_node
                    .tx_hs_messages
                    .send((addr.clone(), msg.clone()))
                    .await
                    .unwrap();
            }
        }

        #[allow(clippy::mutable_key_type)]
        let mut votes = HashMap::<_, Vec<_>>::new();
        for node in validator_instances.iter_mut() {
            let (vote1, leader) = node.recv_vote_message().await;
            votes.entry(leader).or_default().push((vote1, node.identity()));
        }
        for leader in votes.keys() {
            for vote in votes.get(leader).unwrap() {
                let node_index = node_map.get(leader).unwrap();
                validator_instances
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
