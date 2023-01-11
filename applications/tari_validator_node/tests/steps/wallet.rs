//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use cucumber::{given, when};
use tari_app_grpc::tari_rpc::GetBalanceRequest;
use tokio::time::sleep;

use crate::{spawn_wallet, TariWorld};

#[given(expr = "a wallet {word} connected to base node {word}")]
async fn start_wallet(world: &mut TariWorld, wallet_name: String, bn_name: String) {
    spawn_wallet(world, wallet_name, bn_name).await;
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
