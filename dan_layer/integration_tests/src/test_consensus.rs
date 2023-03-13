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

use std::{collections::HashMap, ops::Range, sync::Arc, time::Duration};

use lazy_static::lazy_static;
use rand::rngs::OsRng;
use tari_common_types::types::{PrivateKey, PublicKey};
use tari_comms::{
    multiaddr::Multiaddr,
    peer_manager::PeerFeatures,
    protocol::rpc::__macro_reexports::future::join_all,
    NodeIdentity,
};
use tari_core::ValidatorNodeBMT;
use tari_crypto::{
    keys::PublicKey as PublicKeyT,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_dan_common_types::{vn_bmt_node_hash, Epoch, QuorumCertificate, QuorumDecision, ShardId};
use tari_dan_core::{
    models::{vote_message::VoteMessage, HotStuffMessage, HotstuffPhase, Payload, TariDanPayload},
    services::{
        epoch_manager::RangeEpochManager,
        leader_strategy::{AlwaysFirstLeader, RotatingLeader},
        NodeIdentitySigningService,
    },
    storage::shard_store::{ShardStore, ShardStoreWriteTransaction},
    workers::hotstuff_waiter::RecoveryMessage,
};
use tari_engine_types::instruction::Instruction;
use tari_template_lib::{args, models::TemplateAddress};
use tari_transaction::{Transaction, TransactionBuilder};
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
    let vote = VoteMessage::new(qc.node_hash(), *qc.decision(), qc.all_shard_pledges().clone());

    let mut vn_bmt_vec = Vec::new();
    for pk in &all_vn_keys {
        vn_bmt_vec.push(vn_bmt_node_hash(pk, &ShardId::zero()).to_vec())
    }
    let vn_bmt = ValidatorNodeBMT::create(vn_bmt_vec);

    let validators_metadata: Vec<_> = committee_keys
        .into_iter()
        .map(|(_, secret)| {
            let mut node_vote = vote.clone();
            let node_identity = Arc::new(NodeIdentity::new(
                secret,
                vec![Multiaddr::empty()],
                PeerFeatures::COMMUNICATION_NODE,
            ));
            node_vote
                .sign_vote(
                    &NodeIdentitySigningService::new(node_identity),
                    ShardId::zero(),
                    &vn_bmt,
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
        qc.all_shard_pledges().clone(),
        validators_metadata,
    )
}

lazy_static! {
    static ref SHARD0: ShardId = ShardId::zero();
    static ref SHARD1: ShardId = ShardId([1u8; 32]);
    static ref SHARD2: ShardId = ShardId([2u8; 32]);
    static ref SHARD3: ShardId = ShardId([3u8; 32]);
    static ref NEVER: Duration = Duration::from_secs(86400); // 1 day, can't use MAX, because there is multiplicator in the hotstuff_waitter
    static ref ONE_SEC: Duration = Duration::from_secs(1);
    static ref TEN_SECONDS: Duration = Duration::from_secs(10);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_receives_new_payload_starts_new_chain() {
    // let node1 = "node1".to_string()
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let registered_vn_keys = vec![node1.clone()];
    let epoch_manager = RangeEpochManager::new(registered_vn_keys, *SHARD0..*SHARD1, vec![node1.clone()]);
    let mut instance = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager,
        AlwaysFirstLeader {},
        *NEVER,
    );

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
    let mut instance = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager,
        AlwaysFirstLeader {},
        *NEVER,
    );
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
    let mut instance = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager,
        AlwaysFirstLeader {},
        *NEVER,
    );
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
    let vn_bmt_vec = vec![
        vn_bmt_node_hash(&node1, &ShardId::zero()).to_vec(),
        vn_bmt_node_hash(&node2, &ShardId::zero()).to_vec(),
    ];

    let vn_bmt = ValidatorNodeBMT::create(vn_bmt_vec);

    let epoch_manager =
        RangeEpochManager::new(registered_vn_keys, *SHARD0..*SHARD1, vec![node1.clone(), node2.clone()]);
    let mut instance = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager,
        AlwaysFirstLeader {},
        *NEVER,
    );
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
        new_view_message.high_qc().unwrap().all_shard_pledges().clone(),
    );
    vote.sign_vote(instance.signing_service(), *SHARD0, &vn_bmt).unwrap();
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
    vote.sign_vote(instance.signing_service(), ShardId::zero(), &vn_bmt)
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
    let mut instance = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager,
        AlwaysFirstLeader {},
        *NEVER,
    );
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
        (*SHARD1..*SHARD2, shard1_committee),
    ]);
    let mut node1_instance = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager.clone(),
        AlwaysFirstLeader {},
        *NEVER,
    );
    let mut node2_instance = HsTestHarness::new(
        node2_pk.clone(),
        node2.clone(),
        epoch_manager,
        AlwaysFirstLeader {},
        *NEVER,
    );

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

