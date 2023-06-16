//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::{collections::HashSet, convert::TryFrom, time::Duration};

use anyhow::anyhow;
use futures::{future, future::Either};
use log::*;
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_dan_wallet_sdk::apis::{jwt::JrpcPermission, key_manager};
use tari_engine_types::{instruction::Instruction, substate::SubstateAddress};
use tari_template_lib::{args, models::Amount, prelude::NonFungibleAddress};
use tari_transaction::Transaction;
use tari_wallet_daemon_client::types::{
    AccountGetRequest,
    AccountGetResponse,
    CallInstructionRequest,
    TransactionGetRequest,
    TransactionGetResponse,
    TransactionGetResultRequest,
    TransactionGetResultResponse,
    TransactionSubmitRequest,
    TransactionSubmitResponse,
    TransactionWaitResultRequest,
    TransactionWaitResultResponse,
};
use tokio::time;

use super::{accounts, context::HandlerContext};
use crate::{
    handlers::HandlerError,
    services::{TransactionSubmittedEvent, WalletEvent},
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::handlers::transaction";

pub async fn handle_submit_instruction(
    context: &HandlerContext,
    token: Option<String>,
    req: CallInstructionRequest,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let mut instructions = vec![req.instruction];
    if let Some(dump_account) = req.dump_outputs_into {
        instructions.push(Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"bucket".to_vec(),
        });
        let AccountGetResponse {
            account: dump_account, ..
        } = accounts::handle_get(context, token.clone(), AccountGetRequest {
            name_or_address: dump_account,
        })
        .await?;
        instructions.push(Instruction::CallMethod {
            component_address: dump_account.address.as_component_address().unwrap(),
            method: "deposit".to_string(),
            args: args![Variable("bucket")],
        });
    }
    let AccountGetResponse {
        account: fee_account, ..
    } = accounts::handle_get(context, token.clone(), AccountGetRequest {
        name_or_address: req.fee_account,
    })
    .await?;
    let request = TransactionSubmitRequest {
        signing_key_index: Some(fee_account.key_index),
        fee_instructions: vec![Instruction::CallMethod {
            component_address: fee_account.address.as_component_address().unwrap(),
            method: "pay_fee".to_string(),
            args: args![Amount::try_from(req.fee)?],
        }],
        instructions,
        inputs: req.inputs,
        override_inputs: req.override_inputs.unwrap_or_default(),
        new_outputs: req.new_outputs.unwrap_or(0),
        specific_non_fungible_outputs: req.specific_non_fungible_outputs,
        new_resources: req.new_resources,
        new_non_fungible_outputs: req.new_non_fungible_outputs,
        new_non_fungible_index_outputs: req.new_non_fungible_index_outputs,
        is_dry_run: req.is_dry_run,
        proof_ids: vec![],
    };
    handle_submit(context, token, request).await
}

