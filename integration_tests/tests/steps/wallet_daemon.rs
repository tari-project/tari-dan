//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use cucumber::{then, when};
use integration_tests::{wallet_daemon_cli, TariWorld};
use tari_common_types::types::{Commitment, PrivateKey, PublicKey};
use tari_crypto::{ristretto::RistrettoComSig, tari_utilities::ByteArray};
use tari_template_lib::prelude::Amount;
use tari_wallet_daemon_client::ComponentAddressOrName;

#[when(
    expr = "I claim burn {word} with {word}, {word} and {word} and spend it into account {word} via the wallet daemon \
            {word}"
)]
async fn when_i_claim_burn_via_wallet_daemon(
    world: &mut TariWorld,
    commitment_name: String,
    proof_name: String,
    rangeproof_name: String,
    claim_pubkey_name: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    let commitment = world
        .commitments
        .get(&commitment_name)
        .unwrap_or_else(|| panic!("Commitment {} not found", commitment_name));
    let proof = world
        .commitment_ownership_proofs
        .get(&proof_name)
        .unwrap_or_else(|| panic!("Proof {} not found", proof_name));
    let rangeproof = world
        .rangeproofs
        .get(&rangeproof_name)
        .unwrap_or_else(|| panic!("Rangeproof {} not found", rangeproof_name));
    let reciprocal_claim_public_key = world
        .claim_public_keys
        .get(&claim_pubkey_name)
        .unwrap_or_else(|| panic!("Claim public key {} not found", claim_pubkey_name));
    let claim_burn_resp = wallet_daemon_cli::claim_burn(
        world,
        account_name,
        commitment.clone(),
        rangeproof.clone(),
        proof.clone(),
        reciprocal_claim_public_key.clone(),
        wallet_daemon_name,
        1000,
    )
    .await
    .unwrap();
    if let Some(ref reason) = claim_burn_resp.result.result.reject() {
        panic!("Transaction failed: {}", reason);
    }
}

#[when(
    expr = "I claim burn {word} with {word}, {word} and {word} and spend it into account {word} via the wallet daemon \
            {word}, it fails"
)]
async fn when_i_claim_burn_via_wallet_daemon_it_fails(
    world: &mut TariWorld,
    commitment_name: String,
    proof_name: String,
    rangeproof_name: String,
    claim_pubkey_name: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    let commitment = world
        .commitments
        .get(&commitment_name)
        .unwrap_or_else(|| panic!("Commitment {} not found", commitment_name));
    let proof = world
        .commitment_ownership_proofs
        .get(&proof_name)
        .unwrap_or_else(|| panic!("Proof {} not found", proof_name));
    let rangeproof = world
        .rangeproofs
        .get(&rangeproof_name)
        .unwrap_or_else(|| panic!("Rangeproof {} not found", rangeproof_name));
    let reciprocal_claim_public_key = world
        .claim_public_keys
        .get(&claim_pubkey_name)
        .unwrap_or_else(|| panic!("Claim public key {} not found", claim_pubkey_name));

    // TODO: The walletd picks up the substate that doesnt exist before the transaction is submitted. This doesnt test
    // the validator node behaviour. We should submit the transaction directly ithout any special claim burn handling
    let _err = wallet_daemon_cli::claim_burn(
        world,
        account_name,
        commitment.clone(),
        rangeproof.clone(),
        proof.clone(),
        reciprocal_claim_public_key.clone(),
        wallet_daemon_name,
        1000,
    )
    .await
    .unwrap_err();
}

#[when(expr = "I claim fees for validator {word} and epoch {int} into account {word} using the wallet daemon {word}")]
async fn when_i_claim_fees_for_validator_and_epoch(
    world: &mut TariWorld,
    validator_node: String,
    epoch: u64,
    account_name: String,
    wallet_daemon_name: String,
) {
    let resp = wallet_daemon_cli::claim_fees(world, wallet_daemon_name, account_name, validator_node, epoch)
        .await
        .unwrap();
    resp.result.result.accept().unwrap_or_else(|| {
        panic!(
            "Expected fee claim to succeeded but failed with {}",
            resp.result.result.reject().unwrap()
        )
    });
}

#[when(
    expr = "I claim fees for validator {word} and epoch {int} into account {word} using the wallet daemon {word}, it \
            fails"
)]
async fn when_i_claim_fees_for_validator_and_epoch_fails(
    world: &mut TariWorld,
    validator_node: String,
    epoch: u64,
    account_name: String,
    wallet_daemon_name: String,
) {
    let err = wallet_daemon_cli::claim_fees(world, wallet_daemon_name, account_name, validator_node, epoch)
        .await
        .unwrap_err();

    println!("Expected error: {}", err);
}

