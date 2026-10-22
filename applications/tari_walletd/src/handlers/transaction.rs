//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::{collections::HashSet, time::Duration};

use anyhow::anyhow;
use axum_extra::headers::authorization::Bearer;
use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use futures::{future, future::Either};
use indexmap::IndexSet;
use log::*;
use ootle_byte_type::ToByteType;
use tari_ootle_common_types::{
    Epoch,
    InputDeclaration,
    SubstateRequirement,
    optional::Optional,
    response_status::ResponseErrorStatus,
};
use tari_ootle_transaction::{Transaction, TransactionSignature, UnsignedTransaction, args};
use tari_ootle_wallet_sdk::{
    apis::transaction::TransactionApiError,
    models::{
        KeyId,
        OutputStatus,
        StealthUtxoSpendKeyId,
        TransactionContext,
        TransactionStatus,
        WalletEvent,
        WalletLockId,
    },
};
use tari_ootle_wallet_sdk_services::transaction_service::TransactionServiceError;
use tari_ootle_walletd_client::{
    permissions::{Crud, Permission},
    types::{
        CallInstructionRequest,
        PublishTemplateRequest,
        PublishTemplateResponse,
        TransactionDetectInputsRequest,
        TransactionDetectInputsResponse,
        TransactionGetAllRequest,
        TransactionGetAllResponse,
        TransactionGetRequest,
        TransactionGetResponse,
        TransactionGetResultRequest,
        TransactionGetResultResponse,
        TransactionSubmitDryRunRequest,
        TransactionSubmitDryRunResponse,
        TransactionSubmitManifestRequest,
        TransactionSubmitManifestResponse,
        TransactionSubmitRequest,
        TransactionSubmitResponse,
        TransactionWaitResultRequest,
        TransactionWaitResultResponse,
    },
};
use tari_template_lib_types::ComponentAddress;
use tari_transaction_manifest::{ManifestInstructions, parse_manifest};
use tokio::time;

use super::context::HandlerContext;
use crate::{
    WalletSdk,
    handlers::helpers::{
        OrJrpcNotFound,
        ensure_signing_key,
        ensure_transaction_signers,
        get_account,
        get_account_or_default,
        invalid_params,
        invalid_request,
        not_found,
        transaction_rejected,
    },
    services::wasm_optimizer::optimize_wasm_template,
};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::handlers::transaction";

pub async fn handle_submit_instruction(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: CallInstructionRequest,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    // Arbitrary instructions can touch any account the wallet owns — the
    // fee account is only the fee payer, not a scope boundary. Require the
    // broad `transactions:create` grant.
    context.authorize(token, &[Permission::Transactions(Crud::Create)])?;
    context.enforce_max_fee_limit(req.max_fee)?;
    let sdk = context.wallet_sdk();

    let fee_account = get_account(&req.fee_account, &sdk.accounts_api())?;
    let owner_key_id = fee_account.owner_key_id().ok_or_else(|| {
        invalid_params(
            "fee_account",
            Some("Fee account does not have an owner key set".to_string()),
        )
    })?;
    let transaction = context
        .transaction_builder()
        .await?
        .with_instructions(req.instructions)
        .pay_fee_from_component(*fee_account.component_address(), req.max_fee.max(1))
        .with_min_epoch(req.min_epoch.map(Epoch))
        .then(|b| match req.max_epoch {
            Some(max_epoch) => b.with_max_epoch(Epoch(max_epoch)),
            None => b,
        })
        .with_inputs(req.inputs)
        .build_unsigned();

    let request = TransactionSubmitRequest {
        transaction,
        seal_signer: owner_key_id,
        other_signers: vec![],
        signatures: vec![],
        detect_inputs: req.override_inputs.unwrap_or_default(),
        detect_inputs_use_unversioned: true,
        lock_ids: vec![],
    };
    submit_inner(context, request, vec![*fee_account.component_address()]).await
}

pub async fn handle_submit(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionSubmitRequest,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    // Direct submission accepts any pre-built transaction; the wallet
    // cannot constrain which accounts it touches. Callers that already
    // hold a narrower capability (transfer family, publish_template) call
    // `submit_inner` directly with their own scoped check upstream.
    context.authorize(token, &[Permission::Transactions(Crud::Create)])?;
    // Best-effort: tag the transaction with the account that seals (pays for) it, so it can be filtered
    // per account. The seal signer may not map to a known account (e.g. an imported key) — that is fine.
    let linked_accounts = get_account_for_key(context.wallet_sdk(), req.seal_signer)
        .map(|address| vec![address])
        .unwrap_or_default();
    submit_inner(context, req, linked_accounts).await
}

/// Seal and submit an approved transaction request.
///
/// The request's own handler has already checked the approval and passes
/// `detect_inputs: false`, so nothing here alters the frozen transaction.
pub(crate) async fn submit_inner_for_request(
    context: &HandlerContext,
    req: TransactionSubmitRequest,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let linked_accounts = get_account_for_key(context.wallet_sdk(), req.seal_signer)
        .map(|address| vec![address])
        .unwrap_or_default();
    submit_inner(context, req, linked_accounts).await
}

