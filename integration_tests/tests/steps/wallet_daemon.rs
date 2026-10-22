//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{fs, time::Duration};

use cucumber::{gherkin::Step, then, when};
use integration_tests::{
    TariWorld,
    claim_proof::CucumberClaimProof,
    cucumber_log,
    util::transaction_builder,
    wallet_daemon_client,
};
use rand::RngExt;
use tari_crypto::tari_utilities::ByteArray;
use tari_engine_types::commit_result::FinalizeResult;
use tari_ootle_transaction::args;
use tari_ootle_wallet_sdk::{
    apis::{
        confidential_transfer::UtxoInputSelection,
        stealth_transfer::{BadgeUsage, TransferFeeParams},
    },
    crypto::pay_to::PayTo,
    models::OutputStatus,
};
use tari_ootle_walletd_client::{
    ComponentAddressOrName,
    types::{
        AccountsImportViewOnlyRequest,
        StealthTransfer,
        StealthTransferRequest,
        StealthUtxosListRequest,
        TransactionSubmitRequest,
        TransactionWaitResultRequest,
    },
};
use tari_template_lib_types::{Amount, bytes::Bytes, constants::TARI_TOKEN, crypto::PedersenCommitmentBytes};
use tari_transaction_components::{
    tari_amount::T,
    transaction_components::{MemoField, memo_field::TxType},
};

async fn claim_burn(
    world: &mut TariWorld,
    proof_name: String,
    account_name: String,
    wallet_daemon_name: String,
) -> anyhow::Result<FinalizeResult> {
    let claim_proof = world
        .claim_proofs
        .get(&proof_name)
        .unwrap_or_else(|| panic!("Burn proof {} not found", proof_name));
    let claim_proof = claim_proof
        .confirmed()
        .unwrap_or_else(|| panic!("Burn proof {} is not confirmed, cannot claim burn", proof_name));

    cucumber_log!("Claiming burn with proof: {:?}", claim_proof);
    let walletd = world.get_wallet_daemon(&wallet_daemon_name);
    // Then burn into the new account
    let claim_burn_resp = walletd.claim_burn(&account_name, claim_proof.clone()).await?;
    let resp = walletd
        .wait_for_transaction_result(claim_burn_resp.transaction_id)
        .await;
    assert!(!resp.timed_out, "Timed out waiting for claim burn transaction result");
    Ok(resp.result.expect("transaction result is None when claiming burn"))
}

#[when(expr = "I claim burn {word} and spend it into account {word} using wallet daemon {word}")]
#[then(expr = "I claim burn {word} and spend it into account {word} using wallet daemon {word}")]
async fn when_i_claim_burn_via_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    proof_name: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let result = claim_burn(world, proof_name, account_name, wallet_daemon_name)
        .await
        .unwrap();
    if let Some(ref reason) = result.any_reject() {
        cucumber_log!("Transaction failed: {}", reason);
        panic!("Transaction failed: {}", reason);
    }
}

#[when(expr = "I claim burn {word} and spend it into account {word} using wallet daemon {word}, it fails")]
async fn when_i_claim_burn_via_wallet_daemon_it_fails(
    world: &mut TariWorld,
    step: &Step,
    proof_name: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let result = claim_burn(world, proof_name, account_name, wallet_daemon_name)
        .await
        .unwrap();

    assert!(
        result.any_reject().is_some(),
        "Expected transaction to fail, but it succeeded: {:?}",
        result
    );
}

#[when(expr = "I claim fees for validator {word} into account {word} using the wallet daemon {word}")]
async fn when_i_claim_fees_for_validator_and_epoch(
    world: &mut TariWorld,
    step: &Step,
    validator_node: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let resp = wallet_daemon_client::claim_fees(world, wallet_daemon_name, account_name, validator_node, false)
        .await
        .unwrap();
    resp.result.result.any_accept().unwrap_or_else(|| {
        panic!(
            "Expected fee claim to succeed but failed with {}",
            resp.result.result.fee_reject().unwrap()
        )
    });
}

