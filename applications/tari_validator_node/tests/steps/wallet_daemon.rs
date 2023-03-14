//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{convert::TryInto, str::FromStr};

use cucumber::when;
use tari_crypto::tari_utilities::hex::Hex;
use tari_dan_common_types::ShardId;
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::prelude::ComponentAddress;

use crate::{utils::wallet_daemon_cli, TariWorld};

#[when(
    expr = "I claim burn {word} on wallet daemon {word} with {word} and {word} and spend it into account {word} on \
            {word}"
)]
async fn when_i_claim_burn_via_wallet_daemon(
    world: &mut TariWorld,
    commitment_name: String,
    wallet_daemon_name: String,
    proof_name: String,
    rangeproof_name: String,
    account_name: String,
    _vn_name: String,
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

    let account_address = world.get_account_component_address(&account_name).unwrap();
    let component_address = ComponentAddress::from_str(&account_address).expect("Invalid account address");

    let claim_burn_resp = wallet_daemon_cli::claim_burn(
        world,
        component_address,
        commitment.clone(),
        rangeproof.clone(),
        proof.clone(),
        wallet_daemon_name,
    )
    .await;

    assert!(claim_burn_resp.result.is_accept());
}