async fn submit_inner(
    context: &HandlerContext,
    req: TransactionSubmitRequest,
    linked_accounts: Vec<ComponentAddress>,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    req.transaction
        .validate_blob_references()
        .map_err(|e| invalid_params("transaction.blobs", Some(e.to_string())))?;
    ensure_transaction_signers(&req.seal_signer, &req.other_signers)?;
    reject_detect_inputs_with_signatures(req.detect_inputs, &req.signatures)?;
    let detected_inputs = if req.detect_inputs {
        // If we are not overriding inputs, we will use inputs that we know about in the local substate id db
        let substates = req.transaction.to_referenced_substates()?;
        let substates = substates
            .into_iter()
            .chain(
                req.transaction
                    .inputs()
                    .into_iter()
                    .map(|req| req.substate_id().clone()),
            )
            .collect::<Vec<_>>();
        let loaded_substates = sdk
            .substate_api()
            .locate_dependent_substates(&substates, req.detect_inputs_use_unversioned)
            .await
            .or_jrpc_not_found()?;
        let declared = req.transaction.inputs().clone();
        loaded_substates
            .into_iter()
            .map(|input| declare_detected_input(input, &declared, req.detect_inputs_use_unversioned))
            .collect()
    } else {
        vec![]
    };

    info!(
        target: LOG_TARGET,
        "Detected {} input(s) (detect_inputs = {}, detect_inputs_use_unversioned = {})",
        detected_inputs.len(),
        req.detect_inputs,
        req.detect_inputs_use_unversioned,
    );

    // Signatures collected out of band are attached first; walletd's own
    // signatures are added on top and the seal signature commits to all of them.
    let mut transaction = context
        .transaction_builder_from_unsigned(req.transaction)
        .with_inputs(detected_inputs)
        .with_signatures(req.signatures);

    // Stealth inputs are spent with a key derived per-UTXO from the sender's
    // account key and that UTXO's nonce. Derive the set from the locks rather
    // than accepting it from the caller: the locked outputs are what this
    // transaction actually spends, so a caller cannot ask the wallet to sign
    // with a key of its choosing.
    let stealth_signers = derive_stealth_signers(sdk, &req.lock_ids)?;

    if !req.other_signers.is_empty() || !stealth_signers.is_empty() {
        let seal_signer = sdk.key_manager_api().get_public_key(req.seal_signer)?;
        let seal_signer_pk = seal_signer.public_key.to_byte_type();
        let local_signer = sdk.signer_api().with_context(&seal_signer_pk);
        for key in req.other_signers {
            transaction = local_signer.sign(key, transaction)?;
        }
        for key in &stealth_signers {
            transaction = local_signer.sign_with_stealth_key(key, transaction)?;
        }
    }

    let transaction = sdk.signer_api().sign(req.seal_signer, transaction)?;

    let tx_id = transaction.calculate_id();
    for lock_id in req.lock_ids {
        // update the locks with the corresponding transaction ID so that they can be finalized/released once the
        // transaction resolves
        sdk.transaction_api().locks_set_transaction_id(lock_id, tx_id)?;
    }

    info!(
        target: LOG_TARGET,
        "Submitted transaction with hash {}",
        transaction.calculate_id()
    );

    // Link the transaction to the involved account(s) (if any are known) so the list can be filtered per
    // account.
    let tx_context = (!linked_accounts.is_empty()).then(|| TransactionContext::with_accounts(linked_accounts));
    let transaction_id = context
        .transaction_service()
        .submit_transaction_with_opts(transaction, tx_context, None)
        .await
        .map_err(map_transaction_submission_error)?;

    Ok(TransactionSubmitResponse { transaction_id })
}

/// Signatures collected out of band are made over the transaction body as submitted, and detecting inputs adds to
/// that body, so a request cannot ask for both.
fn reject_detect_inputs_with_signatures(
    detect_inputs: bool,
    signatures: &[TransactionSignature],
) -> Result<(), anyhow::Error> {
    if detect_inputs && !signatures.is_empty() {
        return Err(invalid_params(
            "detect_inputs",
            Some(
                "detect_inputs changes the transaction body after the provided signatures were made. Detect inputs \
                 before collecting signatures (transactions.detect_inputs) and submit with detect_inputs=false",
            ),
        ));
    }
    Ok(())
}