#[then(expr = "I run up {int} in fees using the wallet daemon {word} and account {word}")]
async fn when_i_run_up_fees(
    world: &mut TariWorld,
    step: &Step,
    amount: u64,
    wallet_daemon_name: String,
    account_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let template = world
        .templates
        .get("fees")
        .expect("fees template must be registered before this step can be used");
    let account = world
        .wallet_accounts
        .get(&account_name)
        .unwrap_or_else(|| panic!("No account named {}", account_name));

    let mut fees_total = 0;

    loop {
        let payload = Bytes::from(vec![rand::rng().random::<u8>(); 64 * 1024]);

        let transaction = transaction_builder()
            .pay_fee_from_component(*account.component_address(), 100_000u64)
            .call_function(template.address, "new", args![payload])
            .add_input(*account.component_address())
            .build_unsigned();

        let transaction_submit_req = TransactionSubmitRequest {
            transaction,
            seal_signer: account.owner_key_id().expect("no owner key id"),
            other_signers: vec![],
            signatures: vec![],
            detect_inputs: true,
            detect_inputs_use_unversioned: true,
            lock_ids: vec![],
        };

        let walletd = world.get_wallet_daemon(&wallet_daemon_name);
        let mut client = walletd.get_authed_client().await;
        let resp = client.submit_transaction(transaction_submit_req).await.unwrap();

        let wait_req = TransactionWaitResultRequest {
            transaction_id: resp.transaction_id,
            timeout_secs: Some(120),
        };
        let wait_resp = client.wait_transaction_result(wait_req).await.unwrap();
        if wait_resp.timed_out {
            panic!("Timed out waiting for transaction result");
        }
        if let Some(reason) = wait_resp.result.as_ref().unwrap().any_reject() {
            panic!("Transaction failed: {}", reason);
        }

        fees_total += wait_resp.result.as_ref().unwrap().fee_receipt.total_fees_paid();
        if fees_total >= amount {
            integration_tests::cucumber_log!("Reached target of {} fees", fees_total);
            break;
        }
        integration_tests::cucumber_log!("Accumulated {} fees, continuing", fees_total);
    }
}

#[when(expr = "I claim fees for validator {word} into account {word} using the wallet daemon {word}, it fails")]
async fn when_i_claim_fees_for_validator_and_epoch_fails(
    world: &mut TariWorld,
    step: &Step,
    validator_node: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let err = wallet_daemon_client::claim_fees(world, wallet_daemon_name, account_name, validator_node, false)
        .await
        .unwrap_err();

    cucumber_log!("Expected error: {}", err);
}

#[then(expr = "I create an account {word} via the wallet daemon {word}")]
#[when(expr = "I create an account {word} via the wallet daemon {word}")]
async fn when_i_create_account_via_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    account_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    wallet_daemon_client::create_account(world, wallet_daemon_name, account_name).await;
}

#[then(expr = "I create an account {word} via the wallet daemon {word} with {int} XTR")]
#[when(expr = "I create an account {word} via the wallet daemon {word} with {int} XTR")]
async fn when_i_create_account_via_wallet_daemon_with_free_coins(
    world: &mut TariWorld,
    step: &Step,
    account_name: String,
    wallet_daemon_name: String,
    // TODO: remove
    _amount: u64,
) {
    cucumber_log!("==== Step: {}", step.value);
    wallet_daemon_client::create_account_with_free_coins(world, account_name, wallet_daemon_name).await;
}

