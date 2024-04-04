//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use cucumber::{given, when};
use minotari_app_grpc::tari_rpc::{GetBalanceRequest, ValidateRequest};
use tari_common_types::types::{Commitment, PrivateKey, PublicKey};
use tari_crypto::{ristretto::RistrettoComSig, tari_utilities::ByteArray};
use tokio::time::sleep;

use crate::{spawn_wallet, TariWorld};

#[given(expr = "a wallet {word} connected to base node {word}")]
async fn start_wallet(world: &mut TariWorld, wallet_name: String, bn_name: String) {
    spawn_wallet(world, wallet_name, bn_name).await;
}

#[when(
    expr = "I burn {int}T on wallet {word} into commitment {word} with proof {word} for {word}, range proof {word} \
            and claim public key {word}"
)]
async fn when_i_burn_on_wallet(
    world: &mut TariWorld,
    amount: u64,
    wallet_name: String,
    commitment: String,
    proof: String,
    account_name: String,
    range_proof: String,
    claim_public_key_name: String,
) {
    let wallet = world
        .wallets
        .get(&wallet_name)
        .unwrap_or_else(|| panic!("Wallet {} not found", wallet_name));

    let (_, public_key) = world
        .account_keys
        .get(&account_name)
        .unwrap_or_else(|| panic!("Account {} not found", account_name));

    let mut client = wallet.create_client().await;
    let resp = client
        .create_burn_transaction(minotari_app_grpc::tari_rpc::CreateBurnTransactionRequest {
            amount: amount * 1_000_000,
            fee_per_gram: 1,
            message: "Burn".to_string(),
            claim_public_key: public_key.to_vec(),
            sidechain_id: vec![],
            sidechain_id_knowledge_proof: None
        })
        .await
        .unwrap()
        .into_inner();

    assert!(resp.is_success);
    world.commitments.insert(commitment, resp.commitment);
    // TODO: use proto::transaction::CommitmentSignature to deserialize once we update tari to include https://github.com/tari-project/tari/pull/5200
    let ownership_proof = resp.ownership_proof.unwrap();
    world.commitment_ownership_proofs.insert(
        proof,
        RistrettoComSig::new(
            Commitment::from_public_key(&PublicKey::from_canonical_bytes(&ownership_proof.public_nonce).unwrap()),
            PrivateKey::from_canonical_bytes(&ownership_proof.u).unwrap(),
            PrivateKey::from_canonical_bytes(&ownership_proof.v).unwrap(),
        ),
    );
    world.rangeproofs.insert(range_proof, resp.range_proof);
    world.claim_public_keys.insert(
        claim_public_key_name,
        PublicKey::from_canonical_bytes(&resp.reciprocal_claim_public_key).unwrap(),
    );
}

#[when(expr = "wallet {word} has at least {int} {word}")]
async fn check_balance(world: &mut TariWorld, wallet_name: String, balance: u64, units: String) {
    let wallet = world
        .wallets
        .get(&wallet_name)
        .unwrap_or_else(|| panic!("Wallet {} not found", wallet_name));

    let mut client = wallet.create_client().await;
    let mut iterations = 0;
    let balance = match units.as_str() {
        "T" => balance * 1_000_000,
        "uT" => balance,
        _ => panic!("Unknown unit {}", units),
    };
    loop {
        let _result = client.validate_all_transactions(ValidateRequest {}).await;
        let resp = client.get_balance(GetBalanceRequest {}).await.unwrap().into_inner();
        if resp.available_balance >= balance {
            break;
        }
        eprintln!(
            "Waiting for wallet {} to have at least {} uT (balance: {} uT, pending: {} uT)",
            wallet_name, balance, resp.available_balance, resp.pending_incoming_balance
        );
        sleep(Duration::from_secs(1)).await;

        if iterations == 40 {
            panic!(
                "Wallet {} did not have at least {} uT after 40 seconds  (balance: {} uT, pending: {} uT)",
                wallet_name, balance, resp.available_balance, resp.pending_incoming_balance
            );
        }
        iterations += 1;
    }
}