/// The stealth spend keys needed to spend everything the given locks hold.
///
/// Derived from the wallet's own record of what each lock locked, never from
/// the caller. `create_stealth_transfer_statement` returns an equivalent list
/// to its caller, but a caller that could hand that list back would be naming
/// the keys the wallet signs with; re-deriving here means the locked outputs
/// are the only thing that decides.
///
/// A lock holding no stealth outputs (a revealed-only transfer, say) yields an
/// empty set rather than an error.
fn derive_stealth_signers(
    sdk: &WalletSdk,
    lock_ids: &[WalletLockId],
) -> Result<Vec<StealthUtxoSpendKeyId>, anyhow::Error> {
    let mut signers = IndexSet::new();

    for lock_id in lock_ids {
        let Some(outputs) = sdk.stealth_outputs_api().get_locked_by_lock_id(*lock_id).optional()? else {
            continue;
        };

        for output in outputs {
            // Only inputs being spent need a spend signature. The same lock
            // also holds the change outputs it created (`LockedUnconfirmed`),
            // which are not being spent by this transaction.
            if !matches!(output.status, OutputStatus::LockedForSpend) {
                continue;
            }
            // A view-only output has no spend key: the wallet can see it but
            // cannot spend it, so a transaction that tries is malformed.
            let account_key_id = output.owner_key_id.ok_or_else(|| {
                invalid_request(format!(
                    "Cannot spend view-only stealth output {} (lock {lock_id}): no owner key",
                    output.commitment
                ))
            })?;
            signers.insert(StealthUtxoSpendKeyId {
                account_key_id,
                public_nonce: output.sender_public_nonce,
            });
        }
    }

    Ok(signers.into_iter().collect())
}

/// Parses the request's variables, blobs and manifest source into instructions, mapping parse
/// failures to invalid-params errors.
fn parse_manifest_request(req: &TransactionSubmitManifestRequest) -> Result<ManifestInstructions, anyhow::Error> {
    let variables = req
        .variables
        .iter()
        .map(|(name, value)| {
            value.parse().map(|value| (name.to_string(), value)).map_err(|err| {
                invalid_params(
                    "variables",
                    Some(format!("Failed to parse variable '{}': {}", name, err)),
                )
            })
        })
        .collect::<Result<_, _>>()?;

    let blob_inputs = req
        .blobs
        .iter()
        .map(|(name, blob)| (name.clone(), blob.clone()))
        .collect();
    parse_manifest(&req.manifest, variables, Default::default(), blob_inputs)
        .map_err(|e| invalid_params("manifest", Some(format!("Failed to parse manifest: {}", e))))
}

/// Best-effort: resolves the wallet account whose owner public key is `key_id`. Returns `None` if the
/// key or a matching account is unknown (e.g. an imported key).
fn get_account_for_key(sdk: &WalletSdk, key_id: KeyId) -> Option<ComponentAddress> {
    let pk = sdk.key_manager_api().get_public_key(key_id).ok()?;
    sdk.accounts_api()
        .get_account_by_public_key(&pk.public_key.to_byte_type())
        .optional()
        .ok()
        .flatten()
        .map(|account| account.account.component_address)
}

/// Builds a [`TransactionContext`] linking the fee payer account (if any) and the seal signer's
/// account (if it maps to one of the wallet's accounts). Returns `None` if neither is known.
fn build_transaction_context(
    sdk: &WalletSdk,
    fee_payer: Option<ComponentAddress>,
    seal_signer_key_id: KeyId,
) -> Option<TransactionContext> {
    let mut linked_accounts = Vec::from_iter(fee_payer);
    if let Some(address) =
        get_account_for_key(sdk, seal_signer_key_id).filter(|address| !linked_accounts.contains(address))
    {
        linked_accounts.push(address);
    }
    (!linked_accounts.is_empty()).then(|| TransactionContext::with_accounts(linked_accounts))
}

fn map_transaction_submission_error(e: TransactionServiceError) -> anyhow::Error {
    error!(target: LOG_TARGET, "Transaction submission failed: {}", e);
    match &e {
        TransactionServiceError::TransactionApiError(TransactionApiError::NetworkInterfaceError { status, .. }) => {
            match &status {
                ResponseErrorStatus::TransactionRejected { message } => transaction_rejected(message),
                ResponseErrorStatus::NotFound { message } => not_found(message),
                ResponseErrorStatus::InternalError { message } => anyhow!("Failed to submit transaction: {}", message),
            }
        },
        _ => anyhow!("Failed to submit transaction: {}", e),
    }
}

/// Resolve a transaction's inputs without submitting anything, so a caller that
/// cannot detect them itself can build a complete transaction before requesting
/// or signing. Returns the transaction with the detected inputs merged in.
pub async fn handle_detect_inputs(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionDetectInputsRequest,
) -> Result<TransactionDetectInputsResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Transactions(Crud::Read)])?;
    let sdk = context.wallet_sdk();

    let substates = req.transaction.to_referenced_substates()?;
    let substates = substates
        .into_iter()
        .chain(req.transaction.inputs().iter().map(|r| r.substate_id().clone()))
        .collect::<Vec<_>>();
    let detected = sdk
        .substate_api()
        .locate_dependent_substates(&substates, req.use_unversioned)
        .await
        .or_jrpc_not_found()?
        .into_iter()
        .map(|input| declare_detected_input(input, req.transaction.inputs(), req.use_unversioned))
        .collect::<Vec<_>>();

    let transaction = context
        .transaction_builder_from_unsigned(req.transaction)
        .with_inputs(detected)
        .build_unsigned();

    Ok(TransactionDetectInputsResponse { transaction })
}