// This test triggers foreign recovery on every possible step.
#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_leader_fails_only_foreignly() {
    let (node0_pk, node0) = PublicKey::random_keypair(&mut OsRng);
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let shard0_committee = vec![node0.clone()];
    let shard1_committee = vec![node1.clone()];
    let registered_vn_keys = vec![node0.clone(), node1.clone()];
    let epoch_manager = RangeEpochManager::new_with_multiple(registered_vn_keys, &[
        (*SHARD0..*SHARD1, shard0_committee),
        (*SHARD1..*SHARD2, shard1_committee),
    ]);

    let mut instance0 = HsTestHarness::new(
        node0_pk.clone(),
        node0.clone(),
        epoch_manager.clone(),
        RotatingLeader {},
        *ONE_SEC,
    );
    let mut instance1 = HsTestHarness::new(
        node1_pk.clone(),
        node1.clone(),
        epoch_manager.clone(),
        RotatingLeader {},
        *NEVER,
    );

    let payload = TariDanPayload::new(
        Transaction::builder()
            .add_input(*SHARD0)
            .add_input(*SHARD1)
            .sign(&node0_pk)
            .clone()
            .build(),
    );

    // Start TX
    instance0.tx_new.send((payload.clone(), *SHARD0)).await.unwrap();
    instance1.tx_new.send((payload.clone(), *SHARD1)).await.unwrap();
    // Get new views
    let new_view0 = timeout(*ONE_SEC, instance0.rx_leader.recv()).await;
    let new_view1 = timeout(*ONE_SEC, instance1.rx_leader.recv()).await;
    assert!(new_view0.is_ok(), "New_view0 was send");
    assert!(new_view1.is_ok(), "New_view1 was send");
    let new_view0 = new_view0.unwrap().unwrap().1;
    let new_view1 = new_view1.unwrap().unwrap().1;
    // Leaders are selected
    instance0.tx_hs_messages.send((node0.clone(), new_view0)).await.unwrap();
    instance1.tx_hs_messages.send((node1.clone(), new_view1)).await.unwrap();
    for phase in [
        HotstuffPhase::Prepare,
        HotstuffPhase::PreCommit,
        HotstuffPhase::Commit,
        HotstuffPhase::Decide,
    ] {
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Proposals should be send
        let proposal0 = timeout(*ONE_SEC, instance0.rx_broadcast.recv()).await;
        let proposal1 = timeout(*ONE_SEC, instance1.rx_broadcast.recv()).await;
        assert!(proposal0.is_ok(), "Proposal0 should be send");
        assert!(proposal1.is_ok(), "Proposal1 should be send");
        // Now don't send the proposal1 to commmittee0
        let proposal0 = proposal0.unwrap().unwrap().0;
        let proposal1 = proposal1.unwrap().unwrap().0;
        assert_eq!(proposal0.node().unwrap().payload_phase(), phase);
        assert_eq!(proposal1.node().unwrap().payload_phase(), phase);
        // Node1 will never send proposal directly, only from recovery.
        instance0
            .tx_hs_messages
            .send((node0.clone(), proposal0.clone()))
            .await
            .unwrap();
        instance1.tx_hs_messages.send((node0.clone(), proposal0)).await.unwrap();
        instance1.tx_hs_messages.send((node1.clone(), proposal1)).await.unwrap();
        let recovery_message = timeout(*ONE_SEC * 10, instance0.rx_recovery_broadcast.recv()).await;
        assert!(
            recovery_message.is_ok(),
            "Recovery message should be send from committee0"
        );
        let recovery_message = recovery_message.unwrap().unwrap().0;
        assert!(matches!(recovery_message, RecoveryMessage::MissingProposal(..)));
        // Now we send the request and get the missing proposal
        instance1
            .tx_recovery_messages
            .send((node0.clone(), recovery_message))
            .await
            .unwrap();
        // The node should send it on normal channel, not the recovery
        let recovery_proposal1 = timeout(*ONE_SEC * 10, instance1.rx_leader.recv()).await;
        assert!(
            recovery_proposal1.is_ok(),
            "The node should send proposal as a response to the recovery request"
        );
        let recovery_proposal1 = recovery_proposal1.unwrap().unwrap().1;
        assert_eq!(recovery_proposal1.node().unwrap().payload_phase(), phase);
        instance0
            .tx_hs_messages
            .send((node1.clone(), recovery_proposal1))
            .await
            .unwrap();
        if phase != HotstuffPhase::Decide {
            tokio::time::sleep(Duration::from_millis(100)).await;
            // Now the node0 has both proposal and if they are valid it should send a vote.
            let vote0 = timeout(*ONE_SEC, instance0.rx_vote_message.recv()).await;
            let vote1 = timeout(*ONE_SEC, instance1.rx_vote_message.recv()).await;
            assert!(vote0.is_ok(), "The node0 should send a vote once it has both proposals"); // This one is from the recovered proposal.
            assert!(vote1.is_ok(), "The node1 should send a vote");
            let vote0 = vote0.unwrap().unwrap().0;
            let vote1 = vote1.unwrap().unwrap().0;
            instance0.tx_votes.send((node0.clone(), vote0)).await.unwrap();
            instance1.tx_votes.send((node1.clone(), vote1)).await.unwrap();
        }
    }
    instance0.assert_shuts_down_safely().await;
    instance1.assert_shuts_down_safely().await;
}

