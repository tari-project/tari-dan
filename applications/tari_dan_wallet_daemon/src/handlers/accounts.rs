//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::convert::{TryFrom, TryInto};

use anyhow::anyhow;
use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use base64;
use log::*;
use serde_json as json;
use tari_common_types::types::{FixedHash, PrivateKey, PublicKey};
use tari_crypto::{
    commitment::{HomomorphicCommitment as Commitment, HomomorphicCommitmentFactory},
    keys::PublicKey as _,
    ristretto::RistrettoComSig,
};
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_dan_wallet_sdk::{
    apis::key_manager,
    confidential::{get_commitment_factory, ConfidentialProofStatement},
    models::{ConfidentialOutputModel, OutputStatus, VersionedSubstateAddress},
};
use tari_engine_types::{confidential::ConfidentialClaim, instruction::Instruction, substate::SubstateAddress};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, NonFungibleAddress, UnclaimedConfidentialOutputAddress},
    prelude::{ResourceType, CONFIDENTIAL_TARI_RESOURCE_ADDRESS},
};
use tari_transaction::Transaction;
use tari_utilities::ByteArray;
use tari_wallet_daemon_client::types::{
    AccountByNameRequest,
    AccountByNameResponse,
    AccountsCreateRequest,
    AccountsCreateResponse,
    AccountsGetBalancesRequest,
    AccountsGetBalancesResponse,
    AccountsInvokeRequest,
    AccountsInvokeResponse,
    AccountsListRequest,
    AccountsListResponse,
    BalanceEntry,
    ClaimBurnRequest,
    ClaimBurnResponse,
    ConfidentialTransferRequest,
    ConfidentialTransferResponse,
    RevealFundsRequest,
    RevealFundsResponse,
};
use tokio::{sync::broadcast, task};

use super::context::HandlerContext;
use crate::services::{TransactionFinalizedEvent, TransactionSubmittedEvent, WalletEvent};

const LOG_TARGET: &str = "tari::dan_wallet_daemon::handlers::transaction";

pub async fn handle_create(
    context: &HandlerContext,
    req: AccountsCreateRequest,
) -> Result<AccountsCreateResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();

    if let Some(name) = req.account_name.as_ref() {
        if sdk.accounts_api().get_account_by_name(name).optional()?.is_some() {
            return Err(anyhow!("Account name '{}' already exists", name));
        }
    }

    let (key_index, signing_key) = sdk
        .key_manager_api()
        .get_key_or_active(key_manager::TRANSACTION_BRANCH, req.signing_key_index)?;
    let owner_pk = sdk
        .key_manager_api()
        .get_public_key(key_manager::TRANSACTION_BRANCH, req.signing_key_index)?;
    let owner_token =
        NonFungibleAddress::from_public_key(RistrettoPublicKeyBytes::from_bytes(owner_pk.as_bytes()).unwrap());

    info!(target: LOG_TARGET, "Creating account with owner token {}", owner_pk);

    let transaction = Transaction::builder()
        .call_function(ACCOUNT_TEMPLATE_ADDRESS, "create", args![owner_token])
        .with_new_outputs(1)
        .sign(&signing_key.k)
        .build();

    let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;

    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent { hash: tx_hash });

    let finalized = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow!("Create account transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.transaction_failure {
        return Err(anyhow!("Create account transaction failed: {}", reason));
    }
    let diff = finalized.finalize.result.accept().unwrap();
    let (addr, _) = diff
        .up_iter()
        .find(|(addr, _)| addr.is_component())
        .ok_or_else(|| anyhow!("Create account transaction accepted but no component address was returned"))?;

    sdk.accounts_api()
        .add_account(req.account_name.as_deref(), addr, key_index)?;

    Ok(AccountsCreateResponse {
        address: addr.clone(),
        public_key: owner_pk,
        result: finalized.finalize,
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    req: AccountsListRequest,
) -> Result<AccountsListResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let accounts = sdk.accounts_api().get_many(req.offset, req.limit)?;
    let total = sdk.accounts_api().count()?;
    let km = sdk.key_manager_api();
    let accounts = accounts
        .into_iter()
        .map(|a| {
            let key = km.derive_key(key_manager::TRANSACTION_BRANCH, a.key_index)?;
            let pk = PublicKey::from_secret_key(&key.k);
            Ok((a, pk))
        })
        .collect::<Result<_, anyhow::Error>>()?;

    Ok(AccountsListResponse { accounts, total })
}

