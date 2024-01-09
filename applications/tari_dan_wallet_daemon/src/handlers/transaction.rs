//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::{collections::HashSet, convert::TryFrom, time::Duration};

use anyhow::anyhow;
use futures::{future, future::Either};
use log::*;
use tari_dan_common_types::{optional::Optional, Epoch};
use tari_dan_wallet_sdk::{
    apis::{jwt::JrpcPermission, key_manager},
    network::{TransactionFinalizedResult, TransactionQueryResult},
};
use tari_engine_types::{indexed_value::IndexedValue, instruction::Instruction, substate::SubstateAddress};
use tari_template_lib::{args, args::Arg, models::Amount};
use tari_transaction::Transaction;
use tari_wallet_daemon_client::types::{
    AccountGetRequest,
    AccountGetResponse,
    CallInstructionRequest,
    TransactionGetAllRequest,
    TransactionGetAllResponse,
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
    let mut instructions = req.instructions;
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
            args: args![Amount::try_from(req.max_fee)?],
        }],
        instructions,
        inputs: req.inputs,
        override_inputs: req.override_inputs.unwrap_or_default(),
        is_dry_run: req.is_dry_run,
        proof_ids: vec![],
        min_epoch: req.min_epoch.map(Epoch),
        max_epoch: req.max_epoch.map(Epoch),
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
        let mut substates = get_referenced_substate_addresses(&req.instructions)?;
        substates.extend(get_referenced_substate_addresses(&req.fee_instructions)?);
        let substates = substates.into_iter().collect::<Vec<_>>();
        let loaded_dependent_substates = sdk
            .substate_api()
            .locate_dependent_substates(&substates)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        [req.inputs, loaded_dependent_substates].concat()
    };

    let transaction = Transaction::builder()
        .with_instructions(req.instructions)
        .with_fee_instructions(req.fee_instructions)
        .with_min_epoch(req.min_epoch)
        .with_max_epoch(req.max_epoch)
        .sign(&key.key)
        .build();

    for proof_id in req.proof_ids {
        // update the proofs table with the corresponding transaction hash
        sdk.confidential_outputs_api()
            .proofs_set_transaction_hash(proof_id, *transaction.id())?;
    }

    info!(
        target: LOG_TARGET,
        "Submitted transaction with hash {}",
        transaction.hash()
    );
    if req.is_dry_run {
        let response: TransactionQueryResult = sdk
            .transaction_api()
            .submit_dry_run_transaction(transaction, inputs.clone())
            .await?;

        let json_result = match &response.result {
            TransactionFinalizedResult::Pending => None,
            TransactionFinalizedResult::Finalized { json_results, .. } => Some(json_results.clone()),
        };

        Ok(TransactionSubmitResponse {
            transaction_id: response.transaction_id,
            result: response.result.into_execute_result(),
            json_result,
            inputs,
        })
    } else {
        let transaction_id = sdk
            .transaction_api()
            .submit_transaction(transaction, inputs.clone())
            .await?;

        context.notifier().notify(TransactionSubmittedEvent {
            transaction_id,
            new_account: None,
        });

        Ok(TransactionSubmitResponse {
            transaction_id,
            inputs,
            result: None,
            json_result: None,
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
        .get(req.transaction_id)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    Ok(TransactionGetResponse {
        transaction: transaction.transaction,
        result: transaction.finalize,
        status: transaction.status,
        last_update_time: transaction.last_update_time,
    })
}

pub async fn handle_get_all(
    context: &HandlerContext,
    token: Option<String>,
    req: TransactionGetAllRequest,
) -> Result<TransactionGetAllResponse, anyhow::Error> {
    context
        .wallet_sdk()
        .jwt_api()
        .check_auth(token, &[JrpcPermission::TransactionGet])?;
    let transactions = context
        .wallet_sdk()
        .transaction_api()
        .fetch_all(req.status, req.component)?;
    Ok(TransactionGetAllResponse {
        transactions: transactions
            .into_iter()
            .map(|tx| (tx.transaction, tx.finalize, tx.status, tx.last_update_time))
            .collect(),
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
        .get(req.transaction_id)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    Ok(TransactionGetResultResponse {
        transaction_id: req.transaction_id,
        result: transaction.finalize,
        status: transaction.status,
        json_result: transaction.json_result,
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
        .check_auth(token, &[JrpcPermission::TransactionGet])?;
    let mut events = context.notifier().subscribe();
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.transaction_id)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    if let Some(result) = transaction.finalize {
        return Ok(TransactionWaitResultResponse {
            transaction_id: req.transaction_id,
            result: Some(result),
            status: transaction.status,
            final_fee: transaction.final_fee.unwrap_or_default(),
            timed_out: false,
            json_result: transaction.json_result,
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
                    json_result: event.json_result,
                });
            },
            Some(WalletEvent::TransactionInvalid(event)) if event.transaction_id == req.transaction_id => {
                return Ok(TransactionWaitResultResponse {
                    transaction_id: req.transaction_id,
                    result: event.finalize,
                    status: event.status,
                    final_fee: event.final_fee.unwrap_or_default(),
                    timed_out: false,
                    json_result: None,
                });
            },
            Some(_) => continue,
            None => {
                return Ok(TransactionWaitResultResponse {
                    transaction_id: req.transaction_id,
                    result: None,
                    status: transaction.status,
                    final_fee: Amount::zero(),
                    timed_out: true,
                    json_result: None,
                });
            },
        };
    }
}

fn get_referenced_substate_addresses(instructions: &[Instruction]) -> anyhow::Result<HashSet<SubstateAddress>> {
    let mut substates = HashSet::new();
    for instruction in instructions {
        match instruction {
            Instruction::CallMethod {
                component_address,
                args,
                ..
            } => {
                substates.insert(SubstateAddress::Component(*component_address));
                for arg in args {
                    if let Arg::Literal(bytes) = arg {
                        let val = IndexedValue::from_raw(bytes)?;
                        substates.extend(val.referenced_substates());
                    }
                }
            },
            Instruction::CallFunction { args, .. } => {
                for arg in args {
                    if let Arg::Literal(bytes) = arg {
                        let val = IndexedValue::from_raw(bytes)?;
                        substates.extend(val.referenced_substates());
                    }
                }
            },
            _ => {},
        }
    }
    Ok(substates)
}