#[when(expr = "I burn {int}T on wallet {word} for wallet daemon {word} into proof {word}")]
async fn when_i_burn_funds_with_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    amount: u64,
    wallet_name: String,
    wallet_daemon_name: String,
    proof_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let mut wallet_daemon_client =
        wallet_daemon_client::get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;

    let account = wallet_daemon_client.accounts_get_default().await.unwrap();

    let public_key = account.account.owner_public_key();
    integration_tests::cucumber_log!("Burning funds using claim key {}", public_key);

    let wallet = world.get_wallet(&wallet_name);

    let amount = (amount * T).as_u64();
    let mut client = wallet.create_client().await;
    let resp = client
        .create_burn_transaction(minotari_app_grpc::tari_rpc::CreateBurnTransactionRequest {
            amount,
            fee_per_gram: 1,
            payment_id: MemoField::new_open_from_string("Burn", TxType::Burn)
                .unwrap()
                .to_bytes(),
            claim_public_key: public_key.to_vec(),
            sidechain_deployment_key: vec![],
        })
        .await
        .unwrap()
        .into_inner();

    assert!(resp.is_success, "Burn transaction failed: {}", resp.failure_message);

    let commitment = PedersenCommitmentBytes::from_bytes(&resp.commitment).expect("commitment parse error");

    // Extract kernel signature data
    let kernel_excess_sig_nonce = resp.kernel_excess_nonce.clone();
    let kernel_excess_sig_signature = resp.kernel_excess_signature.clone();

    integration_tests::cucumber_log!(
        "Burn transaction created with kernel_excess_sig nonce: {}, signature: {}",
        hex::encode(&kernel_excess_sig_nonce),
        hex::encode(&kernel_excess_sig_signature)
    );

    world.claim_proofs.insert(proof_name, CucumberClaimProof::Pending {
        commitment,
        kernel_excess_sig_nonce,
        kernel_excess_sig_signature,
    });
}

#[when(regex = r"I check the balance of (\S+) on wallet daemon (\S+) the amount is (at )?(\S+) (\d+)")]
async fn check_account_balance_via_daemon(
    world: &mut TariWorld,
    step: &Step,
    account_name: String,
    wallet_daemon_name: String,
    _at: String,
    least_or_most: String,
    amount: i64,
) {
    cucumber_log!("==== Step: {}", step.value);
    // This also refreshes the wallet vaults
    let current_balance =
        wallet_daemon_client::get_balance(world, &account_name, &wallet_daemon_name, TARI_TOKEN).await;
    world
        .last_checked_balances
        .insert(account_name.clone(), current_balance);
    match least_or_most.to_lowercase().as_str() {
        "least" => {
            if current_balance < amount {
                cucumber_log!("Expected balance to be at least {} but was {}", amount, current_balance);
                panic!("Expected balance to be at least {} but was {}", amount, current_balance);
            }
        },
        "most" => {
            if current_balance > amount {
                cucumber_log!("Expected balance to be at most {} but was {}", amount, current_balance);
                panic!("Expected balance to be at most {} but was {}", amount, current_balance);
            }
        },
        "exactly" => {
            if current_balance != amount {
                cucumber_log!("Expected balance to be exactly {} but was {}", amount, current_balance);
                panic!("Expected balance to be exactly {} but was {}", amount, current_balance);
            }
        },

        _ => panic!("Expected 'at least', 'at most' or 'exactly', got {}", least_or_most),
    }
}

#[when(regex = r"I check the balance of (\S+) on wallet daemon (\S+) increased by at most (\d+) since the last check")]
async fn check_account_balance_increase_via_daemon(
    world: &mut TariWorld,
    step: &Step,
    account_name: String,
    wallet_daemon_name: String,
    max_increase: u64,
) {
    cucumber_log!("==== Step: {}", step.value);
    let previous = *world
        .last_checked_balances
        .get(&account_name)
        .unwrap_or_else(|| panic!("Balance of {} has not been checked before this step", account_name));
    let current_balance =
        wallet_daemon_client::get_balance(world, &account_name, &wallet_daemon_name, TARI_TOKEN).await;
    world
        .last_checked_balances
        .insert(account_name.clone(), current_balance);
    let increase = current_balance - previous;
    if increase > Amount::from(max_increase) {
        cucumber_log!(
            "Expected balance to increase by at most {} but it went from {} to {} ({})",
            max_increase,
            previous,
            current_balance,
            increase
        );
        panic!(
            "Expected balance to increase by at most {} but it went from {} to {} ({})",
            max_increase, previous, current_balance, increase
        );
    }
}

