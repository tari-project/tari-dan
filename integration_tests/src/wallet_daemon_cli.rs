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

use std::{collections::HashMap, str::FromStr, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::json;
use tari_crypto::{
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    signatures::CommitmentSignature,
    tari_utilities::ByteArray,
};
use tari_dan_common_types::Epoch;
use tari_engine_types::instruction::Instruction;
use tari_template_lib::{
    args,
    constants::CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
    models::Amount,
    prelude::ResourceAddress,
    resource::TOKEN_SYMBOL,
};
use tari_transaction::SubstateRequirement;
use tari_transaction_manifest::{parse_manifest, ManifestValue};
use tari_validator_node_cli::command::transaction::CliArg;
use tari_wallet_daemon_client::{
    error::WalletDaemonClientError,
    types::{
        AccountGetResponse,
        AccountsCreateFreeTestCoinsRequest,
        AccountsCreateRequest,
        AccountsGetBalancesRequest,
        ClaimBurnRequest,
        ClaimBurnResponse,
        ClaimValidatorFeesRequest,
        ClaimValidatorFeesResponse,
        ConfidentialTransferRequest,
        MintAccountNftRequest,
        ProofsGenerateRequest,
        RevealFundsRequest,
        TransactionSubmitRequest,
        TransactionWaitResultRequest,
        TransactionWaitResultResponse,
        TransferRequest,
    },
    ComponentAddressOrName,
    WalletDaemonClient,
};
use tokio::time::timeout;

use crate::{validator_node_cli::add_substate_ids, TariWorld};

pub async fn claim_burn(
    world: &mut TariWorld,
    account_name: String,
    commitment: Vec<u8>,
    range_proof: Vec<u8>,
    ownership_proof: CommitmentSignature<RistrettoPublicKey, RistrettoSecretKey>,
    reciprocal_claim_public_key: RistrettoPublicKey,
    wallet_daemon_name: String,
    max_fee: i64,
) -> Result<ClaimBurnResponse, WalletDaemonClientError> {
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
        max_fee: Some(Amount(max_fee)),
        key_id: None,
    };

    client.claim_burn(claim_burn_request).await
}

pub async fn claim_fees(
    world: &mut TariWorld,
    wallet_daemon_name: String,
    account_name: String,
    validator_name: String,
    epoch: u64,
    dry_run: bool,
) -> Result<ClaimValidatorFeesResponse, WalletDaemonClientError> {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let vn = world.get_validator_node(&validator_name);

    let request = ClaimValidatorFeesRequest {
        account: Some(ComponentAddressOrName::Name(account_name)),
        max_fee: None,
        validator_public_key: vn.public_key.clone(),
        epoch: Epoch(epoch),
        dry_run,
    };

    client.claim_validator_fees(request).await
}