pub async fn handle_invoke(
    context: &HandlerContext,
    req: AccountsInvokeRequest,
) -> Result<AccountsInvokeResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let (_, signing_key) = sdk
        .key_manager_api()
        .get_key_or_active(key_manager::TRANSACTION_BRANCH, None)?;

    let account = sdk.accounts_api().get_account_by_name(&req.account_name)?;
    let account_substate = sdk.substate_api().get_substate(&account.address)?;

    let vault = sdk
        .accounts_api()
        .get_vault_by_resource(&account.address, &CONFIDENTIAL_TARI_RESOURCE_ADDRESS)?;

    let vault_substate = sdk.substate_api().get_substate(&vault.address)?;
    // Because of the fee, we pledge that the account and confidential vault substates will be mutated
    let outputs = vec![
        ShardId::from_address(&account_substate.address.address, account_substate.address.version + 1),
        ShardId::from_address(&vault_substate.address.address, vault_substate.address.version + 1),
    ];

    let inputs = sdk
        .substate_api()
        .load_dependent_substates(&[account.address.clone()])?;

    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let account_address = account.address.as_component_address().unwrap();
    let transaction = Transaction::builder()
        .fee_transaction_pay_from_component(account_address, req.fee)
        .call_method(account_address, &req.method, req.args)
        .with_inputs(inputs)
        .with_outputs(outputs)
        .sign(&signing_key.k)
        .build();

    let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;
    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent { hash: tx_hash });

    let mut finalized = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reject) = finalized.transaction_failure {
        return Err(anyhow!("Transaction rejected: {}", reject));
    }

    Ok(AccountsInvokeResponse {
        result: finalized.finalize.execution_results.pop(),
    })
}

pub async fn handle_get_balances(
    context: &HandlerContext,
    req: AccountsGetBalancesRequest,
) -> Result<AccountsGetBalancesResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let account = sdk
        .accounts_api()
        .get_account_by_name(&req.account_name)
        .optional()?
        .ok_or_else(|| {
            JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(404),
                format!("Account '{}' not found", req.account_name),
                json::Value::Null,
            )
        })?;
    let vaults = sdk.accounts_api().get_vaults_by_account(&account.address)?;
    let outputs_api = sdk.confidential_outputs_api();

    let mut balances = Vec::with_capacity(vaults.len());
    for vault in vaults {
        let confidential_balance = if matches!(vault.resource_type, ResourceType::Confidential) {
            Amount::try_from(outputs_api.get_unspent_balance(&vault.address)?)?
        } else {
            Amount::zero()
        };

        balances.push(BalanceEntry {
            vault_address: vault.address,
            resource_address: vault.resource_address,
            balance: vault.balance,
            resource_type: vault.resource_type,
            confidential_balance,
            token_symbol: vault.token_symbol,
        })
    }

    Ok(AccountsGetBalancesResponse {
        address: account.address,
        balances,
    })
}

pub async fn handle_get_by_name(
    context: &HandlerContext,
    req: AccountByNameRequest,
) -> Result<AccountByNameResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let account = sdk.accounts_api().get_account_by_name(&req.name)?;
    let km = sdk.key_manager_api();
    let key = km.derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
    let public_key = PublicKey::from_secret_key(&key.k);
    Ok(AccountByNameResponse { account, public_key })
}

