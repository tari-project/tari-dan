//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeMap, str::FromStr};

use anyhow::anyhow;
use log::info;
use tari_common_types::types::PublicKey;
use tari_crypto::{keys::PublicKey as PK, ristretto::RistrettoSecretKey, tari_utilities::ByteArray};
use tari_dan_common_types::SubstateRequirement;
use tari_dan_wallet_sdk::{
    apis::{jwt::JrpcPermission, key_manager},
    models::Account,
};
use tari_engine_types::{instruction::Instruction, substate::SubstateId};
use tari_template_builtin::ACCOUNT_NFT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    crypto::RistrettoPublicKeyBytes,
    prelude::{Amount, ComponentAddress, Metadata, NonFungibleAddress, NonFungibleId, ResourceAddress},
};
use tari_transaction::{Transaction, TransactionId};
use tari_wallet_daemon_client::types::{
    GetAccountNftRequest,
    GetAccountNftResponse,
    ListAccountNftRequest,
    ListAccountNftResponse,
    MintAccountNftRequest,
    MintAccountNftResponse,
};
use tokio::sync::broadcast;

use super::{context::HandlerContext, helpers::get_account_or_default};
use crate::{
    handlers::helpers::get_account,
    services::{TransactionFinalizedEvent, WalletEvent},
    DEFAULT_FEE,
};

const LOG_TARGET: &str = "tari::dan::wallet_daemon::handlers::nfts";

pub async fn handle_get_nft(
    context: &HandlerContext,
    token: Option<String>,
    req: GetAccountNftRequest,
) -> Result<GetAccountNftResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let non_fungible_api = sdk.non_fungible_api();

    let non_fungible = non_fungible_api
        .non_fungible_token_get_by_nft_id(req.nft_id)
        .map_err(|e| anyhow!("Failed to get non fungible token, with error: {}", e))?;

    Ok(non_fungible)
}

pub async fn handle_list_nfts(
    context: &HandlerContext,
    token: Option<String>,
    req: ListAccountNftRequest,
) -> Result<ListAccountNftResponse, anyhow::Error> {
    let ListAccountNftRequest { account, limit, offset } = req;
    let sdk = context.wallet_sdk();
    let account = get_account_or_default(account, &sdk.accounts_api())?;
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let non_fungible_api = sdk.non_fungible_api();

    let non_fungibles = non_fungible_api
        .non_fungible_token_get_all(account.address.as_component_address().unwrap(), limit, offset)
        .map_err(|e| anyhow!("Failed to list all non fungibles, with error: {}", e))?;
    Ok(ListAccountNftResponse { nfts: non_fungibles })
}

pub async fn handle_mint_account_nft(
    context: &HandlerContext,
    token: Option<String>,
    req: MintAccountNftRequest,
) -> Result<MintAccountNftResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let key_manager_api = sdk.key_manager_api();
    sdk.jwt_api().check_auth(token.clone(), &[JrpcPermission::Admin])?;

    let account = get_account(&req.account, &sdk.accounts_api())?;

    let signing_key_index = account.key_index;
    let signing_key = key_manager_api.derive_key(key_manager::TRANSACTION_BRANCH, signing_key_index)?;

    let owner_pk = PublicKey::from_secret_key(&signing_key.key);
    let owner_token =
        NonFungibleAddress::from_public_key(RistrettoPublicKeyBytes::from_bytes(owner_pk.as_bytes()).unwrap());

    info!(target: LOG_TARGET, "Minting new NFT with metadata {}", req.metadata);

    let mut total_fee = Amount::new(0);
    let component_address = match req.existing_nft_component {
        Some(existing_nft_component) => existing_nft_component,
        None => {
            let resp = create_account_nft(
                context,
                &account,
                &signing_key.key,
                owner_token,
                req.create_account_nft_fee.unwrap_or(DEFAULT_FEE),
                token.clone(),
            )
            .await?;

            total_fee += resp.final_fee;
            if let Some(reason) = resp.finalize.result.full_reject() {
                return Err(anyhow!("Failed to create account NFT: {}", reason));
            }
            let component_address = resp
                .finalize
                .result
                .accept()
                .unwrap()
                .up_iter()
                .filter(|(id, _)| id.is_component())
                .find(|(_, s)| s.substate_value().component().unwrap().template_address == ACCOUNT_NFT_TEMPLATE_ADDRESS)
                .map(|(id, _)| id.as_component_address().unwrap())
                .ok_or_else(|| anyhow!("Failed to find account NFT component address"))?;

            // Strange issue with current rust version, if return the _OWNED_ value directly, it will not compile.
            #[allow(clippy::let_and_return)]
            component_address
        },
    };

    let metadata = Metadata::from(serde_json::from_value::<BTreeMap<String, String>>(req.metadata)?);

    let resp = mint_account_nft(
        context,
        token,
        account,
        component_address,
        &signing_key.key,
        req.mint_fee.unwrap_or(DEFAULT_FEE),
        metadata,
    )
    .await?;
    // TODO: is there a more direct way to extract nft_id and resource address ??
    let (resource_address, nft_id) = resp
        .finalize
        .events
        .iter()
        .find(|e| e.topic().as_str() == "mint")
        .map(|e| {
            (
                e.get_payload("resource_address").expect("Resource address not found"),
                e.get_payload("id").expect("NFTID not found"),
            )
        })
        .expect("NFT ID event payload not found");
    let resource_address = ResourceAddress::from_str(&resource_address)?;
    let nft_id = NonFungibleId::try_from_canonical_string(nft_id.as_str())
        .map_err(|e| anyhow!("Failed to parse non fungible id, with error: {:?}", e))?;

    total_fee += resp.final_fee;

    Ok(MintAccountNftResponse {
        result: resp.finalize,
        resource_address,
        nft_id,
        fee: total_fee,
    })
}

