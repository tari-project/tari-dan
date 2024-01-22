//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::convert::TryInto;

use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use log::*;
use rand::rngs::OsRng;
use serde_json::json;
use tari_common_types::types::PublicKey;
use tari_crypto::{commitment::HomomorphicCommitmentFactory, keys::PublicKey as _};
use tari_dan_wallet_sdk::{
    apis::{jwt::JrpcPermission, key_manager},
    confidential::{get_commitment_factory, ConfidentialProofStatement},
    models::{ConfidentialOutputModel, OutputStatus},
};
use tari_template_lib::models::Amount;
use tari_wallet_daemon_client::types::{
    ConfidentialCreateOutputProofRequest,
    ConfidentialCreateOutputProofResponse,
    ProofsCancelRequest,
    ProofsCancelResponse,
    ProofsGenerateRequest,
    ProofsGenerateResponse,
};

use crate::handlers::{
    helpers::{get_account_or_default, invalid_params},
    HandlerContext,
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::json_rpc::confidential";

pub async fn handle_create_transfer_proof(
    context: &HandlerContext,
    token: Option<String>,
    req: ProofsGenerateRequest,
) -> Result<ProofsGenerateResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    if req.amount.is_negative() || req.reveal_amount.is_negative() {
        return Err(JsonRpcError::new(
            JsonRpcErrorReason::InvalidRequest,
            format!(
                "Amount to send must be positive. Amount = {}, Revealed = {}",
                req.amount, req.reveal_amount
            ),
            json!({}),
        )
        .into());
    }

    let account = get_account_or_default(req.account, &sdk.accounts_api())?;
    let vault = sdk
        .accounts_api()
        .get_vault_by_resource(&account.address, &req.resource_address)?;
    let proof_id = sdk.confidential_outputs_api().add_proof(&vault.address)?;
    // Lock inputs we're going to spend
    let (inputs, total_input_value) = sdk.confidential_outputs_api().lock_outputs_by_amount(
        &vault.address,
        req.amount + req.reveal_amount,
        proof_id,
        false,
    )?;

    info!(
        target: LOG_TARGET,
        "Locked {} inputs for proof {} worth {} µT",
        inputs.len(),
        proof_id,
        total_input_value
    );

    // TODO: Any errors from here need to unlock the outputs, ideally just roll back (refactor required but doable).

    // TODO: Wrap up key/encrypted data handling in the wallet SDK
    let account_secret = sdk
        .key_manager_api()
        .derive_key(key_manager::TRANSACTION_BRANCH, account.key_index)?;
    let output_mask = sdk.key_manager_api().next_key(key_manager::TRANSACTION_BRANCH)?;
    let (_, public_nonce) = PublicKey::random_keypair(&mut OsRng);

    let encrypted_data = sdk.confidential_crypto_api().encrypt_value_and_mask(
        req.amount.as_u64_checked().unwrap(),
        &output_mask.key,
        &public_nonce,
        &account_secret.key,
    )?;

    let output_statement = ConfidentialProofStatement {
        amount: req.amount,
        mask: output_mask.key,
        sender_public_nonce: public_nonce,
        minimum_value_promise: 0,
        encrypted_data,
        reveal_amount: req.reveal_amount,
    };

    let change_amount = total_input_value - req.amount.value() as u64 - req.reveal_amount.value() as u64;
    let maybe_change_statement = if change_amount > 0 {
        let change_mask = sdk.key_manager_api().next_key(key_manager::TRANSACTION_BRANCH)?;
        let (_, public_nonce) = PublicKey::random_keypair(&mut OsRng);

        let encrypted_data = sdk.confidential_crypto_api().encrypt_value_and_mask(
            change_amount,
            &change_mask.key,
            &public_nonce,
            &change_mask.key,
        )?;

        sdk.confidential_outputs_api().add_output(ConfidentialOutputModel {
            account_address: account.address,
            vault_address: vault.address,
            commitment: get_commitment_factory().commit_value(&change_mask.key, change_amount),
            value: change_amount,
            sender_public_nonce: Some(public_nonce.clone()),
            encryption_secret_key_index: change_mask.key_index,
            encrypted_data: encrypted_data.clone(),
            public_asset_tag: None,
            status: OutputStatus::LockedUnconfirmed,
            locked_by_proof: Some(proof_id),
        })?;

        Some(ConfidentialProofStatement {
            amount: change_amount.try_into()?,
            mask: change_mask.key,
            sender_public_nonce: public_nonce,
            encrypted_data,
            minimum_value_promise: 0,
            reveal_amount: Amount::zero(),
        })
    } else {
        None
    };

    let inputs = sdk
        .confidential_outputs_api()
        .resolve_output_masks(inputs, key_manager::TRANSACTION_BRANCH)?;

    let proof = sdk.confidential_crypto_api().generate_withdraw_proof(
        &inputs,
        &output_statement,
        maybe_change_statement.as_ref(),
    )?;

    Ok(ProofsGenerateResponse { proof_id, proof })
}

pub async fn handle_finalize_transfer(
    context: &HandlerContext,
    token: Option<String>,
    req: ProofsCancelRequest,
) -> Result<ProofsCancelResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    sdk.confidential_outputs_api()
        .finalize_outputs_for_proof(req.proof_id)?;
    Ok(ProofsCancelResponse {})
}

pub async fn handle_cancel_transfer(
    context: &HandlerContext,
    token: Option<String>,
    req: ProofsCancelRequest,
) -> Result<ProofsCancelResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;
    sdk.confidential_outputs_api().release_proof_outputs(req.proof_id)?;
    Ok(ProofsCancelResponse {})
}

pub async fn handle_create_output_proof(
    context: &HandlerContext,
    token: Option<String>,
    req: ConfidentialCreateOutputProofRequest,
) -> Result<ConfidentialCreateOutputProofResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    if req.amount.is_negative() {
        return Err(invalid_params("amount", Some("must be positive")));
    }

    let output_mask = sdk.key_manager_api().next_key(key_manager::TRANSACTION_BRANCH)?;
    let (_, public_nonce) = PublicKey::random_keypair(&mut OsRng);
    let encrypted_data = sdk.confidential_crypto_api().encrypt_value_and_mask(
        req.amount.as_u64_checked().unwrap(),
        &output_mask.key,
        &public_nonce,
        &output_mask.key,
    )?;

    let statement = ConfidentialProofStatement {
        amount: req.amount,
        mask: output_mask.key,
        sender_public_nonce: public_nonce,
        minimum_value_promise: 0,
        encrypted_data,
        reveal_amount: Amount::zero(),
    };
    let proof = sdk.confidential_crypto_api().generate_output_proof(&statement)?;
    Ok(ConfidentialCreateOutputProofResponse { proof })
}