#[allow(clippy::too_many_lines)]
pub async fn handle_reveal_funds(
    context: &HandlerContext,
    req: RevealFundsRequest,
) -> Result<RevealFundsResponse, anyhow::Error> {
    let notifier = context.notifier().clone();
    let sdk = context.wallet_sdk().clone();

    task::spawn(async move {
        let mut inputs = vec![];

        if req.pay_fee_from_reveal && req.amount_to_reveal < req.fee {
            return Err(JsonRpcError::new(
                JsonRpcErrorReason::InvalidParams,
                "When pay_fee_from_reveal is true, amount to reveal must be greater than or equal to the fee"
                    .to_string(),
                json::Value::Null,
            )
            .into());
        }

        let account = sdk.accounts_api().get_account(&req.account.into())?;

        // Add the account component
        let account_substate = sdk.substate_api().get_substate(&req.account.into())?;
        inputs.push(account_substate.address);

        // Add all versioned account child addresses as inputs
        let child_addresses = sdk.substate_api().load_dependent_substates(&[req.account.into()])?;
        inputs.extend(child_addresses);

        let vault = sdk
            .accounts_api()
            .get_vault_by_resource(&req.account.into(), &CONFIDENTIAL_TARI_RESOURCE_ADDRESS)?;

        let proof_id = sdk.confidential_outputs_api().add_proof(&vault.address)?;

        let (inputs, input_value) = sdk.confidential_outputs_api().lock_outputs_by_amount(
            &vault.address,
            req.amount_to_reveal.as_u64_checked().unwrap(),
            proof_id,
        )?;
        let input_amount = Amount::try_from(input_value)?;

        let account_key = sdk
            .key_manager_api()
            .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
        let account_public_key = PublicKey::from_secret_key(&account_key.k);

        let (output_mask, public_nonce) = sdk
            .confidential_crypto_api()
            .derive_output_mask_for_destination(&account_public_key);

        let output_statement = ConfidentialProofStatement {
            amount: input_amount - req.amount_to_reveal,
            mask: output_mask,
            sender_public_nonce: Some(public_nonce),
            minimum_value_promise: 0,
            reveal_amount: req.amount_to_reveal,
        };

        let inputs = sdk
            .confidential_outputs_api()
            .resolve_output_masks(inputs, key_manager::TRANSACTION_BRANCH)?;

        let reveal_proof = sdk
            .confidential_crypto_api()
            .generate_withdraw_proof(&inputs, &output_statement, None)?;

        info!(
            target: LOG_TARGET,
            "Locked {} inputs ({}) for reveal funds transaction on account: {}",
            inputs.len(),
            input_amount,
            account.address
        );

        let account_address = account.address.as_component_address().unwrap();

        let mut builder = Transaction::builder();
        if req.pay_fee_from_reveal {
            builder = builder.with_fee_instructions(vec![
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "withdraw_confidential".to_string(),
                    args: args![CONFIDENTIAL_TARI_RESOURCE_ADDRESS, reveal_proof],
                },
                Instruction::PutLastInstructionOutputOnWorkspace {
                    key: b"revealed".to_vec(),
                },
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "deposit".to_string(),
                    args: args![Workspace("revealed".to_string())],
                },
                Instruction::CallMethod {
                    component_address: account_address,
                    method: "pay_fee".to_string(),
                    args: args![req.fee],
                },
            ]);
        } else {
            builder = builder
                .fee_transaction_pay_from_component(account_address, req.fee)
                .call_method(account_address, "withdraw_confidential", args![
                    CONFIDENTIAL_TARI_RESOURCE_ADDRESS,
                    reveal_proof
                ])
                .put_last_instruction_output_on_workspace("revealed")
                .call_method(account_address, "deposit", args![Workspace("revealed")]);
        }

        // Add the account component
        let account_substate = sdk.substate_api().get_substate(&account.address)?;
        // Add all versioned account child addresses as inputs
        let child_addresses = sdk.substate_api().load_dependent_substates(&[account.address])?;
        let mut inputs = vec![account_substate.address];
        inputs.extend(child_addresses);

        // TODO: we assume that all inputs will be consumed and produce a new output however this is only the case when
        // the       object is mutated
        let outputs = inputs
            .iter()
            .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version + 1))
            .collect::<Vec<_>>();

        let inputs = inputs
            .into_iter()
            .map(|addr| ShardId::from_address(&addr.address, addr.version))
            .collect();

        let transaction = builder
            .with_inputs(inputs)
            .with_outputs(outputs)
            .sign(&account_key.k)
            .build();

        sdk.confidential_outputs_api()
            .proofs_set_transaction_hash(proof_id, *transaction.hash())?;

        let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;

        let mut events = notifier.subscribe();
        notifier.notify(TransactionSubmittedEvent { hash: tx_hash });

        let finalized = wait_for_result(&mut events, tx_hash).await?;
        if let Some(reason) = finalized.transaction_failure {
            return Err(anyhow::anyhow!("Transaction failed: {}", reason));
        }

        Ok(RevealFundsResponse {
            hash: tx_hash,
            fee: finalized.final_fee,
            result: finalized.finalize,
        })
    })
    .await?
}