pub async fn handle_submit(
    context: &HandlerContext,
    token: Option<String>,
    req: TransactionSubmitRequest,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    // TODO: fine-grained checks of individual addresses involved (resources, components, etc)
    sdk.jwt_api()
        .check_auth(token, &[JrpcPermission::TransactionSend(None)])?;
    let key_api = sdk.key_manager_api();
    // Fetch the key to sign the transaction
    // TODO: Ideally the SDK should take care of signing the transaction internally
    let (_, key) = key_api.get_key_or_active(key_manager::TRANSACTION_BRANCH, req.signing_key_index)?;

    let inputs = if req.override_inputs {
        req.inputs
    } else {
        // If we are not overriding inputs, we will use inputs that we know about in the local substate address db
        let mut substates = get_referenced_component_addresses(&req.instructions);
        substates.extend(get_referenced_component_addresses(&req.fee_instructions));
        let substates = substates.iter().collect::<Vec<_>>();
        let loaded_dependent_substates = sdk
            .substate_api()
            .locate_dependent_substates(&substates)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        vec![req.inputs, loaded_dependent_substates].concat()
    };

    let outputs: Vec<ShardId> = req
        .specific_non_fungible_outputs
        .into_iter()
        .map(|(resx_addr, id)| {
            ShardId::from_address(&SubstateAddress::NonFungible(NonFungibleAddress::new(resx_addr, id)), 0)
        })
        .collect();

    let transaction = Transaction::builder()
        .with_instructions(req.instructions)
        .with_fee_instructions(req.fee_instructions)
        .with_required_inputs(inputs.clone())
        .with_outputs(outputs.clone())
        .with_new_resources(req.new_resources)
        .with_new_non_fungible_outputs(req.new_non_fungible_outputs)
        .with_new_non_fungible_index_outputs(req.new_non_fungible_index_outputs)
        .sign(&key.k)
        .build();

    for proof_id in req.proof_ids {
        // update the proofs table with the corresponding transaction hash
        sdk.confidential_outputs_api()
            .proofs_set_transaction_hash(proof_id, *transaction.hash())?;
    }

    info!(
        target: LOG_TARGET,
        "Submitted transaction with hash {}",
        transaction.hash()
    );

    if req.is_dry_run {
        let response = sdk.transaction_api().submit_dry_run_transaction(transaction).await?;
        let result = match response.execution_result {
            Some(res) => Some(res.finalize),
            None => None,
        };
        Ok(TransactionSubmitResponse {
            hash: response.transaction_hash,
            inputs,
            outputs,
            result,
        })
    } else {
        let response = sdk.transaction_api().submit_transaction(transaction).await?;
        context.notifier().notify(TransactionSubmittedEvent {
            hash: response.transaction_hash,
            new_account: None,
        });
        Ok(TransactionSubmitResponse {
            hash: response.transaction_hash,
            inputs,
            outputs,
            result: None,
        })
    }
}

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<String>,
    req: TransactionGetRequest,
) -> Result<TransactionGetResponse, anyhow::Error> {
    context
        .wallet_sdk()
        .jwt_api()
        .check_auth(token, &[JrpcPermission::TransactionGet])?;
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.hash)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    Ok(TransactionGetResponse {
        hash: req.hash,
        transaction: transaction.transaction,
        result: transaction.finalize,
        status: transaction.status,
        transaction_failure: transaction.transaction_failure,
    })
}

pub async fn handle_get_result(
    context: &HandlerContext,
    token: Option<String>,
    req: TransactionGetResultRequest,
) -> Result<TransactionGetResultResponse, anyhow::Error> {
    context
        .wallet_sdk()
        .jwt_api()
        .check_auth(token, &[JrpcPermission::TransactionGet])?;
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.hash)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    Ok(TransactionGetResultResponse {
        hash: req.hash,
        result: transaction.finalize,
        // TODO: Populate QC
        qc: None,
        status: transaction.status,
    })
}

pub async fn handle_wait_result(
    context: &HandlerContext,
    token: Option<String>,
    req: TransactionWaitResultRequest,
) -> Result<TransactionWaitResultResponse, anyhow::Error> {
    context
        .wallet_sdk()
        .jwt_api()
        .check_auth(token, &[JrpcPermission::Admin])?;
    let mut events = context.notifier().subscribe();
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.hash)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    if let Some(result) = transaction.finalize {
        return Ok(TransactionWaitResultResponse {
            hash: req.hash,
            result: Some(result),
            status: transaction.status,
            qcs: transaction.qcs,
            final_fee: transaction.final_fee.unwrap_or_default(),
            timed_out: false,
            transaction_failure: transaction.transaction_failure,
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
            Some(WalletEvent::TransactionFinalized(event)) if event.hash == req.hash => {
                return Ok(TransactionWaitResultResponse {
                    hash: req.hash,
                    result: Some(event.finalize),
                    qcs: event.qcs,
                    status: event.status,
                    transaction_failure: event.transaction_failure,
                    final_fee: event.final_fee,
                    timed_out: false,
                });
            },
            Some(WalletEvent::TransactionInvalid(event)) if event.hash == req.hash => {
                return Ok(TransactionWaitResultResponse {
                    hash: req.hash,
                    result: None,
                    qcs: vec![],
                    status: event.status,
                    transaction_failure: None,
                    final_fee: event.final_fee,
                    timed_out: false,
                });
            },
            Some(_) => continue,
            None => {
                return Ok(TransactionWaitResultResponse {
                    hash: req.hash,
                    result: None,
                    qcs: vec![],
                    status: transaction.status,
                    transaction_failure: transaction.transaction_failure,
                    final_fee: Amount::zero(),
                    timed_out: true,
                });
            },
        };
    }
}

fn get_referenced_component_addresses(instructions: &[Instruction]) -> HashSet<SubstateAddress> {
    let mut components = HashSet::new();
    for instruction in instructions {
        if let Instruction::CallMethod { component_address, .. } = instruction {
            components.insert(SubstateAddress::Component(*component_address));
        }
    }
    components
}