#[allow(clippy::too_many_lines)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_leader_fails_only_locally() {
    // We create 2 committees. Committee0 will fail only locally. Committee1 will work fine.
    let mut test = Test::init(vec![(4, *ONE_SEC), (1, *NEVER)]);
    test.send_tx_new_to_all().await;
    let new_views = test.receive_all_rx_leader().await;
    test.leaders_receive_messages(new_views).await;
    let proposals = test.get_proposals().await;
    assert!(
        proposals
            .iter()
            .all(|proposal| proposal.0.node().unwrap().payload_phase() == HotstuffPhase::Prepare),
        "All proposals should be for 'Prepare' phase"
    );
    test.send_proposal_to_all_committees(proposals[1].clone()).await;
    // Leader in committee0 fails only locally.
    test.send_proposal_to_committee(1, proposals[0].clone()).await;
    test.remove_leader_from_committee(0).await;

    // Committee1 will act normally.
    let votes1 = test.committees[1].get_votes().await;
    test.committees[1].leader_receive_votes(votes1).await;
    let pre_commit_proposal1 = test.committees[1].get_proposal().await;
    assert_eq!(
        pre_commit_proposal1.0.node().unwrap().payload_phase(),
        HotstuffPhase::PreCommit
    );
    test.send_proposal_to_all_committees(pre_commit_proposal1).await;

    // Now the trigger of local leader failure will happen in committe0.
    // Committee 0 start election;
    let new_views0 = test.committees[0].receive_rx_leader().await;
    test.committees[0].leader_receive_messages(new_views0).await;

    let prepare_proposal0 = test.committees[0].get_proposal().await;
    assert_eq!(
        prepare_proposal0.0.node().unwrap().payload_phase(),
        HotstuffPhase::Prepare
    );
    test.send_proposal_to_all_committees(prepare_proposal0).await;

    // Committee1 received a valid proposal, so they vote again.
    let votes1 = test.committees[1].get_votes().await;
    // Just for the sake of real life simulation we send these to their leader, but they are ignored
    test.committees[1].leader_receive_votes(votes1).await;

    let votes0 = test.committees[0].get_votes().await;
    test.committees[0].leader_receive_votes(votes0).await;

    let pre_commit_proposal0 = test.committees[0].get_proposal().await;
    assert_eq!(
        pre_commit_proposal0.0.node().unwrap().payload_phase(),
        HotstuffPhase::PreCommit
    );
    test.send_proposal_to_all_committees(pre_commit_proposal0).await;

    // Now both should be on the same height. Let's finish this transaction.
    for phase in [HotstuffPhase::Commit, HotstuffPhase::Decide] {
        println!("Phase: {:?}", phase);
        let votes = test.get_all_votes().await;
        test.all_leaders_receive_votes(votes).await;
        let proposals = test.get_proposals().await;
        assert!(
            proposals
                .iter()
                .all(|proposal| proposal.0.node().unwrap().payload_phase() == phase),
            "All proposals should be for '{:?}' phase",
            phase
        );
        test.send_all_proposals_to_all_committees(proposals).await;
    }
}