#[allow(clippy::too_many_lines)]
pub async fn handle_claim_burn(
    context: &HandlerContext,
    req: ClaimBurnRequest,
) -> Result<ClaimBurnResponse, anyhow::Error> {
    let ClaimBurnRequest {
        account: account_address,
        claim_proof,
        fee,
    } = req;

    let reciprocal_claim_public_key = PublicKey::from_bytes(
        &base64::decode(
            claim_proof["reciprocal_claim_public_key"]
                .as_str()
                .ok_or_else(|| invalid_params("reciprocal_claim_public_key", None))?,
        )
        .map_err(|e| invalid_params("reciprocal_claim_public_key", Some(e.to_string())))?,
    )?;
    let commitment = base64::decode(
        claim_proof["commitment"]
            .as_str()
            .ok_or_else(|| invalid_params("commitment", None))?,
    )
    .map_err(|e| invalid_params("commitment", Some(e.to_string())))?;
    let range_proof = base64::decode(
        claim_proof["range_proof"]
            .as_str()
            .or_else(|| claim_proof["rangeproof"].as_str())
            .ok_or_else(|| invalid_params("range_proof", None))?,
    )
    .map_err(|e| invalid_params("range_proof", Some(e.to_string())))?;

    let public_nonce = PublicKey::from_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["public_nonce"]
                .as_str()
                .ok_or_else(|| invalid_params("ownership_proof.public_nonce", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.public_nonce", Some(e.to_string())))?,
    )?;
    let u = PrivateKey::from_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["u"]
                .as_str()
                .ok_or_else(|| invalid_params("ownership_proof.u", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.u", Some(e.to_string())))?,
    )?;
    let v = PrivateKey::from_bytes(
        &base64::decode(
            claim_proof["ownership_proof"]["v"]
                .as_str()
                .ok_or_else(|| invalid_params("ownership_proof.v", None))?,
        )
        .map_err(|e| invalid_params("ownership_proof.v", Some(e.to_string())))?,
    )?;

    let sdk = context.wallet_sdk();
    let account = sdk.accounts_api().get_account(&account_address.into())?;
    let account_secret_key = sdk
        .key_manager_api()
        .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
    let account_public_key = PublicKey::from_secret_key(&account_secret_key.k);

    info!(
        target: LOG_TARGET,
        "Signing claim burn with key {}. This must be the same as the claiming key used in the burn transaction.",
        account_public_key
    );

    let mut inputs = vec![];

    // Add the account component
    let account_substate = sdk.substate_api().get_substate(&account.address)?;
    inputs.push(account_substate.address);

    // Add all versioned account child addresses as inputs
    let child_addresses = sdk.substate_api().load_dependent_substates(&[account.address])?;
    inputs.extend(child_addresses);

    // TODO: we assume that all inputs will be consumed and produce a new output however this is only the case when the
    //       object is mutated
    let outputs = inputs
        .iter()
        .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version + 1))
        .collect::<Vec<_>>();

    // add the commitment substate address as input to the claim burn transaction
    let commitment_substate_address = VersionedSubstateAddress {
        address: SubstateAddress::UnclaimedConfidentialOutput(UnclaimedConfidentialOutputAddress::try_from(
            commitment.as_slice(),
        )?),
        version: 0,
    };
    inputs.push(commitment_substate_address.clone());

    info!(
        target: LOG_TARGET,
        "Loaded {} inputs for claim burn transaction on account: {}",
        inputs.len(),
        account_address
    );

    // We have to unmask the commitment to allow us to reveal funds for the fee payment
    let (_, output) = sdk
        .substate_api()
        .scan_from_vn(&commitment_substate_address.address)
        .await?;
    let output = output.into_unclaimed_confidential_output().unwrap();
    let unmasked_output = sdk.confidential_crypto_api().unblind_output(
        &output.commitment,
        &output.encrypted_value,
        &account_secret_key.k,
        &reciprocal_claim_public_key,
    )?;

    let (mask, output_public_nonce) = sdk
        .confidential_crypto_api()
        .derive_output_mask_for_destination(&account_public_key);

    let output_statement = ConfidentialProofStatement {
        amount: Amount::try_from(unmasked_output.value)? - fee,
        mask,
        sender_public_nonce: Some(output_public_nonce),
        minimum_value_promise: 0,
        reveal_amount: req.fee,
    };

    let reveal_proof =
        sdk.confidential_crypto_api()
            .generate_withdraw_proof(&[unmasked_output], &output_statement, None)?;

    let inputs = inputs
        .into_iter()
        .map(|s| ShardId::from_address(&s.address, s.version))
        .collect();

    let transaction = Transaction::builder()
        .with_fee_instructions(vec![
            Instruction::ClaimBurn {
                claim: Box::new(
                    ConfidentialClaim {
                        public_key: reciprocal_claim_public_key,
                        output_address: commitment_substate_address.address.as_unclaimed_confidential_output_address().unwrap(),
                        range_proof,
                        proof_of_knowledge: RistrettoComSig::new(Commitment::from_public_key(&public_nonce), u, v),
                        withdraw_proof: Some(reveal_proof),
                    }
                )
            },
            Instruction::PutLastInstructionOutputOnWorkspace {key: b"burn".to_vec()},
            Instruction::CallMethod {
                component_address: account_address,
                method: "deposit".to_string(),
                args: args![Workspace("burn")]
            },
            Instruction::CallMethod {
                component_address: account_address,
                method: "pay_fee".to_string(),
                args: args![req.fee]
            }
        ])
        .with_inputs(inputs)
        .with_outputs(outputs)
        // transaction should have one output, corresponding to the same shard
        // as the account substate address
        // TODO: on a second claim burn, we shouldn't have any new outputs being created.
        .with_new_outputs(1)
        .sign(&account_secret_key.k)
        .build();

    let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;

    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent { hash: tx_hash });

    let finalized = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = finalized.finalize.result.reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.transaction_failure {
        return Err(anyhow::anyhow!(
            "Fee transaction succeeded (fees charged) however the transaction failed: {}",
            reason
        ));
    }

    Ok(ClaimBurnResponse {
        hash: tx_hash,
        fee: finalized.final_fee,
        result: finalized.finalize,
    })
}

