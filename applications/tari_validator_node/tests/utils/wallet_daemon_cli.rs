//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{
    collections::HashMap,
    convert::{TryFrom, TryInto},
    str::FromStr,
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::json;
use tari_crypto::{
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    signatures::CommitmentSignature,
    tari_utilities::ByteArray,
};
use tari_engine_types::{
    instruction::Instruction,
    substate::{SubstateAddress, SubstateDiff},
};
use tari_template_lib::{args, constants::CONFIDENTIAL_TARI_RESOURCE_ADDRESS, models::Amount, prelude::NonFungibleId};
use tari_transaction_manifest::{parse_manifest, ManifestValue};
use tari_validator_node_cli::{command::transaction::CliArg, versioned_substate_address::VersionedSubstateAddress};
use tari_wallet_daemon_client::{
    types::{
        AccountByNameResponse,
        AccountsCreateRequest,
        AccountsGetBalancesRequest,
        ClaimBurnRequest,
        ClaimBurnResponse,
        ProofsGenerateRequest,
        TransactionGetResultRequest,
        TransactionSubmitRequest,
        TransactionWaitResultRequest,
    },
    WalletDaemonClient,
};

use super::{validator_node_cli::get_key_manager, wallet_daemon::get_walletd_client};
use crate::TariWorld;

pub async fn claim_burn(
    world: &mut TariWorld,
    account_name: String,
    commitment: Vec<u8>,
    range_proof: Vec<u8>,
    ownership_proof: CommitmentSignature<RistrettoPublicKey, RistrettoSecretKey>,
    reciprocal_claim_public_key: RistrettoPublicKey,
    wallet_daemon_name: String,
) -> ClaimBurnResponse {
    let mut client = get_wallet_daemon_client(world, wallet_daemon_name).await;

    let account = client.accounts_get_by_name(account_name.as_str()).await.unwrap();
    let account_address = account.account.address.as_component_address().unwrap();

    let claim_burn_request = ClaimBurnRequest {
        account: account_address,
        claim_proof: json!({
            "commitment": BASE64.encode(commitment.as_bytes()),
            "ownership_proof": {
                "public_nonce": BASE64.encode(ownership_proof.public_nonce().as_bytes()),
                "u": BASE64.encode(ownership_proof.u().as_bytes()),
                "v": BASE64.encode(ownership_proof.v().as_bytes())
            },
            "reciprocal_claim_public_key": BASE64.encode(reciprocal_claim_public_key.as_bytes()),
            "range_proof": BASE64.encode(range_proof.as_bytes()),
        }),
        fee: Amount(1),
    };

    client.claim_burn(claim_burn_request).await.unwrap()
}

pub async fn create_transfer_proof(
    world: &mut TariWorld,
    source_account_name: String,
    dest_account_name: String,
    amount: u64,
    wallet_daemon_name: String,
    outputs_name: String,
) -> tari_wallet_daemon_client::types::TransactionSubmitResponse {
    let mut client = get_wallet_daemon_client(world, wallet_daemon_name).await;

    let AccountByNameResponse { account, .. } = client.accounts_get_by_name(&source_account_name).await.unwrap();
    let source_component_address = account
        .address
        .as_component_address()
        .expect("Invalid component address for source address");

    let destination_account_resp = client
        .accounts_get_by_name(&dest_account_name)
        .await
        .expect("Failed to retrieve destination account address from its name");

    let destination_account = destination_account_resp
        .account
        .address
        .as_component_address()
        .expect("Failed to get component address from destination account");
    let destination_public_key = destination_account_resp.public_key;

    let resource_address = CONFIDENTIAL_TARI_RESOURCE_ADDRESS;

    let create_transfer_proof_req = ProofsGenerateRequest {
        amount: Amount(amount.try_into().unwrap()),
        reveal_amount: Amount(0),
        source_account_name: source_account_name.clone(),
        resource_address,
        destination_public_key,
    };
    let transfer_proof_resp = client.create_transfer_proof(create_transfer_proof_req).await.unwrap();
    let withdraw_proof = transfer_proof_resp.proof;
    let proof_id = transfer_proof_resp.proof_id;

    let instructions = vec![
        Instruction::CallMethod {
            component_address: source_component_address,
            method: String::from("withdraw_confidential"),
            args: args![resource_address, withdraw_proof],
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"bucket".to_vec(),
        },
        Instruction::CallMethod {
            component_address: destination_account,
            method: String::from("deposit"),
            args: args![Variable("bucket")],
        },
    ];

    let submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        fee_instructions: vec![Instruction::CallMethod {
            component_address: source_component_address,
            method: "pay_fee".to_string(),
            args: args![Amount::try_from(0).unwrap()],
        }],
        proof_ids: vec![proof_id],
        new_non_fungible_index_outputs: vec![],
        new_non_fungible_outputs: vec![],
        specific_non_fungible_outputs: vec![],
        is_dry_run: false,
        override_inputs: true,
        instructions,
        inputs: vec![],
        new_outputs: 1,
    };

    let submit_resp = client.submit_transaction(submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        hash: submit_resp.hash,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    add_substate_addresses_from_wallet_daemon(
        world,
        outputs_name,
        &wait_resp
            .result
            .expect("Transaction has timed out")
            .result
            .expect("Transaction has failed"),
    );

    submit_resp
}