pub struct Committee {
    keys: Vec<(RistrettoSecretKey, RistrettoPublicKey)>,
    shard_committee: Vec<RistrettoPublicKey>,
    instances: Vec<HsTestHarness>,
    previous_leader: Option<RistrettoPublicKey>,
    shard_range: Range<ShardId>,
    timeout: Duration,
}

impl Committee {
    pub fn init(size: usize, timeout: Duration, shard_range: Range<ShardId>) -> Self {
        let keys = (0..size)
            .map(|_| PublicKey::random_keypair(&mut OsRng))
            .collect::<Vec<_>>();
        let shard_committee = keys
            .iter()
            .map(|(_, public_key)| public_key.clone())
            .collect::<Vec<_>>();
        println!("Committe : {:?}", shard_committee);
        Self {
            keys,
            shard_committee,
            instances: vec![],
            previous_leader: None,
            shard_range,
            timeout,
        }
    }

    pub fn init_instances(&mut self, epoch_manager: RangeEpochManager<RistrettoPublicKey>) {
        self.instances = self
            .keys
            .iter()
            .map(|(private, public)| {
                HsTestHarness::new(
                    private.clone(),
                    public.clone(),
                    epoch_manager.clone(),
                    RotatingLeader {},
                    self.timeout,
                )
            })
            .collect::<Vec<_>>();
    }

    pub fn get_range_with_keys(&self) -> (Range<ShardId>, Vec<RistrettoPublicKey>) {
        (self.shard_range.clone(), self.shard_committee.clone())
    }

    pub fn get_shard_id(&self) -> ShardId {
        self.shard_range.start
    }

    pub async fn remove_leader(&mut self) {
        self.previous_leader = Some(self.shard_committee.remove(0));
        let shutdown = timeout(*TEN_SECONDS, self.instances.remove(0).assert_shuts_down_safely()).await;
        if shutdown.is_err() {
            println!("SHUTDOWN PROBLEM : Instance can't be shutdown properly");
        }
        // assert!(shutdown.is_ok(), "Instance can't be shutdown properly");
    }

    pub async fn send_tx_new(&self, payload: &TariDanPayload) {
        let sends = timeout(
            *TEN_SECONDS,
            join_all(
                self.instances
                    .iter()
                    .map(|instance| instance.tx_new.send((payload.clone(), self.shard_range.start)))
                    .collect::<Vec<_>>(),
            ),
        )
        .await;
        assert!(sends.is_ok(), "Send should not fail");
        let sends = sends.unwrap();
        assert!(sends.iter().all(Result::is_ok), "Send should not fail");
    }

    pub async fn receive_rx_leader(&mut self) -> Vec<HotStuffMessage<TariDanPayload, RistrettoPublicKey>> {
        let hs_message = timeout(
            *TEN_SECONDS,
            join_all(
                self.instances
                    .iter_mut()
                    .map(|instance| instance.rx_leader.recv())
                    .collect::<Vec<_>>(),
            ),
        )
        .await;
        assert!(hs_message.is_ok(), "Replicas should send ");
        let hs_message = hs_message.unwrap();
        assert!(hs_message.iter().all(Option::is_some), "Replicas send empty message");
        let hs_message = hs_message
            .into_iter()
            .map(|message| message.unwrap())
            .collect::<Vec<_>>();
        assert!(
            hs_message
                .iter()
                .all(|(dest_key, _)| dest_key == &self.shard_committee[0]),
            "{:?} {:?}",
            hs_message,
            self.shard_committee
        );
        hs_message.into_iter().map(|(_, message)| message).collect::<Vec<_>>()
    }

