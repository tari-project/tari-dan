//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::BTreeMap, str::FromStr};

use anyhow::anyhow;
use log::info;
use tari_common_types::types::PublicKey;
use tari_crypto::{keys::PublicKey as PK, ristretto::RistrettoSecretKey};
use tari_dan_common_types::ShardId;
use tari_dan_wallet_sdk::{
    apis::{jwt::JrpcPermission, key_manager},
    models::Account,
};
use tari_engine_types::{
    component::new_component_address_from_parts,
    instruction::Instruction,
    substate::SubstateAddress,
};
use tari_template_builtin::ACCOUNT_NFT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    crypto::RistrettoPublicKeyBytes,
    prelude::{Amount, ComponentAddress, Metadata, NonFungibleAddress, NonFungibleId, ResourceAddress},
};
use tari_transaction::{SubstateRequirement, Transaction, TransactionId};
use tari_utilities::ByteArray;
use tari_wallet_daemon_client::types::{
    AccountNftInfo,
    GetAccountNftRequest,
    GetAccountNftResponse,
    ListAccountNftRequest,
    ListAccountNftResponse,
    MintAccountNftRequest,
    MintAccountNftResponse,
};
use tokio::sync::broadcast;

use super::context::HandlerContext;
use crate::{
    handlers::get_account,
    services::{TransactionFinalizedEvent, TransactionSubmittedEvent, WalletEvent},
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
    let token_symbol = non_fungible.token_symbol.clone();
    let is_burned = non_fungible.is_burned;
    let metadata = serde_json::to_value(&non_fungible.metadata)?;
    let resp = GetAccountNftResponse {
        token_symbol,
        metadata,
        is_burned,
    };

    Ok(resp)
}

pub async fn handle_list_nfts(
    context: &HandlerContext,
    token: Option<String>,
    req: ListAccountNftRequest,
) -> Result<ListAccountNftResponse, anyhow::Error> {
    let ListAccountNftRequest { limit, offset, .. } = req;
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let non_fungible_api = sdk.non_fungible_api();

    let non_fungibles = non_fungible_api
        .non_fungible_token_get_all(limit, offset)
        .map_err(|e| anyhow!("Failed to list all non fungibles, with error: {}", e))?;
    let non_fungibles = non_fungibles
        .iter()
        .map(|n| {
            let metadata = serde_json::to_value(&n.metadata).expect("failed to parse metadata to JSON format");
            AccountNftInfo {
                token_symbol: n.token_symbol.clone(),
                is_burned: n.is_burned,
                metadata,
            }
        })
        .collect();
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

    // check if the component address already exists
    let component_address = new_component_address_from_parts(
        &ACCOUNT_NFT_TEMPLATE_ADDRESS,
        &owner_token
            .to_public_key()
            .unwrap_or_else(|| panic!("owner_token is not a valid public key: {}", owner_token))
            .as_hash(),
    );

    let mut accrued_fee = Amount::new(0);
    if sdk
        .substate_api()
        .scan_for_substate(&SubstateAddress::Component(component_address), None)
        .await
        .is_err()
    {
        accrued_fee = create_account_nft(
            context,
            &account,
            &signing_key.key,
            owner_token,
            &req.token_symbol,
            req.create_account_nft_fee.unwrap_or(DEFAULT_FEE),
            token.clone(),
        )
        .await?;
    }

    let metadata = Metadata::from(serde_json::from_value::<BTreeMap<String, String>>(req.metadata)?);

    mint_account_nft(
        context,
        token,
        account,
        component_address,
        &signing_key.key,
        req.mint_fee.unwrap_or(DEFAULT_FEE),
        metadata,
        accrued_fee,
    )
    .await
}

async fn mint_account_nft(
    context: &HandlerContext,
    token: Option<String>,
    account: Account,
    component_address: ComponentAddress,
    owner_sk: &RistrettoSecretKey,
    fee: Amount,
    metadata: Metadata,
    mut accrued_fee: Amount,
) -> Result<MintAccountNftResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let inputs = sdk
        .substate_api()
        .locate_dependent_substates(&[&account.address])
        .await?;

    let mut inputs = inputs
        .iter()
        .map(|v| SubstateRequirement::new(v.address.clone(), Some(v.version)))
        .collect::<Vec<_>>();
    inputs.extend([SubstateRequirement::new(
        SubstateAddress::Component(component_address),
        None,
    )]);

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

    let tx_hash = sdk.transaction_api().submit_transaction(transaction, inputs).await?;
    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent {
        transaction_id: tx_hash,
        new_account: None,
    });

    let event = wait_for_result(&mut events, tx_hash).await?;
    if let Some(reject) = event.finalize.result.reject() {
        return Err(anyhow!(
            "Mint new NFT using account {} was rejected: {}",
            account.name,
            reject
        ));
    }
    if let Some(reason) = event.transaction_failure {
        return Err(anyhow!(
            "Mint new NFT using account {}, failed: {}",
            account.name,
            reason
        ));
    }

    // TODO: is there a more direct way to extract nft_id and resource address ??
    let (resource_address, nft_id) = event
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
    accrued_fee += event.final_fee;

    Ok(MintAccountNftResponse {
        result: event.finalize,
        resource_address,
        nft_id,
        fee: accrued_fee,
    })
}

async fn create_account_nft(
    context: &HandlerContext,
    account: &Account,
    owner_sk: &RistrettoSecretKey,
    owner_token: NonFungibleAddress,
    token_symbol: &str,
    fee: Amount,
    token: Option<String>,
) -> Result<Amount, anyhow::Error> {
    let sdk = context.wallet_sdk();
    sdk.jwt_api().check_auth(token, &[JrpcPermission::Admin])?;

    let inputs = sdk
        .substate_api()
        .locate_dependent_substates(&[&account.address])
        .await?;
    let inputs = inputs
        .iter()
        .map(|addr| ShardId::from_address(&addr.address, addr.version))
        .collect::<Vec<_>>();

    let transaction = Transaction::builder()
        .fee_transaction_pay_from_component(account.address.as_component_address().unwrap(), fee)
        .with_inputs(inputs)
        .call_function(*ACCOUNT_NFT_TEMPLATE_ADDRESS, "create", args![
            owner_token,
            token_symbol
        ])
        .sign(owner_sk)
        .build();

    let tx_id = sdk.transaction_api().submit_transaction(transaction, vec![]).await?;
    let mut events = context.notifier().subscribe();
    context.notifier().notify(TransactionSubmittedEvent {
        transaction_id: tx_id,
        new_account: None,
    });

    let event = wait_for_result(&mut events, tx_id).await?;
    if let Some(reject) = event.finalize.result.reject() {
        return Err(anyhow!(
            "Create NFT resource address from account {} was rejected: {}",
            account.name,
            reject
        ));
    }
    if let Some(reason) = event.transaction_failure {
        return Err(anyhow!(
            "Create NFT resource address transaction, from account {}, failed: {}",
            account.name,
            reason
        ));
    }

    Ok(event.final_fee)
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