pub async fn handle_submit_dry_run(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionSubmitDryRunRequest,
) -> Result<TransactionSubmitDryRunResponse, anyhow::Error> {
    // Dry-run doesn't change state but uses the submission machinery for
    // an arbitrary transaction. Gated on `transactions:read`.
    context.authorize(token, &[Permission::Transactions(Crud::Read)])?;
    submit_dry_run_inner(context, req).await
}

async fn submit_dry_run_inner(
    context: &HandlerContext,
    req: TransactionSubmitDryRunRequest,
) -> Result<TransactionSubmitDryRunResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    req.transaction
        .validate_blob_references()
        .map_err(|e| invalid_params("transaction.blobs", Some(e.to_string())))?;
    ensure_transaction_signers(&req.seal_signer, &req.other_signers)?;
    reject_detect_inputs_with_signatures(req.detect_inputs, &req.signatures)?;

    let detected_inputs = if req.detect_inputs {
        // If we are not overriding inputs, we will use inputs that we know about in the local substate id db
        let substates = req.transaction.to_referenced_substates()?;
        let substates = substates.into_iter().collect::<Vec<_>>();
        let dependencies = sdk
            .substate_api()
            .locate_dependent_substates(&substates, req.detect_inputs_use_unversioned)
            .await
            .or_jrpc_not_found()?;
        let declared = req.transaction.inputs().clone();
        dependencies
            .into_iter()
            .map(|input| declare_detected_input(input, &declared, req.detect_inputs_use_unversioned))
            .collect()
    } else {
        vec![]
    };

    let mut transaction = context
        .transaction_builder_from_unsigned(req.transaction)
        .with_inputs(detected_inputs)
        .with_dry_run(true)
        .with_signatures(req.signatures);

    // Sign exactly as a real submission would, so the dry run reflects a
    // spendable transaction: stealth inputs are signed with keys derived from
    // the locks, and the seal signature is added last.
    let stealth_signers = derive_stealth_signers(sdk, &req.lock_ids)?;
    if !req.other_signers.is_empty() || !stealth_signers.is_empty() {
        let seal_signer = sdk.key_manager_api().get_public_key(req.seal_signer)?;
        let seal_signer_pk = seal_signer.public_key.to_byte_type();
        let local_signer = sdk.signer_api().with_context(&seal_signer_pk);
        for key in req.other_signers {
            transaction = local_signer.sign(key, transaction)?;
        }
        for key in &stealth_signers {
            transaction = local_signer.sign_with_stealth_key(key, transaction)?;
        }
    }
    let transaction = sdk.signer_api().sign(req.seal_signer, transaction)?;

    // A dry run never finalizes, so it must not link the locks: a lock tied to a
    // transaction that never resolves would never be released.
    info!(
        target: LOG_TARGET,
        "Submitted dry-run transaction {}",
        transaction.calculate_id()
    );
    let exec_result = context
        .transaction_service()
        .submit_dry_run_transaction(transaction)
        .await
        .map_err(map_transaction_submission_error)?;

    let required_fees = exec_result.finalize.required_fees();
    Ok(TransactionSubmitDryRunResponse {
        transaction_id: exec_result.finalize.transaction_hash.into_array().into(),
        required_fees,
        result: exec_result,
    })
}