pub async fn reveal_burned_funds(world: &mut TariWorld, account_name: String, amount: u64, wallet_daemon_name: String) {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let request = RevealFundsRequest {
        account: Some(ComponentAddressOrName::Name(account_name)),
        amount_to_reveal: Amount(amount as i64),
        max_fee: Some(Amount(1)),
        pay_fee_from_reveal: true,
    };

    let resp = client
        .accounts_reveal_funds(request)
        .await
        .expect("Failed to request reveal funds");

    let wait_req = TransactionWaitResultRequest {
        transaction_id: resp.transaction_id,
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
    min_epoch: Option<Epoch>,
    max_epoch: Option<Epoch>,
) -> tari_wallet_daemon_client::types::TransactionSubmitResponse {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let source_account_addr = world
        .get_account_component_address(&source_account_name)
        .unwrap_or_else(|| panic!("Source account {} not found", source_account_name));
    let dest_account_addr = world
        .get_account_component_address(&dest_account_name)
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

    let resource_address = CONFIDENTIAL_TARI_RESOURCE_ADDRESS;

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
        is_dry_run: false,
        override_inputs: false,
        instructions,
        inputs: vec![source_account_addr, dest_account_addr],
        min_epoch,
        max_epoch,
    };

    let submit_resp = client.submit_transaction(submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        transaction_id: submit_resp.transaction_id,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    add_substate_ids(
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
        is_default: false,
        max_fee: None,
        key_id: None,
    };

    let resp = timeout(Duration::from_secs(240), client.create_account(request))
        .await
        .unwrap()
        .unwrap();

    // TODO: store the secret key in the world, but we don't have a need for it at the moment
    world.account_keys.insert(
        account_name.clone(),
        (RistrettoSecretKey::default(), resp.public_key.clone()),
    );

    add_substate_ids(
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
    key_name: Option<String>,
) {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let key_index = key_name.map(|k| {
        *world
            .wallet_keys
            .get(&k)
            .unwrap_or_else(|| panic!("Wallet {} not found", wallet_daemon_name))
    });
    let request = AccountsCreateFreeTestCoinsRequest {
        account: Some(ComponentAddressOrName::Name(account_name.clone())),
        amount,
        max_fee: None,
        key_id: key_index,
    };

    let resp = client.create_free_test_coins(request).await.unwrap();
    // TODO: store the secret key in the world, but we don't have a need for it at the moment
    world.account_keys.insert(
        account_name.clone(),
        (RistrettoSecretKey::default(), resp.public_key.clone()),
    );
    let wait_req = TransactionWaitResultRequest {
        transaction_id: resp.result.transaction_hash.into_array().into(),
        timeout_secs: Some(120),
    };
    let _wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    add_substate_ids(
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

    let metadata = metadata.unwrap_or_else(|| {
        serde_json::json!({
            TOKEN_SYMBOL: "MY_NFT",
            "name": "TariProject",
            "departure": "Now",
            "landing_on": "Moon"
        })
    });

    let request = MintAccountNftRequest {
        account: ComponentAddressOrName::Name(account_name.clone()),
        metadata,
        mint_fee: Some(Amount::new(1_000)),
        create_account_nft_fee: None,
    };
    let resp = client
        .mint_account_nft(request)
        .await
        .expect("Failed to mint new account NFT");

    let wait_req = TransactionWaitResultRequest {
        transaction_id: resp.result.transaction_hash.into_array().into(),
        timeout_secs: Some(120),
    };
    let _wait_resp = client
        .wait_transaction_result(wait_req)
        .await
        .expect("Wait response failed");

    add_substate_ids(
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
    outputs_name: String,
    min_epoch: Option<Epoch>,
    max_epoch: Option<Epoch>,
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
                .map(move |(child_name, addr)| (format!("{}/{}", name, child_name), addr.substate_id.clone().into()))
        })
        .collect();

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
        .map(|(_, addr)| addr.clone())
        .collect::<Vec<_>>();

    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let account_name = ComponentAddressOrName::Name(account_signing_key);
    let AccountGetResponse { account, .. } = client.accounts_get(account_name).await.unwrap();

    let instructions = parse_manifest(&manifest_content, globals, HashMap::new()).unwrap();
    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: Some(account.key_index),
        instructions: instructions.instructions,
        fee_instructions: vec![],
        override_inputs: false,
        is_dry_run: false,
        proof_ids: vec![],
        inputs,
        min_epoch,
        max_epoch,
    };

    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        transaction_id: resp.transaction_id,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();
    if let Some(reason) = wait_resp.result.as_ref().and_then(|result| result.reject().cloned()) {
        panic!("Transaction failed: {}", reason);
    }

    add_substate_ids(
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
    outputs_name: String,
    min_epoch: Option<Epoch>,
    max_epoch: Option<Epoch>,
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
                .map(move |(child_name, addr)| (format!("{}/{}", name, child_name), addr.substate_id.clone().into()))
        })
        .collect();

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
        .map(|(_, addr)| addr.clone())
        .collect::<Vec<_>>();

    let instructions = parse_manifest(&manifest_content, globals, HashMap::new()).unwrap();

    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions: instructions.instructions,
        fee_instructions: vec![],
        override_inputs: false,
        is_dry_run: false,
        proof_ids: vec![],
        inputs,
        min_epoch,
        max_epoch,
    };

    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;
    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        transaction_id: resp.transaction_id,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    if let Some(reason) = wait_resp.result.clone().and_then(|finalize| finalize.reject().cloned()) {
        panic!("Transaction failed: {:?}", reason);
    }
    add_substate_ids(
        world,
        outputs_name,
        &wait_resp
            .result
            .expect("Transaction has timed out")
            .result
            .expect("Transaction has failed"),
    );
}

pub async fn submit_transaction(
    world: &mut TariWorld,
    wallet_daemon_name: String,
    fee_instructions: Vec<Instruction>,
    instructions: Vec<Instruction>,
    inputs: Vec<SubstateRequirement>,
    outputs_name: String,
    min_epoch: Option<Epoch>,
    max_epoch: Option<Epoch>,
) -> TransactionWaitResultResponse {
    let mut client = get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions,
        fee_instructions,
        override_inputs: false,
        is_dry_run: false,
        inputs,
        proof_ids: vec![],
        min_epoch,
        max_epoch,
    };

    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        transaction_id: resp.transaction_id,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    if let Some(diff) = wait_resp.result.as_ref().and_then(|r| r.result.accept()) {
        add_substate_ids(world, outputs_name, diff);
    }
    wait_resp
}

pub async fn create_component(
    world: &mut TariWorld,
    outputs_name: String,
    template_name: String,
    _account_name: String,
    wallet_daemon_name: String,
    function_call: String,
    args: Vec<String>,
    min_epoch: Option<Epoch>,
    max_epoch: Option<Epoch>,
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

    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions: vec![instruction],
        fee_instructions: vec![],
        override_inputs: false,
        is_dry_run: false,
        proof_ids: vec![],
        inputs: vec![],
        min_epoch,
        max_epoch,
    };

    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    let wait_req = TransactionWaitResultRequest {
        transaction_id: resp.transaction_id,
        timeout_secs: Some(120),
    };
    let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();

    if let Some(reason) = wait_resp.result.clone().and_then(|finalize| finalize.reject().cloned()) {
        panic!("Transaction failed: {}", reason);
    }
    add_substate_ids(
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
    let max_fee = Some(Amount(1));

    let request = TransferRequest {
        account,
        amount,
        resource_address,
        destination_public_key,
        max_fee,
        dry_run: false,
    };

    let resp = client.accounts_transfer(request).await.unwrap();
    add_substate_ids(world, outputs_name, resp.result.result.accept().unwrap());
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
    let max_fee = Some(Amount(2000));

    let request = ConfidentialTransferRequest {
        account,
        amount,
        destination_public_key,
        max_fee,
        resource_address: CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
        dry_run: false,
    };

    let resp = client.accounts_confidential_transfer(request).await.unwrap();
    add_substate_ids(world, outputs_name, resp.result.result.accept().unwrap());
}

pub async fn get_auth_wallet_daemon_client(world: &TariWorld, wallet_daemon_name: &str) -> WalletDaemonClient {
    world
        .wallet_daemons
        .get(wallet_daemon_name)
        .unwrap_or_else(|| panic!("Wallet daemon not found with name {}", wallet_daemon_name))
        .get_authed_client()
        .await
}