#[allow(clippy::too_many_lines)]
pub async fn handle_confidential_transfer(
    context: &HandlerContext,
    req: ConfidentialTransferRequest,
) -> Result<ConfidentialTransferResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    let notifier = context.notifier().clone();

    task::spawn(async move {
        let outputs_api = sdk.confidential_outputs_api();
        let crypto_api = sdk.confidential_crypto_api();
        let accounts_api = sdk.accounts_api();
        let substate_api = sdk.substate_api();
        let key_manager_api = sdk.key_manager_api();

        // -------------------------------- Load up known substates -------------------------------- //
        let account = accounts_api.get_account(&req.account.into())?;
        let account_secret = key_manager_api.derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
        let source_component_address = account
            .address
            .as_component_address()
            .ok_or_else(|| anyhow!("Invalid component address for source address"))?;

        let account_substate = substate_api.get_substate(&account.address)?;
        let src_vault = accounts_api.get_vault_by_resource(&account.address, &CONFIDENTIAL_TARI_RESOURCE_ADDRESS)?;
        let src_vault_substate = substate_api.get_substate(&src_vault.address)?;

        info!(target: LOG_TARGET, "Scanning for account: {}", req.destination_account);
        // TODO: Scan for account and determine the dest vault for a particular resource
        //       For now we assume we already know it
        let dest_dependent_states = substate_api.load_dependent_substates(&[req.destination_account.into()])?;

        // -------------------------------- Lock outputs for spending -------------------------------- //
        let total_amount = req.fee + req.amount;
        let proof_id = outputs_api.add_proof(&src_vault.address)?;
        let (inputs, total_input_value) =
            outputs_api.lock_outputs_by_amount(&src_vault.address, total_amount.as_u64_checked().unwrap(), proof_id)?;

        let (output_mask, public_nonce) = crypto_api.derive_output_mask_for_destination(&req.destination_public_key);

        let output_statement = ConfidentialProofStatement {
            amount: req.amount,
            mask: output_mask,
            sender_public_nonce: Some(public_nonce),
            minimum_value_promise: 0,
            reveal_amount: Amount::zero(),
        };

        let change_amount = total_input_value - req.amount.as_u64_checked().unwrap();
        let change_key = sdk.key_manager_api().next_key(key_manager::TRANSACTION_BRANCH)?;
        outputs_api.add_output(ConfidentialOutputModel {
            account_address: account.address,
            vault_address: src_vault.address,
            commitment: get_commitment_factory().commit_value(&change_key.k, change_amount),
            value: change_amount,
            sender_public_nonce: None,
            secret_key_index: change_key.key_index,
            public_asset_tag: None,
            status: OutputStatus::LockedUnconfirmed,
            locked_by_proof: Some(proof_id),
        })?;

        let change_statement = ConfidentialProofStatement {
            amount: change_amount.try_into()?,
            mask: change_key.k,
            sender_public_nonce: None,
            minimum_value_promise: 0,
            reveal_amount: Amount::zero(),
        };

        let inputs = outputs_api.resolve_output_masks(inputs, key_manager::TRANSACTION_BRANCH)?;

        let proof = crypto_api.generate_withdraw_proof(&inputs, &output_statement, Some(&change_statement))?;

        let mut shard_inputs = vec![
            // Source account input
            ShardId::from_address(&account_substate.address.address, account_substate.address.version),
            ShardId::from_address(&src_vault_substate.address.address, src_vault_substate.address.version),
        ];
        let mut shard_outputs = vec![
            // Source account mutated
            ShardId::from_address(&account_substate.address.address, account_substate.address.version + 1),
            ShardId::from_address(
                &src_vault_substate.address.address,
                src_vault_substate.address.version + 1,
            ),
        ];

        // Dest account and all vaults - TODO: Only pledge the vault that is going to be mutated
        shard_inputs.extend(
            dest_dependent_states
                .iter()
                .map(|s| ShardId::from_address(&s.address, s.version)),
        );
        shard_outputs.extend(
            dest_dependent_states
                .iter()
                .map(|s| ShardId::from_address(&s.address, s.version + 1)),
        );

        let transaction = Transaction::builder()
            .fee_transaction_pay_from_component(source_component_address, req.fee)
            .call_method(source_component_address, "withdraw_confidential", args![
                req.resource_address,
                proof
            ])
            .put_last_instruction_output_on_workspace(b"bucket")
            .call_method(req.destination_account, "deposit", args![Variable("bucket")])
            .with_inputs(shard_inputs)
            .with_outputs(shard_outputs)
            // Possible new dest vault
            .with_new_outputs(1)
            .sign(&account_secret.k)
            .build();

        outputs_api.proofs_set_transaction_hash(proof_id, *transaction.hash())?;

        let tx_hash = sdk.transaction_api().submit_to_vn(transaction).await?;

        let mut events = notifier.subscribe();
        notifier.notify(TransactionSubmittedEvent { hash: tx_hash });

        let finalized = wait_for_result(&mut events, tx_hash).await?;
        if let Some(reject) = finalized.finalize.result.reject() {
            return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
        }
        if let Some(reason) = finalized.transaction_failure {
            return Err(anyhow::anyhow!(
                "Fee transaction succeeded (fees charged) however the transaction failed: {}",
                reason
            ));
        }

        Ok(ConfidentialTransferResponse {
            hash: tx_hash,
            fee: finalized.final_fee,
            result: finalized.finalize,
        })
    })
    .await?
}

async fn wait_for_result(
    events: &mut broadcast::Receiver<WalletEvent>,
    tx_hash: FixedHash,
) -> Result<TransactionFinalizedEvent, anyhow::Error> {
    loop {
        let wallet_event = events.recv().await?;
        match wallet_event {
            WalletEvent::TransactionFinalized(event) if event.hash == tx_hash => return Ok(event),
            _ => {},
        }
    }
}

fn invalid_params(field: &str, details: Option<String>) -> JsonRpcError {
    JsonRpcError::new(
        JsonRpcErrorReason::InvalidParams,
        format!(
            "Invalid param '{}'{}",
            field,
            details.map(|d| format!(": {}", d)).unwrap_or_default()
        ),
        serde_json::Value::Null,
    )
}