pub async fn handle_submit_manifest(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionSubmitManifestRequest,
) -> Result<TransactionSubmitManifestResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();

    // Manifests carry arbitrary instructions; the default account is the
    // implicit fee payer but instructions can touch any account. Require
    // the broad `transactions:create` grant.
    context.authorize(token, &[Permission::Transactions(Crud::Create)])?;
    if !req.dry_run {
        context.enforce_max_fee_limit(req.max_fee)?;
    }
    let default_account = sdk
        .accounts_api()
        .get_default()
        .optional()?
        .ok_or_else(|| invalid_request("No default account found".to_string()))?;

    let instructions = parse_manifest_request(&req)?;

    let default_owner_key_id = default_account.owner_key_id().ok_or_else(|| {
        invalid_params(
            "seal_signer_key_id",
            Some("Default account does not have an owner key set".to_string()),
        )
    })?;

    let seal_signer_key_id = req.seal_signer_key_id.unwrap_or(default_owner_key_id);
    ensure_signing_key("seal_signer_key_id", &seal_signer_key_id)?;
    req.signing_key_ids
        .iter()
        .try_for_each(|key_id| ensure_signing_key("signing_key_ids", key_id))?;

    // Fee of zero is never valid
    let fee_amount = req.max_fee.max(1);

    // The default account is the implicit fee payer when the manifest has no fee instructions.
    let fee_payer = instructions
        .fee_instructions
        .is_empty()
        .then(|| *default_account.component_address());

    let mut transaction = context
        .transaction_builder()
        .await?
        .with_dry_run(req.dry_run)
        .with_fee_instructions_builder(|builder| {
            if instructions.fee_instructions.is_empty() {
                builder.call_method(*default_account.component_address(), "pay_fee", args![fee_amount])
            } else {
                builder.with_instructions(instructions.fee_instructions)
            }
        })
        .with_instructions(instructions.instructions)
        .build_unsigned();

    // Attach blobs from the manifest (in the order they were first referenced) to the
    // unsigned transaction. The manifest generator already assigned `BlobIndex`es matching
    // this ordering, so the indices in instructions resolve correctly.
    for blob in instructions.blobs.iter().cloned() {
        transaction
            .add_blob(blob)
            .map_err(|e| invalid_params("blobs", Some(e.to_string())))?;
    }

    // Detect inputs
    let substates = transaction.to_referenced_substates()?.into_iter().collect::<Vec<_>>();
    let dependencies = sdk
        .substate_api()
        .locate_dependent_substates(&substates, true)
        .await
        .or_jrpc_not_found()?;
    let inputs = dependencies
        .into_iter()
        .map(|input| InputDeclaration::write(input.into_substate_id()));

    let unsigned = transaction.with_inputs(inputs);

    let sign = |unsigned: UnsignedTransaction| {
        sign_manifest_transaction(sdk, unsigned, seal_signer_key_id, &req.signing_key_ids)
    };
    if req.dry_run {
        let exec_result = context
            .transaction_service()
            .submit_dry_run_transaction(sign(unsigned)?)
            .await?;

        if let Some(reject) = exec_result.finalize.any_reject() {
            return Err(JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(5),
                format!("Dry-run transaction rejected: {reject}"),
                serde_json::Value::Null,
            )
            .into());
        }
        let required_fees = exec_result.finalize.required_fees();
        return Ok(TransactionSubmitManifestResponse {
            transaction_id: exec_result.finalize.transaction_hash.into_array().into(),
            required_fees: Some(required_fees),
            result: Some(exec_result),
        });
    }

    let transaction = declare_written_inputs_only(context, unsigned, sign).await?;

    // Link the fee payer and the seal signer's account (when it maps to one of the wallet's
    // accounts). Manifest instructions can touch any account, so this is best-effort.
    let tx_context = build_transaction_context(sdk, fee_payer, seal_signer_key_id);
    let transaction_id = context
        .transaction_service()
        .submit_transaction_with_opts(transaction, tx_context, None)
        .await?;

    Ok(TransactionSubmitManifestResponse {
        transaction_id,
        required_fees: None,
        result: None,
    })
}

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionGetRequest,
) -> Result<TransactionGetResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Transactions(Crud::Read)])?;
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.transaction_id)
        .optional()?
        .ok_or_else(|| not_found(format!("Transaction {}", req.transaction_id)))?;

    Ok(TransactionGetResponse {
        transaction: transaction.transaction,
        result: transaction.finalize,
        status: transaction.status,
        final_fee: transaction.final_fee,
        invalid_reason: transaction.invalid_reason,
        last_update_time: transaction.last_update_time,
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionGetAllRequest,
) -> Result<TransactionGetAllResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Transactions(Crud::Read)])?;
    let transactions = context
        .wallet_sdk()
        .transaction_api()
        .fetch_all(req.status, req.account)?;
    Ok(TransactionGetAllResponse { transactions })
}

pub async fn handle_get_result(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionGetResultRequest,
) -> Result<TransactionGetResultResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Transactions(Crud::Read)])?;
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.transaction_id)
        .optional()?
        .ok_or_else(|| not_found(format!("Transaction {}", req.transaction_id)))?;

    Ok(TransactionGetResultResponse {
        transaction_id: req.transaction_id,
        result: transaction.finalize,
        status: transaction.status,
    })
}

pub async fn handle_wait_result(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransactionWaitResultRequest,
) -> Result<TransactionWaitResultResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Transactions(Crud::Read)])?;
    let mut events = context.notifier().subscribe();
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.transaction_id)
        .optional()?
        .ok_or_else(|| not_found(format!("Transaction {}", req.transaction_id)))?;

    // A transaction rejected before consensus has no finalize result and emits no further events.
    let has_outcome = transaction.finalize.is_some() ||
        matches!(
            transaction.status,
            TransactionStatus::InvalidTransaction | TransactionStatus::DryRunFailed
        );
    if has_outcome {
        return Ok(TransactionWaitResultResponse {
            transaction_id: req.transaction_id,
            result: transaction.finalize,
            status: transaction.status,
            final_fee: transaction.final_fee.unwrap_or_default(),
            timed_out: false,
        });
    }

    let mut timeout = match req.timeout_secs {
        Some(timeout) => Either::Left(Box::pin(time::sleep(Duration::from_secs(timeout)))),
        None => Either::Right(future::pending()),
    };

    loop {
        let evt_or_timeout = tokio::select! {
            biased;
            event = events.recv() => {
                match event {
                    Ok(event) => Some(event),
                    Err(e) => return Err(anyhow!("Unexpected event stream error: {}", e)),
                }
            },
            _ = &mut timeout => None,
        };

        match evt_or_timeout {
            Some(WalletEvent::TransactionFinalized(event)) if event.transaction_id == req.transaction_id => {
                return Ok(TransactionWaitResultResponse {
                    transaction_id: req.transaction_id,
                    result: Some(event.finalize),
                    status: event.status,
                    final_fee: event.final_fee,
                    timed_out: false,
                });
            },
            Some(WalletEvent::TransactionInvalid(event)) if event.transaction_id == req.transaction_id => {
                return Ok(TransactionWaitResultResponse {
                    transaction_id: req.transaction_id,
                    result: event.finalize,
                    status: event.status,
                    final_fee: event.final_fee.unwrap_or_default(),
                    timed_out: false,
                });
            },
            Some(_) => continue,
            None => {
                return Ok(TransactionWaitResultResponse {
                    transaction_id: req.transaction_id,
                    result: None,
                    status: transaction.status,
                    final_fee: 0,
                    timed_out: true,
                });
            },
        };
    }
}