    pub async fn leader_receive_messages(&self, messages: Vec<HotStuffMessage<TariDanPayload, RistrettoPublicKey>>) {
        let sent_messages = timeout(
            *TEN_SECONDS,
            join_all(
                messages
                    .into_iter()
                    .zip(self.shard_committee.iter())
                    .map(|(message, from)| self.instances[0].tx_hs_messages.send((from.clone(), message)))
                    .collect::<Vec<_>>(),
            ),
        )
        .await;
        assert!(sent_messages.is_ok(), "Messages should be send");
        let sent_message = sent_messages.unwrap();
        assert!(
            sent_message.iter().all(Result::is_ok),
            "There was a problem sending message"
        );
    }

    pub async fn get_proposal(
        &mut self,
    ) -> (
        HotStuffMessage<TariDanPayload, RistrettoPublicKey>,
        Vec<RistrettoPublicKey>,
    ) {
        let proposal = timeout(*TEN_SECONDS, self.instances[0].rx_broadcast.recv()).await;
        assert!(proposal.is_ok(), "Proposal should be send");
        let proposal = proposal.unwrap();
        assert!(proposal.is_some(), "Proposal should not be empty");
        proposal.unwrap()
    }

    pub async fn send_proposal(&self, proposal: HotStuffMessage<TariDanPayload, RistrettoPublicKey>) {
        let proposed_by = proposal.node().unwrap().proposed_by();
        let sent_proposals = timeout(
            *TEN_SECONDS,
            join_all(
                self.instances
                    .iter()
                    .map(|instance| instance.tx_hs_messages.send((proposed_by.clone(), proposal.clone())))
                    .collect::<Vec<_>>(),
            ),
        )
        .await;
        assert!(sent_proposals.is_ok(), "The proposal should be send");
        let sent_proposals = sent_proposals.unwrap();
        assert!(sent_proposals.iter().all(Result::is_ok), "Proposal send error");
    }

    pub async fn get_votes(&mut self) -> Vec<VoteMessage> {
        let votes = timeout(
            *TEN_SECONDS,
            join_all(
                self.instances
                    .iter_mut()
                    .map(|instance| instance.rx_vote_message.recv()),
            ),
        )
        .await;
        assert!(votes.is_ok(), "Votes should be send");
        let votes = votes.unwrap();
        assert!(votes.iter().all(Option::is_some), "Vote should't be empty");
        let votes = votes.into_iter().map(|vote| vote.unwrap()).collect::<Vec<_>>();
        assert!(
            votes.iter().all(|(_, dest_key)| dest_key == &self.shard_committee[0]),
            "Committee should send votes to the current leader"
        );
        votes.into_iter().map(|(vote, _)| vote).collect::<Vec<_>>()
    }

    pub async fn leader_receive_votes(&self, votes: Vec<VoteMessage>) {
        let sent_votes = timeout(
            *TEN_SECONDS,
            join_all(
                votes
                    .into_iter()
                    .zip(self.shard_committee.iter())
                    .map(|(vote, from)| self.instances[0].tx_votes.send((from.clone(), vote)))
                    .collect::<Vec<_>>(),
            ),
        )
        .await;
        assert!(sent_votes.is_ok(), "Votes should be send");
        let sent_votes = sent_votes.unwrap();
        assert!(sent_votes.iter().all(Result::is_ok), "Vote send error");
    }

    pub fn committee_is_subset_of(&self, all_committees: Vec<RistrettoPublicKey>) {
        assert!(
            self.shard_committee
                .iter()
                .all(|member| all_committees.contains(member)),
            "Committee is not part of all committee, that means the message was not send to this committee"
        );
    }

    // pub fn get_previous_leader(&self) -> &RistrettoPublicKey {
    //     self.previous_leader.as_ref().unwrap()
    // }

    // pub fn get_current_leader(&self) -> &RistrettoPublicKey {
    //     &self.shard_committee[0]
    // }
}

pub struct Test {
    payload: TariDanPayload,
    committees: Vec<Committee>,
}

