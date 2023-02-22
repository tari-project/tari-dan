//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::time::Duration;

use anyhow::anyhow;
use futures::{future, future::Either};
use log::*;
use tari_dan_common_types::{optional::Optional, ShardId};
use tari_engine_types::{instruction::Instruction, substate::SubstateAddress};
use tari_template_lib::prelude::NonFungibleAddress;
use tari_transaction::Transaction;
use tari_wallet_daemon_client::types::{
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

use super::context::HandlerContext;
use crate::{
    handlers::{HandlerError, TRANSACTION_KEYMANAGER_BRANCH},
    services::{TransactionSubmittedEvent, WalletEvent},
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::handlers::transaction";

pub async fn handle_submit(
    context: &HandlerContext,
    req: TransactionSubmitRequest,
) -> Result<TransactionSubmitResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let key_api = sdk.key_manager_api();
    // Fetch the key to sign the transaction
    // TODO: Ideally the SDK should take care of signing the transaction internally
    let (_, key) = key_api.get_key_or_active(TRANSACTION_KEYMANAGER_BRANCH, req.signing_key_index)?;

    // let transaction_api = sdk.transaction_api();
    let inputs = if req.inputs.is_empty() {
        // If no inputs are specified, we will use the default inputs
        // sdk.transaction_api().default_inputs().await?
        let substates = get_referenced_component_addresses(&req.instructions);
        sdk.substate_api().load_dependent_substates(&substates)?
    } else {
        req.inputs
    };

    // TODO: we assume that all inputs will be consumed and produce a new output however this is only the case when the
    //       object is mutated
    let mut outputs = inputs
        .iter()
        .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version + 1))
        .collect::<Vec<_>>();

    outputs.extend(req.specific_non_fungible_outputs.into_iter().map(|(resx_addr, id)| {
        ShardId::from_address(&SubstateAddress::NonFungible(NonFungibleAddress::new(resx_addr, id)), 0)
    }));

    let inputs = inputs
        .into_iter()
        .map(|versioned_addr| ShardId::from_address(&versioned_addr.address, versioned_addr.version))
        .collect::<Vec<_>>();

    let mut builder = Transaction::builder();
    builder
        .with_fee(req.fee)
        .with_inputs(inputs.clone())
        .with_outputs(outputs.clone())
        .with_new_outputs(req.new_outputs)
        .with_new_non_fungible_outputs(req.new_non_fungible_outputs)
        .with_instructions(req.instructions)
        .sign(&key.k);

    let transaction = builder.build();

    info!(
        target: LOG_TARGET,
        "Submitted transaction with hash {}",
        transaction.hash()
    );
    let hash = if req.is_dry_run {
        sdk.transaction_api().submit_dry_run_to_vn(transaction).await?
    } else {
        sdk.transaction_api().submit_to_vn(transaction).await?
    };

    if !req.is_dry_run {
        context.notifier().notify(TransactionSubmittedEvent { hash });
    }

    Ok(TransactionSubmitResponse { hash, inputs, outputs })
}

pub async fn handle_get(
    context: &HandlerContext,
    req: TransactionGetRequest,
) -> Result<TransactionGetResponse, anyhow::Error> {
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.hash)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    Ok(TransactionGetResponse {
        hash: req.hash,
        transaction: transaction.transaction,
        result: transaction.result,
        status: transaction.status,
    })
}

pub async fn handle_get_result(
    context: &HandlerContext,
    req: TransactionGetResultRequest,
) -> Result<TransactionGetResultResponse, anyhow::Error> {
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.hash)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    Ok(TransactionGetResultResponse {
        hash: req.hash,
        result: transaction.result,
        // TODO: Populate QC
        qc: None,
        status: transaction.status,
    })
}

pub async fn handle_claim_burn(
    context: &HandlerContext,
    req: TransactionWaitResultRequest,
) -> Result<TransactionClaimBurnResponse, anyhow::Error> {
    handle_submit(context, req).await
}

pub async fn handle_wait_result(
    context: &HandlerContext,
    req: TransactionWaitResultRequest,
) -> Result<TransactionWaitResultResponse, anyhow::Error> {
    let mut events = context.notifier().subscribe();
    let transaction = context
        .wallet_sdk()
        .transaction_api()
        .get(req.hash)
        .optional()?
        .ok_or(HandlerError::NotFound)?;

    if let Some(result) = transaction.result {
        return Ok(TransactionWaitResultResponse {
            hash: req.hash,
            result: Some(result),
            status: transaction.status,
            qcs: transaction.qcs,
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
            Some(WalletEvent::TransactionFinalized(finalized)) if finalized.hash == req.hash => {
                return Ok(TransactionWaitResultResponse {
                    hash: req.hash,
                    result: Some(finalized.result),
                    qcs: finalized.qcs,
                    status: finalized.status,
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
                    timed_out: true,
                });
            },
        };
    }
}

fn get_referenced_component_addresses(instructions: &[Instruction]) -> Vec<SubstateAddress> {
    let mut components = Vec::new();
    for instruction in instructions {
        if let Instruction::CallMethod { component_address, .. } = instruction {
            components.push(SubstateAddress::Component(*component_address));
        }
    }
    components
}