pub async fn handle_publish_template(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: PublishTemplateRequest,
) -> Result<PublishTemplateResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Templates(Crud::Create)])?;
    if !req.dry_run {
        context.enforce_max_fee_limit(req.max_fee)?;
    }
    let sdk = context.wallet_sdk();

    let fee_account = get_account_or_default(req.fee_account.as_ref(), &sdk.accounts_api())?;
    let owner_key_id = fee_account.owner_key_id().ok_or_else(|| {
        invalid_params(
            "fee_account",
            Some("Fee account does not have an owner key set".to_string()),
        )
    })?;

    // A max_fee of zero will fail, since paying zero fees is never valid
    let max_fee = req.max_fee.max(1);

    // trying to optimize WASM binary
    let wasm_binary = match optimize_wasm_template(req.binary.as_slice()).await {
        Ok(optimized) => {
            info!(target: LOG_TARGET, "WASM template optimized, original size: {} bytes, new size: {} bytes", req.binary.len(), optimized.len());
            optimized
        },
        Err(error) => {
            warn!(target: LOG_TARGET, "Error while optimizing WASM template (using original version now): {}", error);
            req.binary
        },
    };

    if wasm_binary.len() > tari_engine_types::limits::ENGINE_LIMITS.max_template_binary_size_bytes {
        return Err(invalid_params("binary", Some("WASM binary too large".to_string())));
    }

    let metadata_hash = req.metadata.map(resolve_metadata_hash).transpose()?;

    let builder = context
        .transaction_builder()
        .await?
        .pay_fee_from_component(*fee_account.component_address(), max_fee);
    let builder = match metadata_hash {
        Some(hash) => builder.publish_template_with_metadata(wasm_binary, hash),
        None => builder.publish_template(wasm_binary),
    };
    let transaction = builder.build_unsigned();

    if req.dry_run {
        let request = TransactionSubmitDryRunRequest {
            transaction,
            seal_signer: owner_key_id,
            other_signers: vec![],
            signatures: vec![],
            detect_inputs: req.detect_inputs,
            detect_inputs_use_unversioned: true,
            lock_ids: vec![],
        };
        let resp = submit_dry_run_inner(context, request).await?;
        if let Some(reject) = resp.result.finalize.any_reject() {
            return Err(JsonRpcError::new(
                JsonRpcErrorReason::ApplicationError(5),
                format!("Dry-run transaction rejected: {reject}"),
                serde_json::Value::Null,
            )
            .into());
        }
        return Ok(PublishTemplateResponse {
            transaction_id: resp.transaction_id,
            dry_run_fee: Some(resp.result.finalize.required_fees()),
        });
    }
    let request = TransactionSubmitRequest {
        transaction,
        seal_signer: owner_key_id,
        other_signers: vec![],
        signatures: vec![],
        detect_inputs: req.detect_inputs,
        detect_inputs_use_unversioned: true,
        lock_ids: vec![],
    };
    let resp = submit_inner(context, request, vec![*fee_account.component_address()]).await?;
    Ok(PublishTemplateResponse {
        transaction_id: resp.transaction_id,
        dry_run_fee: None,
    })
}

/// Declares as reads the inputs that a dry run of `unsigned` does not write, and signs the result.
///
/// Detection cannot tell how a manifest uses each input, so every input starts as a write; the dry run
/// shows which ones it actually writes, and declaring the rest as reads keeps the transaction from
/// serialising with others that only read them. A dry run that fails or is not fully accepted says
/// nothing reliable about the main instructions, so the declarations are then left as writes.
async fn declare_written_inputs_only<F>(
    context: &HandlerContext,
    unsigned: UnsignedTransaction,
    sign: F,
) -> Result<Transaction, anyhow::Error>
where
    F: Fn(UnsignedTransaction) -> Result<Transaction, anyhow::Error>,
{
    let dry_run = context
        .transaction_service()
        .submit_dry_run_transaction(sign(unsigned.clone().with_dry_run(true))?)
        .await;
    match dry_run {
        Ok(result) => match result.finalize.accept() {
            Some(diff) => {
                let written = diff.down_iter().map(|(id, _)| id).collect::<HashSet<_>>();
                sign(unsigned.narrow_to_reads(|decl| !written.contains(decl.substate_id())))
            },
            None => sign(unsigned),
        },
        Err(err) => {
            warn!(target: LOG_TARGET, "Manifest dry run failed, declaring every input a write: {err}");
            sign(unsigned)
        },
    }
}