#[when(
    regex = r"I check the balance of (\S+) for resource (\S+) on wallet daemon (\S+) the amount is (at )?(\S+) (\d+)"
)]
async fn check_account_balance_for_resource_via_daemon(
    world: &mut TariWorld,
    step: &Step,
    account_name: String,
    resource_input_name: String,
    wallet_daemon_name: String,
    _at: String,
    least_or_most: String,
    amount: i64,
) {
    cucumber_log!("==== Step: {}", step.value);
    let output = world.get_output_fq(&resource_input_name);
    // This also refreshes the wallet vaults
    let current_balance = wallet_daemon_client::get_balance(
        world,
        &account_name,
        &wallet_daemon_name,
        output
            .substate_id
            .as_resource_address()
            .expect("output is not resource"),
    )
    .await;
    match least_or_most.to_lowercase().as_str() {
        "least" => {
            if current_balance < amount {
                cucumber_log!("Expected balance to be at least {} but was {}", amount, current_balance);
                panic!("Expected balance to be at least {} but was {}", amount, current_balance);
            }
        },
        "most" => {
            if current_balance > amount {
                cucumber_log!("Expected balance to be at most {} but was {}", amount, current_balance);
                panic!("Expected balance to be at most {} but was {}", amount, current_balance);
            }
        },
        "exactly" => {
            if current_balance != amount {
                cucumber_log!("Expected balance to be exactly {} but was {}", amount, current_balance);
                panic!("Expected balance to be exactly {} but was {}", amount, current_balance);
            }
        },

        _ => panic!("Expected 'at least', 'at most' or 'exactly', got {}", least_or_most),
    }
}

#[when(expr = "I wait for {word} on wallet daemon {word} to have balance {word} {int}")]
#[then(expr = "I wait for {word} on wallet daemon {word} to have balance {word} {int}")]
async fn wait_account_balance_via_daemon(
    world: &mut TariWorld,
    step: &Step,
    account_name: String,
    wallet_daemon_name: String,
    operator: String,
    amount: i64,
) {
    cucumber_log!("==== Step: {}", step.value);
    wait_account_balance_impl(world, &account_name, &wallet_daemon_name, &operator, amount, 30).await;
}

#[when(expr = "I wait for {word} on wallet daemon {word} to have balance {word} {int} with timeout {int}s")]
#[then(expr = "I wait for {word} on wallet daemon {word} to have balance {word} {int} with timeout {int}s")]
async fn wait_account_balance_via_daemon_with_timeout(
    world: &mut TariWorld,
    step: &Step,
    account_name: String,
    wallet_daemon_name: String,
    operator: String,
    amount: i64,
    timeout_secs: i64,
) {
    cucumber_log!("==== Step: {}", step.value);
    let timeout_secs = u32::try_from(timeout_secs)
        .ok()
        .filter(|secs| *secs > 0)
        .unwrap_or_else(|| panic!("Expected timeout to be a positive integer that fits into u32, got {timeout_secs}"));
    wait_account_balance_impl(
        world,
        &account_name,
        &wallet_daemon_name,
        &operator,
        amount,
        timeout_secs,
    )
    .await;
}

