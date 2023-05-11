//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use cucumber::{then, when};
use tari_common_types::types::{Commitment, PrivateKey, PublicKey};
use tari_crypto::{ristretto::RistrettoComSig, tari_utilities::ByteArray};
use tari_template_lib::prelude::Amount;

use crate::{utils::wallet_daemon_cli, TariWorld};

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
    )
    .await;

    assert!(claim_burn_resp.result.is_accept());
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
    let mut wallet_daemon_client = wallet_daemon_cli::get_auth_wallet_daemon_client(world, wallet_daemon_name).await;

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
        .create_burn_transaction(tari_app_grpc::tari_rpc::CreateBurnTransactionRequest {
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
            Commitment::from_public_key(&PublicKey::from_bytes(&ownership_proof.public_nonce).unwrap()),
            PrivateKey::from_bytes(&ownership_proof.u).unwrap(),
            PrivateKey::from_bytes(&ownership_proof.v).unwrap(),
        ),
    );
    world.rangeproofs.insert(rangeproof_name, resp.range_proof);
    world.claim_public_keys.insert(
        claim_pubkey_name,
        PublicKey::from_bytes(&resp.reciprocal_claim_public_key).unwrap(),
    );
}

#[when(expr = "I check the balance of {word} on wallet daemon {word} the amount is at least {int}")]
async fn check_account_balance_is_at_least_via_daemon(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    amount: i64,
) {
    let current_balance = wallet_daemon_cli::get_balance(world, account_name, wallet_daemon_name).await;
    assert!(current_balance >= amount);
}

#[when(expr = "I check the balance of {word} on wallet daemon {word} the amount is at most {int}")]
async fn check_account_balance_is_at_most_daemon(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    amount: i64,
) {
    let current_balance = wallet_daemon_cli::get_balance(world, account_name, wallet_daemon_name).await;
    assert!(current_balance <= amount);
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
    let (_, destination_public_key) = world.account_public_keys.get(&destination_public_key).unwrap().clone();
    let amount = Amount::new(amount.into());

    let (resource_input_group, resource_name) = resource_address.split_once('/').unwrap_or_else(|| {
        panic!(
            "Resource address must be in the format '{{group}}/resources/{{index}}', got {}",
            resource_address
        )
    });
    let resource_address = world
        .wallet_daemon_outputs
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
    expr = "I do a confidential transfer of {int}T from account {word} to public key {word} via the wallet daemon \
            {word} named {word}"
)]
async fn when_confidential_transfer_via_wallet_daemon(
    world: &mut TariWorld,
    amount: i32,
    account_name: String,
    destination_public_key: String,
    wallet_daemon_name: String,
    outputs_name: String,
) {
    let (_, destination_public_key) = world.account_public_keys.get(&destination_public_key).unwrap().clone();
    let amount = Amount::new(amount.into());

    wallet_daemon_cli::confidential_transfer(
        world,
        account_name,
        destination_public_key,
        amount,
        wallet_daemon_name,
        outputs_name,
    )
    .await;
}
