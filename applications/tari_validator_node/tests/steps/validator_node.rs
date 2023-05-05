//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{convert::TryInto, str::FromStr};

use cucumber::{then, when};
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_common_types::{Epoch, ShardId};
use tari_engine_types::{confidential::ConfidentialClaim, substate::SubstateAddress};
use tari_template_lib::{args, prelude::ComponentAddress};
use tari_transaction::Transaction;
use tari_validator_node_client::types::{GetStateRequest, SubmitTransactionRequest, TransactionFinalizeResult};

use crate::{utils::validator_node_cli::create_key, TariWorld};

#[then(expr = "validator node {word} has state at {word}")]
async fn then_validator_node_has_state_at(world: &mut TariWorld, vn_name: String, state_address_name: String) {
    let state_address = world
        .addresses
        .get(&state_address_name)
        .unwrap_or_else(|| panic!("Address {} not found", state_address_name));
    let vn = world
        .validator_nodes
        .get(&vn_name)
        .unwrap_or_else(|| panic!("Validator node {} not found", vn_name));
    let mut client = vn.create_client().await;
    let shard_id = ShardId::from_address(
        &SubstateAddress::from_str(state_address).expect("Invalid state address"),
        0,
    );
    client.get_state(GetStateRequest { shard_id }).await.unwrap();
}

#[when(expr = "I claim burn {word} with {word}, {word} and {word} and spend it into account {word} on {word}")]
async fn when_i_claim_burn(
    world: &mut TariWorld,
    commitment_name: String,
    proof_name: String,
    rangeproof_name: String,
    claim_public_key_name: String,
    account_name: String,
    vn_name: String,
) -> Result<TransactionFinalizeResult, anyhow::Error> {
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
    let vn = world
        .validator_nodes
        .get(&vn_name)
        .unwrap_or_else(|| panic!("Validator node {} not found", vn_name));
    let commitment_shard = ShardId::from_address(
        &SubstateAddress::from_str(&format!("commitment_{}", commitment.to_hex())).expect("Invalid state address"),
        0,
    );

    let (account_secret, _) = world
        .account_public_keys
        .get(&account_name)
        .unwrap_or_else(|| panic!("Account {} not found", account_name));

    let account_address = world.get_account_component_address(&account_name).unwrap();
    let component_address = ComponentAddress::from_str(&account_address).expect("Invalid account address");

    let reciprocal_public_key = world
        .claim_public_keys
        .get(&claim_public_key_name)
        .unwrap_or_else(|| panic!("Claim public key {} not found", claim_public_key_name));

    let account_shard = ShardId::from_address(&SubstateAddress::from_str(&account_address).unwrap(), 0);
    let account_v1_shard = ShardId::from_address(&SubstateAddress::from_str(&account_address).unwrap(), 1);

    let transaction = Transaction::builder()
        .claim_burn(ConfidentialClaim {
            public_key: reciprocal_public_key.clone(),
            output_address: commitment.as_slice().try_into()?,
            range_proof: rangeproof.clone(),
            proof_of_knowledge: proof.clone(),
            withdraw_proof: None,
        })
        .put_last_instruction_output_on_workspace("burn")
        .call_method(component_address, "deposit", args![Workspace("burn")])
        .with_outputs(vec![account_v1_shard])
        .with_inputs(vec![commitment_shard, account_shard])
        .with_new_outputs(1)
        .sign(account_secret)
        .build();

    let request = SubmitTransactionRequest {
        transaction,
        wait_for_result: true,
        wait_for_result_timeout: None,
        is_dry_run: false,
    };

    let mut client = vn.create_client().await;
    let resp = client.submit_transaction(request).await?;
    let result = resp.result.ok_or_else(|| anyhow::anyhow!("Transaction failed"))?;

    Ok(result)
}

#[when(
    expr = "I claim burn {word} with {word}, {word} and {word} and spend it into account {word} on {word} a second \
            time, it fails"
)]
async fn when_i_claim_burn_second_time_fails(
    world: &mut TariWorld,
    commitment_name: String,
    proof_name: String,
    rangeproof_name: String,
    claim_pk_name: String,
    account_name: String,
    vn_name: String,
) {
    let result = when_i_claim_burn(
        world,
        commitment_name,
        proof_name,
        rangeproof_name,
        claim_pk_name,
        account_name,
        vn_name,
    )
    .await
    .unwrap();
    let reason = result.transaction_failure.expect("Transaction should have failed");
    eprintln!("Expected transaction failure. Reason: {}", reason);
}

#[then(expr = "{word} is on epoch {int} within {int} seconds")]
async fn vn_has_scanned_to_epoch(world: &mut TariWorld, vn_name: String, epoch: u64, seconds: usize) {
    let epoch = Epoch(epoch);
    let vn = world
        .validator_nodes
        .get(&vn_name)
        .unwrap_or_else(|| panic!("Validator node {} not found", vn_name));
    let mut client = vn.create_client().await;
    for _ in 0..seconds {
        let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
        if stats.current_epoch == epoch {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
    assert_eq!(stats.current_epoch, epoch);
}

#[then(expr = "{word} has scanned to height {int} within {int} seconds")]
async fn vn_has_scanned_to_height(world: &mut TariWorld, vn_name: String, block_height: u64, seconds: usize) {
    let vn = world
        .validator_nodes
        .get(&vn_name)
        .unwrap_or_else(|| panic!("Validator node {} not found", vn_name));
    let mut client = vn.create_client().await;
    for _ in 0..seconds {
        let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
        if stats.current_block_height == block_height {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    let stats = client.get_epoch_manager_stats().await.expect("Failed to get stats");
    assert_eq!(stats.current_block_height, block_height);
}

#[when(expr = "I create a new key pair {word}")]
async fn when_i_create_new_key_pair(world: &mut TariWorld, key_name: String) {
    create_key(world, key_name).await;
}
