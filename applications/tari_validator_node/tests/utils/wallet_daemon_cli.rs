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
use tari_engine_types::instruction::Instruction;
use tari_template_lib::{arg, args::Arg, prelude::NonFungibleId};
use tari_transaction_manifest::{parse_manifest, ManifestValue};
use tari_validator_node_cli::command::transaction::CliArg;
use tari_wallet_daemon_client::{
    types::{
        AccountsCreateRequest,
        AccountsGetBalancesRequest,
        ClaimBurnRequest,
        ClaimBurnResponse,
        TransactionGetRequest,
        TransactionGetResultRequest,
        TransactionSubmitRequest,
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
        fee: 1,
    };

    client.claim_burn(claim_burn_request).await.unwrap()
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

    let wallet_daemon_txs = world.wallet_daemons_txs.entry(wallet_daemon_name).or_default();
    wallet_daemon_txs.insert(
        account_name,
        resp.result.result.expect("Failed to obtain substate diffs"),
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
            let diff = world
                .wallet_daemons_txs
                .get(&wallet_daemon_name)
                .unwrap_or_else(|| panic!("No wallet daemon {} transactions", wallet_daemon_name))
                .get(s.trim())
                .unwrap_or_else(|| panic!("No outputs named {}", s.trim()));
            diff.up_iter()
        })
        .map(|(addr, state)| tari_dan_wallet_sdk::models::VersionedSubstateAddress {
            version: state.version(),
            address: addr.clone(),
        })
        .collect::<Vec<_>>();

    let instructions = parse_manifest(&manifest_content, globals).unwrap();
    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions,
        fee: 1,
        override_inputs: false,
        is_dry_run: false,
        proof_id: None,
        new_outputs: num_outputs as u8,
        specific_non_fungible_outputs,
        inputs,
        new_non_fungible_outputs,
        new_non_fungible_index_outputs,
    };

    let mut client = get_wallet_daemon_client(world, wallet_daemon_name.clone()).await;
    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    tokio::time::sleep(Duration::from_secs(5)).await;

    let get_tx_req = TransactionGetResultRequest { hash: resp.hash };
    let get_tx_resp = client.get_transaction_result(get_tx_req).await.unwrap();

    let wallet_daemon_txs = world.wallet_daemons_txs.entry(wallet_daemon_name).or_default();
    wallet_daemon_txs.insert(
        outputs_name,
        get_tx_resp
            .result
            .unwrap()
            .result
            .expect("Failed to obtain substate diffs"),
    );
}

pub async fn create_component(
    world: &mut TariWorld,
    outputs_name: String,
    template_name: String,
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

    let transaction_submit_req = TransactionSubmitRequest {
        signing_key_index: None,
        instructions: vec![instruction],
        fee: 1,
        override_inputs: false,
        is_dry_run: false,
        proof_id: None,
        new_outputs: num_outputs as u8,
        specific_non_fungible_outputs: vec![],
        inputs: vec![],
        new_non_fungible_outputs: vec![],
        new_non_fungible_index_outputs: vec![],
    };

    let mut client = get_wallet_daemon_client(world, wallet_daemon_name.clone()).await;
    let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

    tokio::time::sleep(Duration::from_secs(5)).await;

    let get_tx_req = TransactionGetResultRequest { hash: resp.hash };
    let get_tx_resp = client.get_transaction_result(get_tx_req).await.unwrap();

    let wallet_daemon_txs = world.wallet_daemons_txs.entry(wallet_daemon_name).or_default();
    wallet_daemon_txs.insert(
        outputs_name,
        get_tx_resp
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