pub async fn create_account(world: &mut TariWorld, account_name: String, wallet_daemon_name: String) {
    let key = get_key_manager(world).get_active_key().expect("No active keypair");
    world
        .account_public_keys
        .insert(account_name.clone(), (key.secret_key.clone(), key.public_key.clone()));

    let request = AccountsCreateRequest {
        account_name: Some(account_name.clone()),
        signing_key_index: None,
        custom_access_rules: None,
        fee: None,
    };

    let mut client = get_wallet_daemon_client(world, wallet_daemon_name.clone()).await;
    let resp = client.create_account(request).await.unwrap();

    add_substate_addresses_from_wallet_daemon(
        world,
        account_name,
        &resp.result.result.expect("Failed to obtain substate diffs"),
    );
}

pub async fn get_balance(world: &mut TariWorld, account_name: String, wallet_daemon_name: String) -> i64 {
    let get_balance_req = AccountsGetBalancesRequest { account_name };
    let mut client = get_wallet_daemon_client(world, wallet_daemon_name).await;
    let resp = client
        .get_account_balances(get_balance_req)
        .await
        .expect("Failed to get balance from account");
    let balances = resp.balances;
    balances.iter().map(|e| e.balance.value()).sum()
}

pub async fn submit_manifest(
    world: &mut TariWorld,
    wallet_daemon_name: String,
    manifest_content: String,
    inputs: String,
    num_outputs: u64,
    outputs_name: String,
) {
    let input_groups = inputs.split(',').map(|s| s.trim()).collect::<Vec<_>>();

    // generate globals for component addresses
    let globals: HashMap<String, ManifestValue> = world
        .wallet_daemon_outputs
        .iter()
        .filter(|(name, _)| input_groups.contains(&name.as_str()))
        .flat_map(|(name, outputs)| {
            outputs
                .iter()
                .map(move |(child_name, addr)| (format!("{}/{}", name, child_name), addr.address.clone().into()))
        })
        .collect();

    // parse the minting specific outputs (if any) specified in the manifest as comments
    let new_non_fungible_outputs = manifest_content
        .lines()
        .filter(|l| l.starts_with("// $mint "))
        .map(|l| l.split_whitespace().skip(2).collect::<Vec<&str>>())
        .map(|l| {
            let manifest_value = globals.get(l[0]).unwrap();
            let resource_address = manifest_value.as_address().unwrap().as_resource_address().unwrap();
            let count = l[1].parse::<u8>().unwrap();
            (resource_address, count)
        })
        .collect::<Vec<_>>();

    let new_non_fungible_index_outputs = manifest_content
        .lines()
        .filter(|l| l.starts_with("// $nft_index "))
        .map(|l| l.split_whitespace().skip(2).collect::<Vec<&str>>())
        .map(|l| {
            let manifest_value = globals.get(l[0]).unwrap();
            let parent_address = manifest_value.as_address().unwrap().as_resource_address().unwrap();
            let index = u64::from_str(l[1]).unwrap();
            (parent_address, index)
        })
        .collect::<Vec<_>>();

    let specific_non_fungible_outputs = manifest_content
        .lines()
        .filter(|l| l.starts_with("// $mint_specific "))
        .map(|l| l.split_whitespace().skip(2).collect::<Vec<&str>>())
        .map(|l| {
            let manifest_value = globals.get(l[0]).unwrap();
            let resource_address = manifest_value.as_address().unwrap().as_resource_address().unwrap();
            let non_fungible_id = NonFungibleId::try_from_canonical_string(l[1]).unwrap();
            (resource_address, non_fungible_id)
        })
        .collect::<Vec<_>>();

    // Supply the inputs explicitly. If this is empty, the internal component manager
    // will attempt to supply the correct inputs
    let inputs = inputs
        .split(',')
        .flat_map(|s| {
            world
                .wallet_daemon_outputs
                .get(s.trim())
                .unwrap_or_else(|| panic!("No outputs named {}", s.trim()))
        })
        .map(|(_, addr)| tari_dan_wallet_sdk::models::VersionedSubstateAddress {
            address: addr.address.clone(),
            version: addr.version,
        })
        .collect::<Vec<_>>();

    let instructions = parse_manifest(&manifest_content, globals).unwrap();
    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions,
        fee_instructions: vec![],
        override_inputs: false,
        is_dry_run: false,
        proof_ids: vec![],
        new_outputs: num_outputs as u8,
        specific_non_fungible_outputs,
        inputs,
        new_non_fungible_outputs,
        new_non_fungible_index_outputs,
    };

    let mut client = get_wallet_daemon_client(world, wallet_daemon_name.clone()).await;
    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        hash: resp.hash,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    add_substate_addresses_from_wallet_daemon(
        world,
        outputs_name,
        &wait_resp
            .result
            .expect("Transaction has timed out")
            .result
            .expect("Transaction has failed"),
    );
}