async fn wait_account_balance_impl(
    world: &mut TariWorld,
    account_name: &str,
    wallet_daemon_name: &str,
    operator: &str,
    amount: i64,
    timeout_secs: u32,
) {
    let op = match operator {
        "gt" => |a, b| a > b,
        "gte" => |a, b| a >= b,
        "lt" => |a, b| a < b,
        "lte" => |a, b| a <= b,
        "eq" => |a, b| a == b,
        _ => panic!("Expected gt, gte, lt, lte or eq, got {}", operator),
    };

    let mut i = 0;
    loop {
        // This also refreshes the wallet vaults
        let current_balance =
            wallet_daemon_client::get_balance(world, account_name, wallet_daemon_name, TARI_TOKEN).await;
        if op(current_balance, amount) {
            break;
        }

        i += 1;
        if i == timeout_secs {
            panic!("Timeout waiting for balance. Current balance = {}", current_balance);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[when(expr = "I check the confidential balance of {word} on wallet daemon {word} the amount is at {word} {int}")]
async fn check_account_confidential_balance_is_via_daemon(
    world: &mut TariWorld,
    step: &Step,
    account_name: String,
    wallet_daemon_name: String,
    least_or_most: String,
    amount: i64,
) {
    cucumber_log!("==== Step: {}", step.value);
    // This also refreshes the wallet vaults
    let current_balance = wallet_daemon_client::get_confidential_balance(world, account_name, wallet_daemon_name).await;
    match least_or_most.to_lowercase().as_str() {
        "least" => {
            if current_balance < amount {
                cucumber_log!("Expected balance to be at least {} but was {}", amount, current_balance);
                panic!("Expected balance to be at least {} but was {}", amount, current_balance);
            }
        },
        "most" => {
            if current_balance > amount {
                cucumber_log!("Expected balance to be at most {} but was {}", amount, current_balance);
                panic!("Expected balance to be at most {} but was {}", amount, current_balance);
            }
        },
        _ => panic!("Expected least or most, got {}", least_or_most),
    }
}

#[when(
    regex = r"I transfer (\d+) tokens of resource (\S+) from account (\S+) to account (\S+) via the wallet daemon (\S+) named (\S+)"
)]
async fn when_transfer_via_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    amount: u32,
    resource_name: String,
    account_name: String,
    dest_account: String,
    wallet_daemon_name: String,
    outputs_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let amount = Amount::from(amount);

    let resource_address = world
        .get_output_fq(&resource_name)
        .substate_id()
        .as_resource_address()
        .unwrap_or_else(|| panic!("{} is not a resource", resource_name));

    let destination_account = world
        .wallet_accounts
        .get(&dest_account)
        .unwrap_or_else(|| panic!("No account address found with name {}", dest_account));
    wallet_daemon_client::transfer(
        world,
        account_name,
        *destination_account.address.account_public_key(),
        resource_address,
        amount,
        wallet_daemon_name,
        outputs_name,
    )
    .await;
}

#[when(
    regex = r"I do a confidential transfer of (\d+) from (\S+) to (\S+) for resource (\S+) selecting (\S+) inputs creating output (\S+) via the wallet daemon (\S+)"
)]
#[allow(clippy::too_many_arguments)]
async fn when_i_do_a_confidential_transfer_via_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    amount: u64,
    source_account_name: String,
    dest_account_name: String,
    resource_input_name: String,
    input_selection: String,
    outputs_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let resource_address = world
        .get_output_fq(&resource_input_name)
        .substate_id
        .as_resource_address()
        .expect("output is not a resource");

    let input_selection = match input_selection.to_lowercase().as_str() {
        "confidential" => UtxoInputSelection::ConfidentialOnly,
        "revealed" => UtxoInputSelection::RevealedOnly,
        "prefer-revealed" => UtxoInputSelection::PreferRevealed,
        "prefer-confidential" => UtxoInputSelection::PreferConfidential,
        other => panic!("Unknown input selection '{other}'"),
    };

    let destination_address = {
        let mut client = wallet_daemon_client::get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;
        client
            .accounts_get(ComponentAddressOrName::Name(dest_account_name))
            .await
            .expect("Failed to resolve destination account by name")
            .address
    };

    wallet_daemon_client::confidential_transfer(
        world,
        source_account_name,
        destination_address,
        amount.into(),
        wallet_daemon_name,
        outputs_name,
        resource_address,
        input_selection,
    )
    .await;
}