/// Signs with each additional signing key, then seals with the seal signer.
fn sign_manifest_transaction(
    sdk: &WalletSdk,
    unsigned: UnsignedTransaction,
    seal_signer_key_id: KeyId,
    signing_key_ids: &[KeyId],
) -> Result<Transaction, anyhow::Error> {
    let mut transaction = unsigned.finish();
    if !signing_key_ids.is_empty() {
        let seal_signer_pk = sdk.key_manager_api().get_public_key(seal_signer_key_id)?;
        let seal_signer_pk_bytes = seal_signer_pk.public_key.to_byte_type();
        let signer = sdk.signer_api().with_context(&seal_signer_pk_bytes);
        for signing_key_id in signing_key_ids {
            if *signing_key_id != seal_signer_key_id {
                transaction = signer.sign(*signing_key_id, transaction)?;
            }
        }
    }
    Ok(sdk.signer_api().sign(seal_signer_key_id, transaction)?)
}

/// Turns a detected dependency into a declaration, deferring to what the transaction already said
/// about that substate.
///
/// Input detection walks the substates an instruction reaches and learns nothing about how each one
/// is used, so on its own it can only declare a write. A caller that declared a read knows better,
/// and folding the detected set in would otherwise widen it back to a write and take the caller's
/// read parallelism away.
fn declare_detected_input(
    detected: SubstateRequirement,
    declared: &IndexSet<InputDeclaration>,
    use_unversioned: bool,
) -> InputDeclaration {
    let is_write = declared
        .get(detected.substate_id())
        .is_none_or(|declaration| declaration.is_write());
    let version = if use_unversioned { None } else { detected.version() };
    InputDeclaration::new(detected.into_substate_id(), version, is_write)
}