async fn mint_account_nft(
    context: &HandlerContext,
    token: Option<String>,
    account: Account,
    component_address: ComponentAddress,
    owner_sk: &RistrettoSecretKey,
    fee: Amount,
    metadata: Metadata,
) -> Result<TransactionFinalizedEvent, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let mut inputs = sdk
        .substate_api()
        .locate_dependent_substates(&[account.address.clone()])
        .await?;

    inputs.extend([SubstateRequirement::new(SubstateId::Component(component_address), None)]);

    let instructions = vec![
        Instruction::CallMethod {
            component_address,
            method: "mint".to_string(),
            args: args![metadata],
        },
        Instruction::PutLastInstructionOutputOnWorkspace {
            key: b"bucket".to_vec(),
        },
        Instruction::CallMethod {
            component_address: account
                .address
                .as_component_address()
                .expect("Failed to get account component address"),
            method: "deposit".to_string(),
            args: args![Workspace("bucket")],
        },
    ];

    let transaction = Transaction::builder()
        .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), fee)
        .with_instructions(instructions)
        .sign(owner_sk)
        .build();

    let mut events = context.notifier().subscribe();
    let tx_id = context
        .transaction_service()
        .submit_transaction(transaction, inputs)
        .await?;

    let event = wait_for_result(&mut events, tx_id).await?;
    if let Some(reject) = event.finalize.result.reject() {
        return Err(anyhow!(
            "Mint new NFT using account {} was rejected: {}",
            account,
            reject
        ));
    }
    if let Some(reason) = event.finalize.reject() {
        return Err(anyhow!("Mint new NFT using account {}, failed: {}", account, reason));
    }

    Ok(event)
}

async fn create_account_nft(
    context: &HandlerContext,
    account: &Account,
    owner_sk: &RistrettoSecretKey,
    owner_token: NonFungibleAddress,
    fee: Amount,
    token: Option<String>,
) -> Result<TransactionFinalizedEvent, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let inputs = sdk
        .substate_api()
        .locate_dependent_substates(&[account.address.clone()])
        .await?;

    let transaction = Transaction::builder()
        .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), fee)
        .call_function(ACCOUNT_NFT_TEMPLATE_ADDRESS, "create", args![owner_token,])
        .with_inputs(inputs)
        .sign(owner_sk)
        .build();

    let tx_id = sdk
        .transaction_api()
        .insert_new_transaction(transaction, vec![], None, false)
        .await?;
    let mut events = context.notifier().subscribe();
    sdk.transaction_api().submit_transaction(tx_id).await?;

    let event = wait_for_result(&mut events, tx_id).await?;
    if let Some(reject) = event.finalize.result.reject() {
        return Err(anyhow!(
            "Create NFT resource address from account {} was rejected: {}",
            account,
            reject
        ));
    }
    if let Some(reason) = event.finalize.reject() {
        return Err(anyhow!(
            "Create NFT resource address transaction, from account {}, failed: {}",
            account,
            reason
        ));
    }

    Ok(event)
}

async fn wait_for_result(
    events: &mut broadcast::Receiver<WalletEvent>,
    transaction_id: TransactionId,
) -> Result<TransactionFinalizedEvent, anyhow::Error> {
    loop {
        let wallet_event = events.recv().await?;
        match wallet_event {
            WalletEvent::TransactionFinalized(event) if event.transaction_id == transaction_id => return Ok(event),
            _ => {},
        }
    }
}
