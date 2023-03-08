//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{convert::TryInto, str::FromStr};

use cucumber::{then, when};
use tari_crypto::tari_utilities::{hex::Hex, ByteArray};
use tari_dan_common_types::{Epoch, ShardId};
use tari_engine_types::{confidential::ConfidentialClaim, instruction::Instruction, substate::SubstateAddress};
use tari_template_lib::{args::Arg, prelude::ComponentAddress};
use tari_transaction::Transaction;
use tari_validator_node_client::types::{GetStateRequest, SubmitTransactionRequest};

use crate::TariWorld;

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

#[when(expr = "I claim burn {word} with {word} and {word} and spend it into account {word} on {word}")]
async fn when_i_claim_burn(
    world: &mut TariWorld,
    commitment_name: String,
    proof_name: String,
    rangeproof_name: String,
    account_name: String,
    vn_name: String,
) -> Result<(), anyhow::Error> {
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

    let account = world
        .account_public_keys
        .get(&account_name)
        .unwrap_or_else(|| panic!("Account {} not found", account_name));

    let account_address = world.get_account_component_address(&account_name).unwrap();
    let component_address = ComponentAddress::from_str(&account_address).expect("Invalid account address");

    let instructions = [
        Instruction::ClaimBurn {
            claim: ConfidentialClaim {
                commitment_address: commitment.to_vec().try_into()?,
                range_proof: rangeproof.clone(),
                proof_of_knowledge: proof.clone().try_into()?,
            },
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: b"burn".to_vec() },
        Instruction::CallMethod {
            component_address,
            method: "deposit".to_string(),
            args: vec![Arg::Variable(b"burn".to_vec())],
        },
    ];

    let account_shard = ShardId::from_address(&SubstateAddress::from_str(&account_address).unwrap(), 0);
    let account_v1_shard = ShardId::from_address(&SubstateAddress::from_str(&account_address).unwrap(), 1);

    let mut builder = Transaction::builder();
    builder
        .with_instructions(instructions.to_vec())
        .with_outputs(vec![account_v1_shard])
        .with_inputs(vec![commitment_shard, account_shard])
        .with_fee(1)
        .with_new_outputs(1)
        .sign(&account.0);
    let transaction = builder.build();

    let request = SubmitTransactionRequest {
        transaction,
        wait_for_result: true,
        wait_for_result_timeout: None,
        is_dry_run: false,
    };

    let mut client = vn.create_client().await;
    client.submit_transaction(request).await?;

    Ok(())
}

#[when(
    expr = "I claim burn {word} with {word} and {word} and spend it into account {word} on {word} a second time, it \
            fails"
)]
async fn when_i_claim_burn_second_time_fails(
    world: &mut TariWorld,
    commitment_name: String,
    proof_name: String,
    rangeproof_name: String,
    account_name: String,
    vn_name: String,
) {
    assert!(when_i_claim_burn(
        world,
        commitment_name,
        proof_name,
        rangeproof_name,
        account_name,
        vn_name,
    )
    .await
    .is_err());
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
