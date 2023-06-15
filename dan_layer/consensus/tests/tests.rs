//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[test]
fn todo_write_some_tests() {}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_hs_worker_leader_proposes() {
    let (node1_pk, node1) = PublicKey::random_keypair(&mut OsRng);
    let (node2_pk, node2) = PublicKey::random_keypair(&mut OsRng);
    let registered_vn_keys = vec![node1.clone(), node2.clone()];
    // let epoch_manager = RangeEpochManager::new(registered_vn_keys, SHARD0..SHARD1, vec![node1.clone(), node2.clone()]);
    // let mut instance = HsTestHarness::new(
    //     node1_pk.clone(),
    //     node1.clone(),
    //     epoch_manager,
    //     AlwaysFirstLeader {},
    //     NEVER,
    // );
    // let payload = ("Hello World".to_string(), vec![SHARD0]);
    // let payload = TariDanPayload::new(Transaction::builder().add_input(SHARD0).sign(&node1_pk).build());
    //
    // let qc = create_test_default_qc(
    //     vec![(node1.clone(), node1_pk), (node2.clone(), node2_pk)],
    //     vec![node1.clone(), node2.clone()],
    //     &payload,
    //     node1.clone(),
    // );
    // let new_view_message = HotStuffMessage::new_view(qc, SHARD0, payload);
    //
    // instance
    //     .tx_hs_messages
    //     .send((node1.clone(), new_view_message.clone()))
    //     .await
    //     .unwrap();
    // instance
    //     .tx_hs_messages
    //     .send((node2.clone(), new_view_message))
    //     .await
    //     .unwrap();
    //
    // let (_, mut broadcast_group) = instance.recv_broadcast().await;
    //
    // broadcast_group.sort();
    // let mut all_nodes = vec![node1, node2];
    // all_nodes.sort();
    // assert_eq!(broadcast_group, all_nodes);
    // instance.assert_shuts_down_safely().await
}
