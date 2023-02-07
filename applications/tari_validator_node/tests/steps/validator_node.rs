//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use cucumber::then;
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_common_types::ShardId;
use tari_engine_types::instruction::Instruction;
use tari_engine_types::substate::SubstateAddress;
use tari_validator_node_client::types::GetStateRequest;

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

#[when(expr = "I claim burn {word} with {word} and {word} and spend it into account {word} on {word}") ]
async fn when_i_claim_burn(world: &mut TariWorld, commitment_name: String,  proof_name: String, rangeproof_name: String, account_name: String, vn_name: String) {
    let commitment = world
        .commitments
        .get(&commitment_name)
        .unwrap_or_else(|| panic!("Commitment {} not found", commitment_name));
    let proof = world
        .proofs
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
        &SubstateAddress::from_str("commitment_0000000000000000000000000000000000000000000000000000000000000000").expect("Invalid state address"),
        0,
    );

    // let account = world
    //     .outputs
    //     .get(&account_name)
    //     .unwrap_or_else(|| panic!("Account {} not found", account_name));


    let instructions = [Instruction::ClaimBurn {
        commitment_address: shard_id,
        range_proof: rangeproof.clone(),
        proof_of_knowledge: proof.clone(),
    },
    Instruction::PutLastInstructionOutputOnWorkspace {
        key: b"burn".to_vec(),
    },
    Instruction::CallMethod {
        component_address: account.,
        method: "".to_string(),
        args: vec![],
    }]

    let mut client = vn.create_client().await;
    client.submit_transaction()
}