pub async fn create_component(
    world: &mut TariWorld,
    outputs_name: String,
    template_name: String,
    account_name: String,
    wallet_daemon_name: String,
    function_call: String,
    args: Vec<String>,
    num_outputs: u64,
) {
    let template_address = world
        .templates
        .get(&template_name)
        .unwrap_or_else(|| panic!("Template not found with name {}", template_name))
        .address;
    let args = args.iter().map(|a| CliArg::from_str(a).unwrap().into_arg()).collect();
    let instruction = Instruction::CallFunction {
        template_address,
        function: function_call,
        args,
    };

    let mut client = get_wallet_daemon_client(world, wallet_daemon_name.clone()).await;
    let source_component_address = client
        .accounts_get_by_name(account_name.as_str())
        .await
        .unwrap()
        .account
        .address
        .as_component_address()
        .unwrap();

    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions: vec![instruction],
        fee_instructions: vec![],
        override_inputs: false,
        is_dry_run: false,
        proof_ids: vec![],
        new_outputs: num_outputs as u8,
        specific_non_fungible_outputs: vec![],
        inputs: vec![],
        new_non_fungible_outputs: vec![],
        new_non_fungible_index_outputs: vec![],
    };

    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        hash: resp.hash,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    add_substate_addresses_from_wallet_daemon(
        world,
        outputs_name,
        &wait_resp
            .result
            .unwrap()
            .result
            .expect("Failed to obtain substate diffs"),
    );
}

pub(crate) async fn get_wallet_daemon_client(world: &TariWorld, wallet_daemon_name: String) -> WalletDaemonClient {
    let port = world.wallet_daemons.get(&wallet_daemon_name).unwrap().json_rpc_port;
    get_walletd_client(port).await
}

fn add_substate_addresses_from_wallet_daemon(world: &mut TariWorld, outputs_name: String, diff: &SubstateDiff) {
    let outputs = world.wallet_daemon_outputs.entry(outputs_name).or_default();
    let mut counters = [0usize, 0, 0, 0, 0, 0, 0];
    for (addr, data) in diff.up_iter() {
        match addr {
            SubstateAddress::Component(_) => {
                let component = data.substate_value().component().unwrap();
                outputs.insert(
                    format!("components/{}", component.module_name),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[0] += 1;
            },
            SubstateAddress::Resource(_) => {
                outputs.insert(format!("resources/{}", counters[1]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[1] += 1;
            },
            SubstateAddress::Vault(_) => {
                outputs.insert(format!("vaults/{}", counters[2]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[2] += 1;
            },
            SubstateAddress::NonFungible(_) => {
                outputs.insert(format!("nfts/{}", counters[3]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[3] += 1;
            },
            SubstateAddress::UnclaimedConfidentialOutput(_) => {
                outputs.insert(
                    format!("layer_one_commitments/{}", counters[4]),
                    VersionedSubstateAddress {
                        address: addr.clone(),
                        version: data.version(),
                    },
                );
                counters[4] += 1;
            },
            SubstateAddress::NonFungibleIndex(_) => {
                outputs.insert(format!("nft_indexes/{}", counters[5]), VersionedSubstateAddress {
                    address: addr.clone(),
                    version: data.version(),
                });
                counters[5] += 1;
            },
        }
    }
}