#[then(
    expr = "I make a confidential transfer with amount {int} from {word} to {word} creating output {word} via the \
            wallet_daemon {word}"
)]
async fn when_i_create_transfer_proof_via_wallet_daemon(
    world: &mut TariWorld,
    amount: u64,
    source_account_name: String,
    dest_account_name: String,
    outputs_name: String,
    wallet_daemon_name: String,
) {
    wallet_daemon_cli::transfer_confidential(
        world,
        source_account_name,
        dest_account_name,
        amount,
        wallet_daemon_name,
        outputs_name,
        None,
        None,
    )
    .await;
}

#[when(expr = "I create an account {word} via the wallet daemon {word}")]
async fn when_i_create_account_via_wallet_daemon(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
) {
    wallet_daemon_cli::create_account(world, account_name, wallet_daemon_name).await;
}

#[when(expr = "I create an account {word} via the wallet daemon {word} with {int} free coins")]
async fn when_i_create_account_via_wallet_daemon_with_free_coins(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    amount: i64,
) {
    wallet_daemon_cli::create_account_with_free_coins(world, account_name, wallet_daemon_name, amount.into(), None)
        .await;
}

#[when(expr = "I create a key named {word} for {word}")]
async fn when_i_create_a_wallet_key(world: &mut TariWorld, key_name: String, wallet_daemon_name: String) {
    let mut client = world.get_wallet_daemon(&wallet_daemon_name).get_authed_client().await;
    let key = client.create_key().await.unwrap();
    world.wallet_keys.insert(key_name, key.id);
}

#[when(expr = "I create an account {word} via the wallet daemon {word} with {int} free coins using key {word}")]
async fn when_i_create_account_via_wallet_daemon_with_free_coins_using_key(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    amount: i64,
    key_name: String,
) {
    wallet_daemon_cli::create_account_with_free_coins(
        world,
        account_name,
        wallet_daemon_name,
        amount.into(),
        Some(key_name),
    )
    .await;
}

#[when(
    expr = "I burn {int}T on wallet {word} with wallet daemon {word} into commitment {word} with proof {word} for \
            {word}, range proof {word} and claim public key {word}"
)]
async fn when_i_burn_funds_with_wallet_daemon(
    world: &mut TariWorld,
    amount: u64,
    wallet_name: String,
    wallet_daemon_name: String,
    commitment_name: String,
    ownership_proof_name: String,
    account_name: String,
    rangeproof_name: String,
    claim_pubkey_name: String,
) {
    let mut wallet_daemon_client = wallet_daemon_cli::get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let account = wallet_daemon_client
        .accounts_get(account_name.parse().unwrap())
        .await
        .unwrap();
    let public_key = account.public_key;

    let wallet = world
        .wallets
        .get(&wallet_name)
        .unwrap_or_else(|| panic!("Wallet {} not found", wallet_name));

    let mut client = wallet.create_client().await;
    let resp = client
        .create_burn_transaction(minotari_app_grpc::tari_rpc::CreateBurnTransactionRequest {
            amount: amount * 1_000_000,
            fee_per_gram: 1,
            message: "Burn".to_string(),
            claim_public_key: public_key.to_vec(),
        })
        .await
        .unwrap()
        .into_inner();

    assert!(resp.is_success);
    world.commitments.insert(commitment_name, resp.commitment);
    // TODO: use proto::transaction::CommitmentSignature to deserialize once we update tari to include https://github.com/tari-project/tari/pull/5200
    let ownership_proof = resp.ownership_proof.unwrap();
    world.commitment_ownership_proofs.insert(
        ownership_proof_name,
        RistrettoComSig::new(
            Commitment::from_public_key(&PublicKey::from_canonical_bytes(&ownership_proof.public_nonce).unwrap()),
            PrivateKey::from_canonical_bytes(&ownership_proof.u).unwrap(),
            PrivateKey::from_canonical_bytes(&ownership_proof.v).unwrap(),
        ),
    );
    world.rangeproofs.insert(rangeproof_name, resp.range_proof);
    world.claim_public_keys.insert(
        claim_pubkey_name,
        PublicKey::from_canonical_bytes(&resp.reciprocal_claim_public_key).unwrap(),
    );
}

#[when(expr = "I check the balance of {word} on wallet daemon {word} the amount is at {word} {int}")]
async fn check_account_balance_via_daemon(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    least_or_most: String,
    amount: i64,
) {
    // This also refreshes the wallet vaults
    let current_balance = wallet_daemon_cli::get_balance(world, &account_name, &wallet_daemon_name).await;
    match least_or_most.to_lowercase().as_str() {
        "least" => {
            if current_balance < amount {
                println!("Expected balance to be at least {} but was {}", amount, current_balance);
                panic!("Expected balance to be at least {} but was {}", amount, current_balance);
            }
        },
        "most" => {
            if current_balance > amount {
                println!("Expected balance to be at most {} but was {}", amount, current_balance);
                panic!("Expected balance to be at most {} but was {}", amount, current_balance);
            }
        },
        _ => panic!("Expected least or most, got {}", least_or_most),
    }
}

