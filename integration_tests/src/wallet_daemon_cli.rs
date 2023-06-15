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

use std::{collections::HashMap, str::FromStr};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::json;
use tari_common_types::types::FixedHash;
use tari_crypto::{
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    signatures::CommitmentSignature,
    tari_utilities::{hex::Hex, ByteArray},
};
use tari_dan_wallet_sdk::apis::jwt::{JrpcPermission, JrpcPermissions};
use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    constants::CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
    crypto::RistrettoPublicKeyBytes,
    models::Amount,
    prelude::{NonFungibleAddress, NonFungibleId, ResourceAddress},
};
use tari_transaction::SubstateRequirement;
use tari_transaction_manifest::{parse_manifest, ManifestValue};
use tari_validator_node_cli::command::transaction::CliArg;
use tari_wallet_daemon_client::{
    types::{
        AccountGetResponse, AccountsCreateFreeTestCoinsRequest, AccountsCreateRequest, AccountsGetBalancesRequest,
        AuthLoginAcceptRequest, AuthLoginRequest, AuthLoginResponse, ClaimBurnRequest, ClaimBurnResponse,
        ConfidentialTransferRequest, MintAccountNFTRequest, ProofsGenerateRequest, RevealFundsRequest,
        TransactionSubmitRequest, TransactionWaitResultRequest, TransferRequest,
    },
    ComponentAddressOrName, WalletDaemonClient,
};

use super::wallet_daemon::get_walletd_client;
use crate::{validator_node_cli::add_substate_addresses, TariWorld};