impl Test {
    // Each committee is a tuple of (size of the committe, timeout), each committee will be used for different shard
    pub fn init(committees: Vec<(usize, Duration)>) -> Self {
        let shards_ranges = vec![*SHARD0..*SHARD1, *SHARD1..*SHARD2, *SHARD2..*SHARD3];
        assert!(
            committees.len() <= shards_ranges.len(),
            "Specify more shards when you want to use bigger committees"
        );
        // Init all commmittes pubkeys
        let mut committees = committees
            .into_iter()
            .zip(shards_ranges.into_iter())
            .map(|((size, timeout), shard_range)| Committee::init(size, timeout, shard_range))
            .collect::<Vec<_>>();
        // Get all VNs keys flattened
        let registered_vn_keys = committees
            .iter()
            .flat_map(|v| v.shard_committee.to_vec())
            .collect::<Vec<_>>();
        let epoch_manager = RangeEpochManager::new_with_multiple(
            registered_vn_keys,
            committees
                .iter()
                .map(|committee| committee.get_range_with_keys())
                .collect::<Vec<_>>()
                .as_slice(),
        );

        // Init the HsTestHarness instances for the committees.
        for committee in &mut committees {
            committee.init_instances(epoch_manager.clone());
        }

        // Create payload
        let payload = TariDanPayload::new(
            Transaction::builder()
                .with_inputs(
                    committees
                        .iter()
                        .map(|committee| committee.get_shard_id())
                        .collect::<Vec<_>>(),
                )
                .sign(&committees[0].keys[0].0)
                .clone()
                .build(),
        );

        Self { payload, committees }
    }

    // pub async fn remove_leader(&mut self, committee_id: usize) {
    //     self.committees[committee_id].remove_leader();
    // }

    pub async fn send_tx_new_to_all(&mut self) {
        let sent_payloads = timeout(
            *TEN_SECONDS,
            join_all(
                self.committees
                    .iter()
                    .map(|committee| committee.send_tx_new(&self.payload))
                    .collect::<Vec<_>>(),
            ),
        )
        .await;
        assert!(sent_payloads.is_ok(), "The payload should be send");
    }

    pub async fn receive_all_rx_leader(&mut self) -> Vec<Vec<HotStuffMessage<TariDanPayload, RistrettoPublicKey>>> {
        timeout(
            *TEN_SECONDS,
            join_all(
                self.committees
                    .iter_mut()
                    .map(|committee| committee.receive_rx_leader())
                    .collect::<Vec<_>>(),
            ),
        )
        .await
        .unwrap()
    }

    pub async fn get_proposals(
        &mut self,
    ) -> Vec<(
        HotStuffMessage<TariDanPayload, RistrettoPublicKey>,
        Vec<RistrettoPublicKey>,
    )> {
        timeout(
            *TEN_SECONDS,
            join_all(
                self.committees
                    .iter_mut()
                    .map(|committee| committee.get_proposal())
                    .collect::<Vec<_>>(),
            ),
        )
        .await
        .unwrap()
    }

    pub async fn get_all_votes(&mut self) -> Vec<Vec<VoteMessage>> {
        timeout(
            *TEN_SECONDS,
            join_all(
                self.committees
                    .iter_mut()
                    .map(|committee| committee.get_votes())
                    .collect::<Vec<_>>(),
            ),
        )
        .await
        .unwrap()
    }

    pub async fn all_leaders_receive_votes(&self, all_votes: Vec<Vec<VoteMessage>>) {
        timeout(
            *TEN_SECONDS,
            join_all(
                self.committees
                    .iter()
                    .zip(all_votes.into_iter())
                    .map(|(committee, votes)| committee.leader_receive_votes(votes))
                    .collect::<Vec<_>>(),
            ),
        )
        .await
        .unwrap();
    }

    pub async fn send_all_proposals_to_all_committees(
        &self,
        proposals: Vec<(
            HotStuffMessage<TariDanPayload, RistrettoPublicKey>,
            Vec<RistrettoPublicKey>,
        )>,
    ) {
        timeout(
            *TEN_SECONDS,
            join_all(
                proposals
                    .into_iter()
                    .map(|proposal| self.send_proposal_to_all_committees(proposal))
                    .collect::<Vec<_>>(),
            ),
        )
        .await
        .unwrap();
    }

    pub async fn send_proposal_to_all_committees(
        &self,
        proposal: (
            HotStuffMessage<TariDanPayload, RistrettoPublicKey>,
            Vec<RistrettoPublicKey>,
        ),
    ) {
        timeout(
            *TEN_SECONDS,
            join_all(
                self.committees
                    .iter()
                    .map(|committee| committee.send_proposal(proposal.0.clone()))
                    .collect::<Vec<_>>(),
            ),
        )
        .await
        .unwrap();
    }