#[then(
    expr = "I do a stealth transfer with amount {int} from {word} to {word} creating output {word} via the \
            wallet_daemon {word}"
)]
async fn when_i_create_transfer_proof_via_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    amount: u64,
    source_account_name: String,
    dest_account_name: String,
    outputs_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    wallet_daemon_client::transfer_stealth(
        world,
        source_account_name,
        dest_account_name,
        amount,
        wallet_daemon_name,
        outputs_name,
        // TODO: support for custom stealth resources
        TARI_TOKEN,
    )
    .await;
}

#[when(
    expr = "I do a stealth transfer of {int} from account {word} to account {word} via the wallet daemon {word} named \
            {word}"
)]
async fn when_stealth_transfer_via_wallet_daemon(
    world: &mut TariWorld,
    step: &Step,
    amount: u64,
    account_name: String,
    destination_acc_name: String,
    wallet_daemon_name: String,
    outputs_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    wallet_daemon_client::transfer_stealth(
        world,
        account_name,
        destination_acc_name,
        amount,
        wallet_daemon_name,
        outputs_name,
        TARI_TOKEN,
    )
    .await;
}

#[when(expr = "I set the default account for {word} to {word}")]
async fn when_i_set_the_default_account(world: &mut TariWorld, step: &Step, wallet_name: String, account_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    let wallet = world.get_wallet_daemon(&wallet_name);
    let mut client = wallet.get_authed_client().await;
    client
        .accounts_set_default(ComponentAddressOrName::Name(account_name))
        .await
        .unwrap();
}

/// Writes a confirmed burn proof as a JSON file into the wallet daemon's `burn_proof_dir`.
/// The auto-claim service watches that directory and will submit the claim transaction
/// automatically, so the test does not need to call the RPC claim endpoint manually.
#[when(expr = "I drop burn proof {word} as a file for wallet daemon {word}")]
async fn when_i_drop_burn_proof_as_file(
    world: &mut TariWorld,
    step: &Step,
    proof_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let complete_proof = world
        .claim_proofs
        .get(&proof_name)
        .unwrap_or_else(|| panic!("Burn proof {} not found", proof_name))
        .complete_proof()
        .unwrap_or_else(|| panic!("Burn proof {} is not confirmed yet", proof_name))
        .clone();

    let walletd = world.get_wallet_daemon(&wallet_daemon_name);
    let dir = walletd.burn_proof_dir.clone();
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("Failed to create burn_proof_dir {}: {}", dir.display(), e));

    let file_path = dir.join(format!("{}.json", proof_name));
    let file = fs::File::create(&file_path)
        .unwrap_or_else(|e| panic!("Failed to create proof file {}: {}", file_path.display(), e));
    serde_json::to_writer_pretty(file, &complete_proof)
        .unwrap_or_else(|e| panic!("Failed to write proof file {}: {}", file_path.display(), e));

    cucumber_log!("Dropped burn proof {} as file {}", proof_name, file_path.display());
}

#[when(
    expr = "I import account {word} from wallet daemon {word} as view-only account {word} into wallet daemon {word}"
)]
async fn when_i_import_view_only_account(
    world: &mut TariWorld,
    step: &Step,
    source_account_name: String,
    source_wallet_daemon_name: String,
    account_name: String,
    wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let mut source = wallet_daemon_client::get_auth_wallet_daemon_client(world, &source_wallet_daemon_name).await;
    let exported = source
        .accounts_export_view_key(ComponentAddressOrName::Name(source_account_name))
        .await
        .unwrap();

    let mut client = wallet_daemon_client::get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;
    client
        .accounts_import_view_only(AccountsImportViewOnlyRequest {
            account_name,
            account_public_key: exported.account_public_key,
            view_secret_key: exported.view_secret_key,
            birthday_epoch: Some(exported.birthday_epoch),
            is_default: None,
        })
        .await
        .unwrap();
}