fn resolve_metadata_hash(
    input: tari_ootle_walletd_client::types::PublishTemplateMetadata,
) -> Result<tari_ootle_template_metadata::MetadataHash, anyhow::Error> {
    use tari_ootle_template_metadata::TemplateMetadata;
    use tari_ootle_walletd_client::types::PublishTemplateMetadata;

    match input {
        PublishTemplateMetadata::Hash(hash) => Ok(hash),
        PublishTemplateMetadata::Literal(meta) => meta.hash().map_err(|e| anyhow!(e)),
        PublishTemplateMetadata::RawCbor(bytes) => {
            let meta = TemplateMetadata::from_cbor(&bytes).map_err(|e| anyhow!(e))?;
            meta.hash().map_err(|e| anyhow!(e))
        },
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_address::Network;
    use tari_ootle_common_types::{SubstateVersion, engine_types::substate::SubstateId};
    use tari_ootle_wallet_sdk::models::KeyBranch;
    use tari_template_lib_types::{
        ComponentAddress,
        crypto::{RistrettoPublicKeyBytes, SchnorrSignatureBytes},
    };

    use super::*;
    use crate::handlers::test_support::TestDaemon;

    fn component(byte: u8) -> SubstateId {
        SubstateId::Component(ComponentAddress::from_array([byte; 32]))
    }

    #[test]
    fn a_detected_input_the_caller_declared_read_stays_a_read() {
        let id = component(1);
        let declared = IndexSet::from([InputDeclaration::read(id.clone())]);

        let decl = declare_detected_input(
            SubstateRequirement::versioned(id, SubstateVersion::new(3)),
            &declared,
            false,
        );

        assert!(decl.is_read());
        assert_eq!(decl.version(), Some(SubstateVersion::new(3)));
    }

    #[test]
    fn a_detected_input_the_caller_did_not_declare_is_a_write() {
        let declared = IndexSet::from([InputDeclaration::read(component(1))]);

        let decl = declare_detected_input(SubstateRequirement::unversioned(component(2)), &declared, false);

        assert!(decl.is_write());
    }

    #[test]
    fn a_detected_input_the_caller_declared_write_stays_a_write() {
        let id = component(1);
        let declared = IndexSet::from([InputDeclaration::write(id.clone())]);

        let decl = declare_detected_input(
            SubstateRequirement::versioned(id, SubstateVersion::new(3)),
            &declared,
            false,
        );

        assert!(decl.is_write());
    }

    #[test]
    fn detecting_inputs_is_refused_when_signatures_are_provided() {
        let signature = TransactionSignature::new(RistrettoPublicKeyBytes::zero(), SchnorrSignatureBytes::zero());

        let signatures = vec![signature];

        assert!(reject_detect_inputs_with_signatures(true, &signatures).is_err());
        assert!(reject_detect_inputs_with_signatures(false, &signatures).is_ok());
        assert!(reject_detect_inputs_with_signatures(true, &[]).is_ok());
    }

    #[test]
    fn unversioned_detection_drops_the_version_it_found() {
        let id = component(1);
        let declared = IndexSet::from([InputDeclaration::read(id.clone())]);

        let decl = declare_detected_input(
            SubstateRequirement::versioned(id, SubstateVersion::new(3)),
            &declared,
            true,
        );

        assert_eq!(decl.version(), None);
        assert!(decl.is_read());
    }

    fn submit_request(seal_signer: KeyId, other_signers: Vec<KeyId>) -> TransactionSubmitRequest {
        TransactionSubmitRequest {
            transaction: UnsignedTransaction::new(Network::LocalNet.as_byte(), Epoch(100)),
            seal_signer,
            other_signers,
            signatures: vec![],
            detect_inputs: false,
            detect_inputs_use_unversioned: true,
            lock_ids: vec![],
        }
    }

    fn assert_invalid_params(err: &anyhow::Error, field: &str) {
        let rpc = err
            .downcast_ref::<JsonRpcError>()
            .unwrap_or_else(|| panic!("expected a JsonRpcError, got: {err}"));
        assert!(
            matches!(rpc.error_reason(), JsonRpcErrorReason::InvalidParams),
            "expected InvalidParams, got: {rpc}"
        );
        assert!(
            rpc.to_string().contains(field),
            "expected `{field}` to be named, got: {rpc}"
        );
    }

    const NON_SIGNING_KEYS: [KeyId; 4] = [
        KeyId::Derived {
            key_branch: KeyBranch::ConfidentialMask,
            index: 0,
        },
        KeyId::Derived {
            key_branch: KeyBranch::StealthMask,
            index: 0,
        },
        KeyId::Derived {
            key_branch: KeyBranch::ViewOnlyKey,
            index: 0,
        },
        KeyId::Derived {
            key_branch: KeyBranch::ElgamalEncryptionViewKey,
            index: 0,
        },
    ];

    #[tokio::test]
    async fn submit_refuses_to_sign_with_mask_and_view_keys() {
        let daemon = TestDaemon::start().await;
        let account = KeyId::derived(KeyBranch::Account, 0);

        for key in NON_SIGNING_KEYS {
            let err = handle_submit(
                &daemon.context,
                Some(&daemon.session),
                submit_request(account, vec![key]),
            )
            .await
            .unwrap_err();
            assert_invalid_params(&err, "other_signers");

            let err = handle_submit(&daemon.context, Some(&daemon.session), submit_request(key, vec![]))
                .await
                .unwrap_err();
            assert_invalid_params(&err, "seal_signer");
        }
    }

    #[tokio::test]
    async fn a_dry_run_refuses_to_sign_with_mask_and_view_keys() {
        let daemon = TestDaemon::start().await;
        let reader = daemon.api_key("reader", "transactions:read");
        let account = KeyId::derived(KeyBranch::Account, 0);

        for key in NON_SIGNING_KEYS {
            let err = handle_submit_dry_run(&daemon.context, Some(&reader), submit_request(account, vec![key]))
                .await
                .unwrap_err();
            assert_invalid_params(&err, "other_signers");

            let err = handle_submit_dry_run(&daemon.context, Some(&reader), submit_request(key, vec![]))
                .await
                .unwrap_err();
            assert_invalid_params(&err, "seal_signer");
        }
    }

    /// `accounts.create_stealth_transfer_statement` hands out a nonce key for the
    /// caller to pass back as a co-signer.
    #[tokio::test]
    async fn a_nonce_key_still_co_signs() {
        let daemon = TestDaemon::start().await;
        let request = submit_request(KeyId::derived(KeyBranch::Account, 0), vec![KeyId::derived(
            KeyBranch::Nonce,
            0,
        )]);

        // The test daemon has no transaction service, so the dry run fails after signing.
        let err = handle_submit_dry_run(&daemon.context, Some(&daemon.session), request)
            .await
            .unwrap_err();
        assert!(err.to_string().starts_with("Failed to submit transaction"), "{err}");
    }

    #[tokio::test]
    async fn the_signer_api_refuses_mask_and_view_keys() {
        let daemon = TestDaemon::start().await;
        let sdk = daemon.context.wallet_sdk();
        let unsigned = UnsignedTransaction::new(Network::LocalNet.as_byte(), Epoch(100));
        let seal_pk = RistrettoPublicKeyBytes::zero();
        let signer = sdk.signer_api().with_context(&seal_pk);

        for key in NON_SIGNING_KEYS {
            signer.generate_signature(key, &unsigned).unwrap_err();
        }
        for key in [
            KeyId::derived(KeyBranch::Account, 0),
            KeyId::derived(KeyBranch::Transaction, 0),
            KeyId::derived(KeyBranch::Nonce, 0),
        ] {
            signer.generate_signature(key, &unsigned).unwrap();
        }
    }
}