#[when(expr = "I wait for {word} on wallet daemon {word} to have balance {word} {int}")]
async fn wait_account_balance_via_daemon(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    operator: String,
    amount: i64,
) {
    let op = match operator.as_str() {
        "gt" => |a, b| a > b,
        "gte" => |a, b| a >= b,
        "lt" => |a, b| a < b,
        "lte" => |a, b| a <= b,
        "eq" => |a, b| a == b,
        _ => panic!("Expected gt, gte, lt, lte or eq, got {}", operator),
    };

    let mut i = 0;
    loop {
        // This also refreshes the wallet vaults
        let current_balance = wallet_daemon_cli::get_balance(world, &account_name, &wallet_daemon_name).await;
        if op(current_balance, amount) {
            break;
        }

        i += 1;
        if i == 10 {
            panic!("Timeout waiting for balance. Current balance = {}", current_balance);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[when(expr = "I check the balance of {word} on wallet daemon {word} the amount is exactly {int}")]
async fn check_account_balance_is_exactly_via_daemon(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    amount: i64,
) {
    // THis refreshes
    let current_balance = wallet_daemon_cli::get_balance(world, &account_name, &wallet_daemon_name).await;
    if current_balance != amount {
        println!("Expected balance to be {} but was {}", amount, current_balance);
        panic!("Expected balance to be {} but was {}", amount, current_balance);
    }
}

#[when(expr = "I check the confidential balance of {word} on wallet daemon {word} the amount is at {word} {int}")]
async fn check_account_confidential_balance_is_via_daemon(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    least_or_most: String,
    amount: i64,
) {
    // This also refreshes the wallet vaults
    let current_balance = wallet_daemon_cli::get_confidential_balance(world, account_name, wallet_daemon_name).await;
    match least_or_most.to_lowercase().as_str() {
        "least" => {
            if current_balance.value() < amount {
                println!("Expected balance to be at least {} but was {}", amount, current_balance);
                panic!("Expected balance to be at least {} but was {}", amount, current_balance);
            }
        },
        "most" => {
            if current_balance.value() > amount {
                println!("Expected balance to be at most {} but was {}", amount, current_balance);
                panic!("Expected balance to be at most {} but was {}", amount, current_balance);
            }
        },
        _ => panic!("Expected least or most, got {}", least_or_most),
    }
}

#[when(
    expr = "I transfer {int} tokens of resource {word} from account {word} to public key {word} via the wallet daemon \
            {word} named {word}"
)]
async fn when_transfer_via_wallet_daemon(
    world: &mut TariWorld,
    amount: i32,
    resource_address: String,
    account_name: String,
    destination_public_key: String,
    wallet_daemon_name: String,
    outputs_name: String,
) {
    let (_, destination_public_key) = world.account_keys.get(&destination_public_key).unwrap().clone();
    let amount = Amount::new(amount.into());

    let (resource_input_group, resource_name) = resource_address.split_once('/').unwrap_or_else(|| {
        panic!(
            "Resource address must be in the format '{{group}}/resources/{{index}}', got {}",
            resource_address
        )
    });
    let resource_address = world
        .outputs
        .get(resource_input_group)
        .unwrap_or_else(|| panic!("No outputs found with name {}", resource_input_group))
        .iter()
        .find(|(name, _)| **name == resource_name)
        .map(|(_, data)| data.clone())
        .unwrap_or_else(|| panic!("No resource named {}", resource_name))
        .address
        .as_resource_address()
        .unwrap_or_else(|| panic!("{} is not a resource", resource_name));

    wallet_daemon_cli::transfer(
        world,
        account_name,
        destination_public_key,
        resource_address,
        amount,
        wallet_daemon_name,
        outputs_name,
    )
    .await;
}

#[when(
    expr = "I do a confidential transfer of {int} from account {word} to public key {word} via the wallet daemon \
            {word} named {word}"
)]
async fn when_confidential_transfer_via_wallet_daemon(
    world: &mut TariWorld,
    amount: i64,
    account_name: String,
    destination_public_key: String,
    wallet_daemon_name: String,
    outputs_name: String,
) {
    let (_, destination_public_key) = world.account_keys.get(&destination_public_key).unwrap().clone();

    wallet_daemon_cli::confidential_transfer(
        world,
        account_name,
        destination_public_key,
        Amount(amount),
        wallet_daemon_name,
        outputs_name,
    )
    .await;
}

#[when(expr = "I set the default account for {word} to {word}")]
async fn when_i_set_the_default_account(world: &mut TariWorld, wallet_name: String, account_name: String) {
    let wallet = world
        .wallet_daemons
        .get(&wallet_name)
        .unwrap_or_else(|| panic!("No wallet daemon named {}", wallet_name));
    let mut client = wallet.get_authed_client().await;
    client
        .accounts_set_default(ComponentAddressOrName::Name(account_name))
        .await
        .unwrap();
}
