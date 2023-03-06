//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_crypto::commitment::HomomorphicCommitmentFactory;
use tari_dan_wallet_sdk::{
    confidential::get_commitment_factory,
    models::{ConfidentialOutput, OutputStatus},
    storage::{WalletStore, WalletStoreReader},
    DanWalletSdk,
    WalletSdkConfig,
};
use tari_dan_wallet_storage_sqlite::SqliteWalletStore;

#[test]
fn outputs_locked_and_released() {
    let temp = tempfile::tempdir().unwrap();
    let store = SqliteWalletStore::try_open(temp.path().join("data/wallet.sqlite")).unwrap();
    store.run_migrations().unwrap();

    let sdk = DanWalletSdk::initialize(store.clone(), WalletSdkConfig {
        password: None,
        validator_node_jrpc_endpoint: "".to_string(),
    })
    .unwrap();

    let accounts_api = sdk.accounts_api();
    accounts_api
        .add_account(
            Some("test"),
            &"component_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a071d15f562"
                .parse()
                .unwrap(),
            0,
        )
        .unwrap();

    let commitment_25 = get_commitment_factory().commit_value(&Default::default(), 25);
    let commitment_49 = get_commitment_factory().commit_value(&Default::default(), 49);
    let commitment_100 = get_commitment_factory().commit_value(&Default::default(), 100);

    let outputs_api = sdk.confidential_outputs_api();
    let proof_id = outputs_api.add_proof("test".to_string()).unwrap();
    outputs_api
        .add_output(ConfidentialOutput {
            account_name: "test".to_string(),
            commitment: commitment_100,
            value: 100,
            sender_public_nonce: None,
            secret_key_index: 0,
            public_asset_tag: None,
            status: OutputStatus::Unspent,
            locked_by_proof: None,
        })
        .unwrap();
    outputs_api
        .add_output(ConfidentialOutput {
            account_name: "test".to_string(),
            commitment: commitment_25.clone(),
            value: 25,
            sender_public_nonce: None,
            secret_key_index: 0,
            public_asset_tag: None,
            status: OutputStatus::Unspent,
            locked_by_proof: None,
        })
        .unwrap();
    outputs_api
        .add_output(ConfidentialOutput {
            account_name: "test".to_string(),
            commitment: commitment_49.clone(),
            value: 49,
            sender_public_nonce: None,
            secret_key_index: 0,
            public_asset_tag: None,
            status: OutputStatus::Unspent,
            locked_by_proof: None,
        })
        .unwrap();
    let (inputs, total_value) = outputs_api.lock_outputs_by_amount("test", 50, proof_id).unwrap();
    assert_eq!(total_value, 74);
    assert_eq!(inputs.len(), 2);

    let locked = store
        .with_read_tx(|tx| tx.outputs_get_locked_by_proof(proof_id))
        .unwrap();

    assert!(locked.iter().any(|l| l.commitment == commitment_25));
    assert!(locked.iter().any(|l| l.commitment == commitment_49));
    assert_eq!(locked.len(), 2);

    outputs_api.release_proof_outputs(proof_id).unwrap();

    let locked = store
        .with_read_tx(|tx| tx.outputs_get_locked_by_proof(proof_id))
        .unwrap();
    assert_eq!(locked.len(), 0);
}

#[test]
fn outputs_locked_and_finalized() {
    let temp = tempfile::tempdir().unwrap();
    let store = SqliteWalletStore::try_open(temp.path().join("data/wallet.sqlite")).unwrap();
    store.run_migrations().unwrap();

    let sdk = DanWalletSdk::initialize(store.clone(), WalletSdkConfig {
        password: None,
        validator_node_jrpc_endpoint: "".to_string(),
    })
    .unwrap();

    let accounts_api = sdk.accounts_api();
    accounts_api
        .add_account(
            Some("test"),
            &"component_0dc41b5cc74b36d696c7b140323a40a2f98b71df5d60e5a6bf4c1a071d15f562"
                .parse()
                .unwrap(),
            0,
        )
        .unwrap();

    let commitment_25 = get_commitment_factory().commit_value(&Default::default(), 25);
    let commitment_49 = get_commitment_factory().commit_value(&Default::default(), 49);
    let commitment_100 = get_commitment_factory().commit_value(&Default::default(), 100);

    let outputs_api = sdk.confidential_outputs_api();
    let proof_id = outputs_api.add_proof("test".to_string()).unwrap();
    outputs_api
        .add_output(ConfidentialOutput {
            account_name: "test".to_string(),
            commitment: commitment_100.clone(),
            value: 100,
            sender_public_nonce: None,
            secret_key_index: 0,
            public_asset_tag: None,
            status: OutputStatus::Unspent,
            locked_by_proof: None,
        })
        .unwrap();
    outputs_api
        .add_output(ConfidentialOutput {
            account_name: "test".to_string(),
            commitment: commitment_25.clone(),
            value: 25,
            sender_public_nonce: None,
            secret_key_index: 0,
            public_asset_tag: None,
            status: OutputStatus::Unspent,
            locked_by_proof: None,
        })
        .unwrap();
    outputs_api
        .add_output(ConfidentialOutput {
            account_name: "test".to_string(),
            commitment: commitment_49.clone(),
            value: 49,
            sender_public_nonce: None,
            secret_key_index: 0,
            public_asset_tag: None,
            status: OutputStatus::Unspent,
            locked_by_proof: None,
        })
        .unwrap();
    let (inputs, total_value) = outputs_api.lock_outputs_by_amount("test", 50, proof_id).unwrap();
    assert_eq!(total_value, 74);
    assert_eq!(inputs.len(), 2);

    let locked = store
        .with_read_tx(|tx| tx.outputs_get_locked_by_proof(proof_id))
        .unwrap();

    assert!(locked.iter().any(|l| l.commitment == commitment_25));
    assert!(locked.iter().any(|l| l.commitment == commitment_49));
    assert_eq!(locked.len(), 2);

    // Add a change output belonging to this proof
    let commitment_change = get_commitment_factory().commit_value(&Default::default(), 24);
    outputs_api
        .add_output(ConfidentialOutput {
            account_name: "test".to_string(),
            commitment: commitment_change.clone(),
            value: 24,
            sender_public_nonce: None,
            secret_key_index: 0,
            public_asset_tag: None,
            status: OutputStatus::LockedUnconfirmed,
            locked_by_proof: Some(proof_id),
        })
        .unwrap();

    outputs_api.finalize_outputs_for_proof(proof_id).unwrap();

    {
        let mut tx = store.create_read_tx().unwrap();
        let locked = tx.outputs_get_locked_by_proof(proof_id).unwrap();
        assert_eq!(locked.len(), 0);

        let unspent = tx
            .outputs_get_by_account_and_status("test", OutputStatus::Unspent)
            .unwrap();
        assert!(unspent.iter().any(|l| l.commitment == commitment_change));
        assert!(unspent.iter().any(|l| l.commitment == commitment_100));
        assert_eq!(unspent.len(), 2);
        let balance = tx.outputs_get_unspent_balance("test").unwrap();
        assert_eq!(balance, 124);
    }
}
