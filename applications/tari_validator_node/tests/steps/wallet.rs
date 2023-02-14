//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use cucumber::{given, when};
use tari_app_grpc::tari_rpc::GetBalanceRequest;
use tari_crypto::tari_utilities::ByteArray;
use tokio::time::sleep;

use crate::{spawn_wallet, TariWorld};

#[given(expr = "a wallet {word} connected to base node {word}")]
async fn start_wallet(world: &mut TariWorld, wallet_name: String, bn_name: String) {
    spawn_wallet(world, wallet_name, bn_name).await;
}

#[when(
    expr = "I burn {int}T on wallet {word} into commitment {word} with proof {word} for {word} and range proof {word}"
)]
async fn when_i_burn_on_wallet(
    world: &mut TariWorld,
    amount: u64,
    wallet_name: String,
    commitment: String,
    proof: String,
    account_name: String,
    range_proof: String,
) {
    let wallet = world
        .wallets
        .get(&wallet_name)
        .unwrap_or_else(|| panic!("Wallet {} not found", wallet_name));

    let public_key = world
        .account_public_keys
        .get(&account_name)
        .unwrap_or_else(|| panic!("Account {} not found", account_name));

    let mut client = wallet.create_client().await;
    let resp = client
        .create_burn_transaction(tari_app_grpc::tari_rpc::CreateBurnTransactionRequest {
            amount: amount * 1_000_000,
            fee_per_gram: 1,
            message: "Burn".to_string(),
            claim_public_key: public_key.1.to_vec(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(resp.is_success);
    world.commitments.insert(commitment, resp.commitment);
    world.commitment_ownership_proofs.insert(proof, resp.ownership_proof);
    world.rangeproofs.insert(range_proof, resp.rangeproof);
}

#[when(expr = "wallet {word} has at least {int} uT")]
async fn check_balance(world: &mut TariWorld, wallet_name: String, balance: u64) {
    let wallet = world
        .wallets
        .get(&wallet_name)
        .unwrap_or_else(|| panic!("Wallet {} not found", wallet_name));

    let mut client = wallet.create_client().await;
    let mut iterations = 0;
    loop {
        let resp = client.get_balance(GetBalanceRequest {}).await.unwrap().into_inner();
        if resp.available_balance >= balance {
            break;
        }
        eprintln!(
            "Waiting for wallet {} to have at least {} uT (balance: {} uT, pending: {} uT)",
            wallet_name, balance, resp.available_balance, resp.pending_incoming_balance
        );
        sleep(Duration::from_secs(1)).await;

        if iterations == 20 {
            panic!(
                "Wallet {} did not have at least {} uT after 20 seconds  (balance: {} uT, pending: {} uT)",
                wallet_name, balance, resp.available_balance, resp.pending_incoming_balance
            );
        }
        iterations += 1;
    }
}