pub async fn claim_burn(
    world: &mut TariWorld,
    account_name: String,
    commitment: Vec<u8>,
    range_proof: Vec<u8>,
    ownership_proof: CommitmentSignature<RistrettoPublicKey, RistrettoSecretKey>,
    reciprocal_claim_public_key: RistrettoPublicKey,
    wallet_daemon_name: String,
) -> ClaimBurnResponse {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let claim_burn_request = ClaimBurnRequest {
        account: Some(ComponentAddressOrName::Name(account_name.clone())),
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
        fee: Some(Amount(1)),
    };

    let claim_burn_resp = match client.claim_burn(claim_burn_request).await {
        Ok(resp) => resp,
        Err(err) => {
            println!("Failed to claim burn: {}", err);
            panic!("Failed to claim burn: {}", err);
        },
    };

    let wait_req = TransactionWaitResultRequest {
        hash: claim_burn_resp.hash,
        timeout_secs: Some(300),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    if let Some(reason) = wait_resp.transaction_failure {
        panic!("Transaction failed: {}", reason);
    }
    add_substate_addresses(
        world,
        format!("claim_burn/{}/{}", account_name, commitment.to_hex()),
        &wait_resp
            .result
            .expect("Transaction has timed out")
            .result
            .expect("Transaction has failed"),
    );

    claim_burn_resp
}

pub async fn reveal_burned_funds(world: &mut TariWorld, account_name: String, amount: u64, wallet_daemon_name: String) {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let request = RevealFundsRequest {
        account: Some(ComponentAddressOrName::Name(account_name)),
        amount_to_reveal: Amount(amount as i64),
        fee: Some(Amount(1)),
        pay_fee_from_reveal: true,
    };

    let resp = client
        .accounts_reveal_funds(request)
        .await
        .expect("Failed to request reveal funds");

    let wait_req = TransactionWaitResultRequest {
        hash: resp.hash,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();
    assert!(wait_resp.result.unwrap().result.is_accept());
}

#[allow(clippy::too_many_arguments)]
pub async fn transfer_confidential(
    world: &mut TariWorld,
    source_account_name: String,
    dest_account_name: String,
    amount: u64,
    wallet_daemon_name: String,
    outputs_name: String,
) -> tari_wallet_daemon_client::types::TransactionSubmitResponse {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let source_account_addr = world
        .get_account_component_address(&source_account_name)
        .map(|addr| SubstateRequirement::new(addr.address.clone(), Some(addr.version)))
        .unwrap_or_else(|| panic!("Source account {} not found", source_account_name));
    let dest_account_addr = world
        .get_account_component_address(&dest_account_name)
        .map(|addr| SubstateRequirement::new(addr.address.clone(), Some(addr.version)))
        .unwrap_or_else(|| panic!("Destination account {} not found", dest_account_name));

    let source_account_name = ComponentAddressOrName::Name(source_account_name);
    let AccountGetResponse { account, .. } = client.accounts_get(source_account_name.clone()).await.unwrap();
    let source_component_address = account
        .address
        .as_component_address()
        .expect("Invalid component address for source address");

    let signing_key_index = account.key_index;

    let dest_account_name = ComponentAddressOrName::Name(dest_account_name);
    let destination_account_resp = client
        .accounts_get(dest_account_name)
        .await
        .expect("Failed to retrieve destination account address from its name");

    let destination_account = destination_account_resp
        .account
        .address
        .as_component_address()
        .expect("Failed to get component address from destination account");
    let destination_public_key = destination_account_resp.public_key;

    let resource_address = *CONFIDENTIAL_TARI_RESOURCE_ADDRESS;

    let create_transfer_proof_req = ProofsGenerateRequest {
        account: Some(source_account_name),
        amount: Amount(amount as i64),
        reveal_amount: Amount(0),
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
        signing_key_index: Some(signing_key_index),
        fee_instructions: vec![Instruction::CallMethod {
            component_address: source_component_address,
            method: "pay_fee".to_string(),
            args: args![Amount(1)],
        }],
        proof_ids: vec![proof_id],
        new_resources: vec![],
        new_non_fungible_index_outputs: vec![],
        new_non_fungible_outputs: vec![],
        specific_non_fungible_outputs: vec![],
        is_dry_run: false,
        override_inputs: false,
        instructions,
        inputs: vec![source_account_addr, dest_account_addr],
        new_outputs: 1,
    };

    let submit_resp = client.submit_transaction(submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        hash: submit_resp.hash,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    add_substate_addresses(
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
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let request = AccountsCreateRequest {
        account_name: Some(account_name.clone()),
        custom_access_rules: None,
        is_default: true,
        fee: None,
    };

    let resp = client.create_account(request).await.unwrap();

    // TODO: store the secret key in the world, but we don't have a need for it at the moment
    world.account_keys.insert(
        account_name.clone(),
        (RistrettoSecretKey::default(), resp.public_key.clone()),
    );

    let wait_req = TransactionWaitResultRequest {
        hash: FixedHash::from(resp.result.transaction_hash.into_array()),
        timeout_secs: Some(120),
    };
    let _wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    add_substate_addresses(
        world,
        account_name,
        &resp.result.result.expect("Failed to obtain substate diffs"),
    );
}

pub async fn create_account_with_free_coins(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
    amount: Amount,
) {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let request = AccountsCreateFreeTestCoinsRequest {
        account: Some(ComponentAddressOrName::Name(account_name.clone())),
        amount,
        fee: None,
    };

    let resp = client.create_free_test_coins(request).await.unwrap();
    // TODO: store the secret key in the world, but we don't have a need for it at the moment
    world.account_keys.insert(
        account_name.clone(),
        (RistrettoSecretKey::default(), resp.public_key.clone()),
    );
    let wait_req = TransactionWaitResultRequest {
        hash: FixedHash::from(resp.result.transaction_hash.into_array()),
        timeout_secs: Some(120),
    };
    let _wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    add_substate_addresses(
        world,
        account_name,
        &resp.result.result.expect("Failed to obtain substate diffs"),
    );
}

pub async fn mint_new_nft_on_account(
    world: &mut TariWorld,
    _nft_name: String,
    account_name: String,
    wallet_daemon_name: String,
    metadata: Option<serde_json::Value>,
) {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;
    let account_keys = world
        .account_keys
        .get(&account_name)
        .expect("Failed to get account key pair");
    let owner_token = NonFungibleAddress::from_public_key(
        RistrettoPublicKeyBytes::from_bytes(account_keys.1.as_bytes()).expect("Failed to parse public key"),
    );
    let token_symbol = "MY_NFT".to_string();
    let metadata = metadata.unwrap_or_else(|| {
        serde_json::json!({
            "name": "TariProject",
            "departure": "Now",
            "landing_on": "Moon"
        })
    });

    let request = MintAccountNFTRequest {
        account: ComponentAddressOrName::Name(account_name.clone()),
        metadata,
        token_symbol,
        owner_token,
        mint_fee: Some(Amount::new(1_000)),
        create_account_nft_fee: None,
    };
    let resp = client
        .mint_account_nft(request)
        .await
        .expect("Failed to mint new account NFT");

    let wait_req = TransactionWaitResultRequest {
        hash: FixedHash::from(resp.result.transaction_hash.into_array()),
        timeout_secs: Some(120),
    };
    let _wait_resp = client
        .wait_transaction_result(wait_req)
        .await
        .expect("Wait response failed");

    add_substate_addresses(
        world,
        account_name,
        &resp.result.result.expect("Failed to obtain substate diffs"),
    );
}

pub async fn get_balance(world: &mut TariWorld, account_name: &str, wallet_daemon_name: &str) -> i64 {
    let account_name = ComponentAddressOrName::Name(account_name.to_string());
    let get_balance_req = AccountsGetBalancesRequest {
        account: Some(account_name),
        refresh: true,
    };
    let mut client = get_auth_wallet_daemon_client(world, wallet_daemon_name).await;

    let resp = client
        .get_account_balances(get_balance_req)
        .await
        .expect("Failed to get balance from account");
    eprintln!("resp = {}", serde_json::to_string_pretty(&resp).unwrap());
    resp.balances.iter().map(|e| e.balance.value()).sum()
}

pub async fn get_confidential_balance(
    world: &mut TariWorld,
    account_name: String,
    wallet_daemon_name: String,
) -> Amount {
    let account_name = ComponentAddressOrName::Name(account_name);
    let get_balance_req = AccountsGetBalancesRequest {
        account: Some(account_name),
        refresh: true,
    };
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let resp = client
        .get_account_balances(get_balance_req)
        .await
        .expect("Failed to get balance from account");
    eprintln!("resp = {}", serde_json::to_string_pretty(&resp).unwrap());
    resp.balances.iter().map(|e| e.confidential_balance).sum()
}

pub async fn submit_manifest_with_signing_keys(
    world: &mut TariWorld,
    wallet_daemon_name: String,
    account_signing_key: String,
    manifest_content: String,
    inputs: String,
    num_outputs: u64,
    outputs_name: String,
) {
    let input_groups = inputs.split(',').map(|s| s.trim()).collect::<Vec<_>>();

    // generate globals for component addresses
    let globals: HashMap<String, ManifestValue> = world
        .outputs
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
                .outputs
                .get(s.trim())
                .unwrap_or_else(|| panic!("No outputs named {}", s.trim()))
        })
        .map(|(_, addr)| SubstateRequirement::new(addr.address.clone(), Some(addr.version)))
        .collect::<Vec<_>>();

    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let account_name = ComponentAddressOrName::Name(account_signing_key);
    let AccountGetResponse { account, .. } = client.accounts_get(account_name).await.unwrap();

    let instructions = parse_manifest(&manifest_content, globals).unwrap();
    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: Some(account.key_index),
        instructions,
        fee_instructions: vec![],
        override_inputs: false,
        is_dry_run: false,
        proof_ids: vec![],
        new_resources: vec![],
        new_outputs: num_outputs as u8,
        specific_non_fungible_outputs,
        inputs,
        new_non_fungible_outputs,
        new_non_fungible_index_outputs,
    };

    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        hash: resp.hash,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();
    if let Some(reason) = wait_resp.transaction_failure {
        panic!("Transaction failed: {}", reason);
    }

    add_substate_addresses(
        world,
        outputs_name,
        &wait_resp
            .result
            .expect("Transaction has timed out")
            .result
            .expect("Transaction has failed"),
    );
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
        .outputs
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
                .outputs
                .get(s.trim())
                .unwrap_or_else(|| panic!("No outputs named {}", s.trim()))
        })
        .map(|(_, addr)| SubstateRequirement::new(addr.address.clone(), Some(addr.version)))
        .collect::<Vec<_>>();

    let instructions = parse_manifest(&manifest_content, globals).unwrap();

    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions,
        fee_instructions: vec![],
        override_inputs: false,
        is_dry_run: false,
        proof_ids: vec![],
        new_resources: vec![],
        new_outputs: num_outputs as u8,
        specific_non_fungible_outputs,
        inputs,
        new_non_fungible_outputs,
        new_non_fungible_index_outputs,
    };

    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;
    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        hash: resp.hash,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    if let Some(reason) = wait_resp.transaction_failure {
        panic!("Transaction failed: {}", reason);
    }
    add_substate_addresses(
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
    _account_name: String,
    wallet_daemon_name: String,
    function_call: String,
    args: Vec<String>,
    _num_outputs: u64,
) {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

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

    // // Supply the inputs explicitly. If this is empty, the internal component manager
    // // will attempt to supply the correct inputs
    // let inputs = world
    //     .wallet_daemon_outputs
    //     .get(&account_name)
    //     .unwrap_or_else(|| panic!("No account_name {}", account_name))
    //     .iter()
    //         address: addr.address.clone(),

    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions: vec![instruction],
        fee_instructions: vec![],
        override_inputs: false,
        is_dry_run: false,
        proof_ids: vec![],
        new_resources: vec![],
        specific_non_fungible_outputs: vec![],
        inputs: vec![],
        new_outputs: 0,
        new_non_fungible_outputs: vec![],
        new_non_fungible_index_outputs: vec![],
    };

    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        hash: resp.hash,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    if let Some(reason) = wait_resp.transaction_failure {
        panic!("Transaction failed: {}", reason);
    }
    add_substate_addresses(
        world,
        outputs_name,
        &wait_resp
            .result
            .unwrap()
            .result
            .expect("Failed to obtain substate diffs"),
    );
}