#[when(
    expr = "I do a stealth transfer of {int} from account {word} on wallet daemon {word} to account {word} on wallet \
            daemon {word} with pay ref {word}"
)]
#[allow(clippy::too_many_arguments)]
async fn when_i_stealth_transfer_with_pay_ref(
    world: &mut TariWorld,
    step: &Step,
    amount: u64,
    source_account_name: String,
    source_wallet_daemon_name: String,
    dest_account_name: String,
    dest_wallet_daemon_name: String,
    pay_ref: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let mut dest = wallet_daemon_client::get_auth_wallet_daemon_client(world, &dest_wallet_daemon_name).await;
    let destination_address = dest
        .accounts_get(ComponentAddressOrName::Name(dest_account_name))
        .await
        .unwrap()
        .address;

    let mut client = wallet_daemon_client::get_auth_wallet_daemon_client(world, &source_wallet_daemon_name).await;
    let resp = client
        .accounts_stealth_transfer(StealthTransferRequest {
            owner_account: ComponentAddressOrName::Name(source_account_name),
            fee_params: TransferFeeParams::new(UtxoInputSelection::PreferRevealed),
            input_selection: UtxoInputSelection::PreferRevealed,
            resource_address: TARI_TOKEN,
            badge_usage: BadgeUsage::None,
            transfers: vec![StealthTransfer {
                destination_address,
                blinded_output_amount: amount,
                revealed_output_amount: Amount::zero(),
                output_memo: None,
                pay_to: PayTo::StealthPublicKey,
                attach_sender_address: true,
                pay_ref: Some(pay_ref),
            }],
            max_fee: 50_000,
            dry_run: false,
        })
        .await
        .unwrap();
    let wait_resp = client
        .wait_transaction_result(TransactionWaitResultRequest {
            transaction_id: resp.transaction_id,
            timeout_secs: Some(120),
        })
        .await
        .unwrap();
    if let Some(reason) = wait_resp.result.expect("Transaction has timed out").result.any_reject() {
        panic!("Transaction failed: {}", reason);
    }
}

#[then(
    expr = "wallet daemon {word} lists a stealth output of {int} for account {word} with pay ref {word} sent by \
            account {word} on wallet daemon {word}"
)]
#[allow(clippy::too_many_arguments)]
async fn then_wallet_daemon_lists_stealth_output_with_pay_ref(
    world: &mut TariWorld,
    step: &Step,
    wallet_daemon_name: String,
    amount: u64,
    account_name: String,
    pay_ref: String,
    sender_account_name: String,
    sender_wallet_daemon_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let mut sender = wallet_daemon_client::get_auth_wallet_daemon_client(world, &sender_wallet_daemon_name).await;
    let sender_address = sender
        .accounts_get(ComponentAddressOrName::Name(sender_account_name))
        .await
        .unwrap()
        .address;

    let mut client = wallet_daemon_client::get_auth_wallet_daemon_client(world, &wallet_daemon_name).await;
    let account_address = *client
        .accounts_get(ComponentAddressOrName::Name(account_name.clone()))
        .await
        .unwrap()
        .account
        .component_address();

    for _ in 0..60 {
        // Requests a UTXO scan for the account
        wallet_daemon_client::get_balance(world, &account_name, &wallet_daemon_name, TARI_TOKEN).await;
        let utxos = client
            .stealth_utxos_list(StealthUtxosListRequest {
                resource_address: TARI_TOKEN,
                account_address: Some(account_address),
                filter_by_status: None,
            })
            .await
            .unwrap()
            .utxos;
        if let Some(utxo) = utxos.iter().find(|u| u.value == Amount::from(amount)) {
            assert!(matches!(utxo.status, OutputStatus::Unspent), "{:?}", utxo.status);
            let (_, _, memo_pay_ref) = utxo
                .memo
                .as_ref()
                .and_then(|m| m.as_sender_address())
                .expect("UTXO memo is not a sender address");
            assert_eq!(memo_pay_ref, pay_ref.as_bytes());
            let utxo_sender = utxo.sender_address.as_ref().expect("UTXO has no sender address");
            assert_eq!(utxo_sender.account_public_key(), sender_address.account_public_key());
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("Timed out waiting for a stealth output of {amount} on account {account_name}");
}