    pub async fn send_proposal_to_committee(
        &self,
        committee_index: usize,
        proposal: (
            HotStuffMessage<TariDanPayload, RistrettoPublicKey>,
            Vec<RistrettoPublicKey>,
        ),
    ) {
        timeout(*TEN_SECONDS, self.committees[committee_index].send_proposal(proposal.0))
            .await
            .unwrap();
    }

    pub async fn leaders_receive_messages(
        &self,
        messages: Vec<Vec<HotStuffMessage<TariDanPayload, RistrettoPublicKey>>>,
    ) {
        timeout(
            *TEN_SECONDS,
            join_all(
                self.committees
                    .iter()
                    .zip(messages.into_iter())
                    .map(|(committee, messages)| committee.leader_receive_messages(messages))
                    .collect::<Vec<_>>(),
            ),
        )
        .await
        .unwrap();
    }

    // This is just for tests that use single committee to make it easier to read the test.
    pub fn remove_committee(&mut self, index: usize) -> Committee {
        self.committees.remove(index)
    }

    pub async fn remove_leader_from_committee(&mut self, index: usize) {
        self.committees[index].remove_leader().await;
    }

    pub fn get_payload(&self) -> &TariDanPayload {
        &self.payload
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_local_leader_failure_multiple() {
    // This test that we have pacemaker in the local_leader_failure_trigger.
    let mut test = Test::init(vec![(13, *ONE_SEC)]);
    let mut committee = test.remove_committee(0);
    committee.send_tx_new(test.get_payload()).await;
    // We send original new_view. There is a check in the receive_rx_leader that the messages are indeed for the
    // current_leader.
    let _new_view_messages = committee.receive_rx_leader().await;
    committee.remove_leader().await;
    // That failed, so pacemaker triggered and we send new new_view.
    let _new_view_messages = committee.receive_rx_leader().await;
    committee.remove_leader().await;
    // Also that failed so we send new new_view again.
    let new_view_messages = committee.receive_rx_leader().await;
    // Let's send the messages to the leader
    committee.leader_receive_messages(new_view_messages).await;
    // Just to be on the safe side the newest leader should send the proposal.
    committee.get_proposal().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_local_leader_failure_whole_tx() {
    // We need 13 members, on every step one will go offline, so f=4
    let mut test = Test::init(vec![(13, *ONE_SEC)]);
    let mut committee = test.remove_committee(0);
    // Remove the leader
    // Send the new TX to replicas

    committee.send_tx_new(test.get_payload()).await;
    // Replicas try to select the leader (which is offline)
    let _new_view_messages = committee.receive_rx_leader().await;
    committee.remove_leader().await;

    // The local leader failure should be triggered and new "new view" message should be send
    let new_view_messages = committee.receive_rx_leader().await;
    // Let's send the messages to the leader
    committee.leader_receive_messages(new_view_messages).await;

    for phase in [
        HotstuffPhase::Prepare,
        HotstuffPhase::PreCommit,
        HotstuffPhase::Commit,
        HotstuffPhase::Decide,
    ] {
        //     // Leader sends proposal
        let (proposal, dest_addresses) = committee.get_proposal().await;
        committee.committee_is_subset_of(dest_addresses);
        assert_eq!(
            proposal.node().unwrap().payload_phase(),
            phase,
            "This should be proposal for phase {:?}",
            phase
        );
        // Leader goes offline
        committee.send_proposal(proposal).await;
        if phase != HotstuffPhase::Decide {
            let _votes = committee.get_votes().await;
            committee.remove_leader().await;
            // The local leader failure should be triggered and new "new view" message should be send
            let new_view_messages = committee.receive_rx_leader().await;
            committee.leader_receive_messages(new_view_messages).await;
            let (proposal, dest_addresses) = committee.get_proposal().await;
            committee.committee_is_subset_of(dest_addresses);
            // The new proposal is still the same phase
            assert_eq!(
                proposal.node().unwrap().payload_phase(),
                phase,
                "This should be proposal for phase {:?}",
                phase
            );
            // The new leader sends new proposal
            committee.send_proposal(proposal).await;
            let votes = committee.get_votes().await;
            committee.leader_receive_votes(votes).await;
        }
    }
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
        *NEVER,
    );
    // node1_instance.add_package(package);
    let node2_instance = HsTestHarness::new(
        node2_pk.clone(),
        node2.clone(),
        epoch_manager,
        AlwaysFirstLeader {},
        *NEVER,
    );
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
