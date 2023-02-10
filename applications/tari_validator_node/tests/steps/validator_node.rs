//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use cucumber::{then, when};
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::{hex::Hex, ByteArray},
};
use tari_dan_common_types::ShardId;
use tari_engine_types::{instruction::Instruction, signature::InstructionSignature, substate::SubstateAddress};
use tari_transaction::SubstateChange;
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
    let vn = world
        .validator_nodes
        .get(&vn_name)
        .unwrap_or_else(|| panic!("Validator node {} not found", vn_name));
    let shard_id = ShardId::from_address(
        &SubstateAddress::from_str(&format!("commitment_{}", commitment.to_hex())).expect("Invalid state address"),
        0,
    );

    let account = world
        .account_public_keys
        .get(&account_name)
        .unwrap_or_else(|| panic!("Account {} not found", account_name));

    let instructions = [
        Instruction::ClaimBurn {
            commitment_address: commitment.to_vec(),
            range_proof: rangeproof.clone(),
            proof_of_knowledge: proof.clone(),
        },
        Instruction::PutLastInstructionOutputOnWorkspace { key: b"burn".to_vec() },
        // Instruction::CallMethod {
        //     component_address: account.,
        //     method: "".to_string(),
        //     args: vec![],
        // }
    ];

    let signature = InstructionSignature::sign(&account.0, &instructions);
    let request = SubmitTransactionRequest {
        instructions: instructions.to_vec(),
        signature,
        fee: 0,
        sender_public_key: account.1.clone(),
        inputs: vec![(shard_id, SubstateChange::Destroy)],
        num_outputs: 0,
        wait_for_result: true,
        wait_for_result_timeout: None,
        is_dry_run: false,
    };

    let mut client = vn.create_client().await;

    client.submit_transaction(request).await.unwrap();
}

#[then(expr = "{word} is on epoch {int} within {int} seconds")]
async fn vn_has_scanned_to_height(world: &mut TariWorld, vn_name: String, epoch: usize, seconds: usize) {
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