pub async fn transfer(
    world: &mut TariWorld,
    account_name: String,
    destination_public_key: RistrettoPublicKey,
    resource_address: ResourceAddress,
    amount: Amount,
    wallet_daemon_name: String,
    outputs_name: String,
) {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let account = Some(ComponentAddressOrName::Name(account_name));
    let fee = Some(Amount(1));

    let request = TransferRequest {
        account,
        amount,
        resource_address,
        destination_public_key,
        fee,
    };

    let resp = client.accounts_transfer(request).await.unwrap();
    add_substate_addresses(world, outputs_name, resp.result.result.accept().unwrap());
}

pub async fn confidential_transfer(
    world: &mut TariWorld,
    account_name: String,
    destination_public_key: RistrettoPublicKey,
    amount: Amount,
    wallet_daemon_name: String,
    outputs_name: String,
) {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let account = Some(ComponentAddressOrName::Name(account_name));
    let fee = Some(Amount(1));

    let request = ConfidentialTransferRequest {
        account,
        amount,
        destination_public_key,
        fee,
        resource_address: *CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
    };

    let resp = client.accounts_confidential_transfer(request).await.unwrap();
    add_substate_addresses(world, outputs_name, resp.result.result.accept().unwrap());
}

pub(crate) async fn get_wallet_daemon_client(world: &TariWorld, wallet_daemon_name: &str) -> WalletDaemonClient {
    let port = world.wallet_daemons.get(wallet_daemon_name).unwrap().json_rpc_port;
    get_walletd_client(port).await
}

pub async fn get_auth_wallet_daemon_client(world: &TariWorld, wallet_daemon_name: &str) -> WalletDaemonClient {
    let mut client = get_wallet_daemon_client(world, wallet_daemon_name).await;
    // authentication
    let AuthLoginResponse { auth_token } = client
        .auth_request(AuthLoginRequest {
            permissions: JrpcPermissions(vec![JrpcPermission::Admin]),
            duration: None,
        })
        .await
        .unwrap();
    let auth_response = client
        .auth_accept(AuthLoginAcceptRequest {
            auth_token,
            name: "Testing Token".to_string(),
        })
        .await
        .unwrap();
    client.set_auth_token(auth_response.permissions_token);
    client
}
