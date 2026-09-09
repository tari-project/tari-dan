//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, iter};

use anyhow::anyhow;
use axum_extra::headers::authorization::Bearer;
use either::Either;
use log::*;
use ootle_byte_type::ToByteType;
use tari_crypto::{keys::PublicKey as _, ristretto::RistrettoPublicKey};
use tari_engine_types::substate::SubstateId;
use tari_ootle_common_types::{SubstateAddress, SubstateRequirement, derive_fee_pool_address};
use tari_ootle_transaction::args;
use tari_ootle_wallet_crypto::{OutputWitness, StealthInputWitness, StealthOutputWitness, memo::Memo};
use tari_ootle_wallet_sdk::models::{KeyBranch, KeyId, TransactionContext};
use tari_ootle_walletd_client::{
    permissions::{Crud, Permission},
    types::{
        AccountOrKeyId,
        ClaimValidatorFeesRequest,
        ClaimValidatorFeesResponse,
        FeePoolDetails,
        GetValidatorFeesRequest,
        GetValidatorFeesResponse,
    },
};
use tari_template_lib_types::{
    ValidatorFeePoolAddress,
    constants::{STEALTH_TARI_RESOURCE_ADDRESS, TARI_TOKEN},
    stealth::{SpendAuthorization, StealthTransferStatement},
};

use crate::{
    NUM_PRESHARDS,
    handlers::{
        HandlerContext,
        helpers::{get_account_or_default, get_account_with_inputs, invalid_params, wait_for_result},
    },
};

const LOG_TARGET: &str = "tari::ootle::walletd::handlers::validator";

pub async fn handle_get_validator_fees(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: GetValidatorFeesRequest,
) -> Result<GetValidatorFeesResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    context.authorize(token, &[Permission::Validators(Crud::Read)])?;

    let claim_key = match req.account_or_key {
        AccountOrKeyId::Account(acc) => {
            let account = get_account_or_default(acc.as_ref(), &sdk.accounts_api())?;
            let account_key_id = account.owner_key_id().ok_or_else(|| {
                anyhow!("The specified account does not have an associated owner key to derive the claim key from")
            })?;
            sdk.key_manager_api().get_public_key(account_key_id)?
        },
        AccountOrKeyId::KeyId(key_id) => sdk.key_manager_api().get_public_key(key_id)?,
    };
    let claim_public_key = claim_key.public_key().to_byte_type();

    let shards = req
        .shard_group
        .map(|sg| Either::Left(sg.shard_iter()))
        .unwrap_or_else(|| Either::Right(NUM_PRESHARDS.all_shards_iter()));

    let ids = shards
        .into_iter()
        .map(|shard| {
            derive_fee_pool_address(&claim_public_key, NUM_PRESHARDS, shard)
                .map(SubstateId::from)
                .map_err(|err| invalid_params("shard_group", Some(err)))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut fees = HashMap::with_capacity(ids.len());
    const CHUNK_SIZE: usize = 20;
    for id_chunk in ids.chunks(CHUNK_SIZE) {
        let substates = context
            .wallet_sdk()
            .substate_api()
            .get_substates_from_network(id_chunk.to_vec())
            .await?;

        info!(target: LOG_TARGET, "🔍️ Found {}/{} fee pool substates for claim key {}", substates.len(), CHUNK_SIZE, claim_public_key);

        for (substate_id, substate) in substates {
            let Some(address) = substate_id.as_validator_fee_pool_address() else {
                warn!(target: LOG_TARGET, "Incorrect substate ID found: {}", substate_id);
                continue;
            };

            let Some(amount) = substate.substate_value().as_validator_fee_pool().map(|p| p.amount()) else {
                warn!(target: LOG_TARGET, "Incorrect substate type found at address {}", substate_id);
                continue;
            };

            if amount > 0 {
                let shard = SubstateAddress::from_substate_id(&substate_id, substate.version()).to_shard(NUM_PRESHARDS);
                fees.insert(shard, FeePoolDetails { amount, address });
            }
        }
    }

    Ok(GetValidatorFeesResponse { fees })
}

#[allow(clippy::too_many_lines)]
pub async fn handle_claim_validator_fees(
    context: &HandlerContext,
    token: Option<&Bearer>,
    mut req: ClaimValidatorFeesRequest,
) -> Result<ClaimValidatorFeesResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    context.authorize(token, &[Permission::Validators(Crud::Update)])?;

    if req.shards.is_empty() {
        return Err(invalid_params("shards", Some("At least one shard must be specified")));
    }

    let (account, inputs) = get_account_with_inputs(req.account.as_ref(), &sdk)?;
    let account_key_id = account.owner_key_id().ok_or_else(|| {
        anyhow!("The specified account does not have an associated owner key to derive the claim key from")
    })?;
    let account_component_address = *account.component_address();

    let claim_public_key = match req.claim_key_index {
        Some(index) => sdk
            .key_manager_api()
            .get_public_key(KeyId::derived(KeyBranch::Account, index))?
            .public_key
            .to_byte_type(),
        None => *account.address.account_public_key(),
    };

    req.shards.sort();
    req.shards.dedup();
    let fee_pool_addresses = req
        .shards
        .iter()
        .map(|shard| {
            derive_fee_pool_address(&claim_public_key, NUM_PRESHARDS, *shard)
                .map_err(|err| invalid_params("shards", Some(err)))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // build the transaction
    let max_fee = req.max_fee.max(1);
    let account_public_key = *account.address.account_public_key();

    let builder = context.transaction_builder().await?.with_dry_run(req.dry_run);

    let builder = if req.output_to_revealed {
        let (first, rest) = fee_pool_addresses
            .split_first()
            .ok_or_else(|| invalid_params("shards", Some("At least one shard must be specified")))?;
        let first = *first;
        let rest = rest.to_vec();
        builder
            .with_fee_instructions_builder(move |builder| {
                let builder = builder
                    .create_account(account_public_key)
                    .claim_validator_fees(first)
                    .put_last_instruction_output_on_workspace("joined");
                rest.into_iter()
                    .enumerate()
                    .fold(builder, |b, (i, address)| {
                        let tmp = format!("tmp{i}");
                        b.claim_validator_fees(address)
                            .put_last_instruction_output_on_workspace(tmp.clone())
                            .put_into_bucket(tmp, "joined")
                    })
                    .call_method(account_component_address, "deposit", args![Workspace("joined")])
                    .call_method(account_component_address, "pay_fee", args![max_fee])
            })
            .with_inputs(inputs.into_iter().map(|input| input.into_unversioned()))
    } else {
        let (statement, pool_amounts) =
            build_self_stealth_statement(&sdk, &account, account_key_id, &fee_pool_addresses, max_fee).await?;
        let (first, rest) = pool_amounts
            .split_first()
            .ok_or_else(|| invalid_params("shards", Some("At least one shard must be specified")))?;
        let (first_address, first_amount) = *first;
        let rest = rest.to_vec();
        builder.with_fee_instructions_builder(move |builder| {
            let builder = builder
                .claim_validator_fees_up_to(first_address, first_amount)
                .put_last_instruction_output_on_workspace("joined");
            rest.into_iter()
                .enumerate()
                .fold(builder, |b, (i, (address, amount))| {
                    let tmp = format!("tmp{i}");
                    b.claim_validator_fees_up_to(address, amount)
                        .put_last_instruction_output_on_workspace(tmp.clone())
                        .put_into_bucket(tmp, "joined")
                })
                .take_from_bucket("joined", max_fee, "fee")
                .pay_fee_from_bucket("fee")
                .stealth_transfer_with_input_bucket(TARI_TOKEN, statement, "joined")
        })
    };

    let unsigned_transaction = builder
        .with_inputs(fee_pool_addresses.iter().copied().map(SubstateRequirement::unversioned))
        .map(|builder| {
            if let Some(index) = req.claim_key_index {
                if claim_public_key == account_public_key {
                    Ok(builder.finish())
                } else {
                    // If the claim key is different from the account secret, we need to sign with both
                    sdk.signer_api()
                        .with_context(&account_public_key)
                        .sign(KeyId::derived(KeyBranch::Account, index), builder.finish())
                }
            } else {
                Ok(builder.finish())
            }
        })?;

    let transaction = sdk.signer_api().sign(account_key_id, unsigned_transaction)?;

    // send the transaction
    if req.dry_run {
        let transaction = sdk
            .transaction_api()
            .submit_dry_run_transaction(transaction, None)
            .await?;
        return Ok(ClaimValidatorFeesResponse {
            transaction_id: transaction.id,
            // Dry-run forces a minimal max_fee so the call doesn't require funded vaults, which clamps
            // `total_fees_paid` to that placeholder. Report the uncapped estimate instead.
            fee: transaction
                .finalize
                .as_ref()
                .map(|f| f.required_fees())
                .unwrap_or_default(),
            result: transaction
                .finalize
                .ok_or_else(|| anyhow!("No finalize result for dry run transaction"))?,
        });
    }

    let mut events = context.notifier().subscribe();
    // Link the transaction to the account the fees are claimed into so it appears in the
    // account-filtered transaction list.
    let tx_id = context
        .transaction_service()
        .submit_transaction_with_opts(
            transaction,
            Some(TransactionContext::with_accounts([account_component_address])),
            None,
        )
        .await?;

    let finalized = wait_for_result(&mut events, tx_id).await?;

    if let Some(reject) = finalized.finalize.fee_reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.finalize.any_reject() {
        return Err(anyhow::anyhow!(
            "Fee transaction succeeded (fees charged) however the transaction failed: {reason}",
        ));
    }
    info!(
        target: LOG_TARGET,
        "✅ Claim fee transaction {} finalized. Fee: {}",
        finalized.transaction_id,
        finalized.final_fee
    );

    Ok(ClaimValidatorFeesResponse {
        transaction_id: tx_id,
        fee: finalized.final_fee,
        result: finalized.finalize,
    })
}

/// Fetches each fee pool's current amount, sums them, and builds a single [`StealthTransferStatement`] that converts
/// the consolidated revealed bucket (minus `max_fee` which is carved into a separate revealed bucket on-chain to pay
/// the network fee) into one stealth UTXO addressed to the account's own owner key.
async fn build_self_stealth_statement(
    sdk: &crate::WalletSdk,
    account: &tari_ootle_wallet_sdk::models::AccountWithAddress,
    account_owner_key_id: tari_ootle_wallet_sdk::models::KeyId,
    fee_pool_addresses: &[ValidatorFeePoolAddress],
    max_fee: u64,
) -> Result<(StealthTransferStatement, Vec<(ValidatorFeePoolAddress, u64)>), anyhow::Error> {
    let network = sdk.config_api().get_network()?;
    let account_owner = sdk.key_manager_api().get_public_key(account_owner_key_id)?;
    let view_only = sdk.key_manager_api().get_public_key(account.view_only_key_id())?;

    let substate_ids: Vec<SubstateId> = fee_pool_addresses.iter().copied().map(SubstateId::from).collect();
    let mut amounts: HashMap<ValidatorFeePoolAddress, u64> = HashMap::with_capacity(substate_ids.len());
    const CHUNK_SIZE: usize = 20;
    for chunk in substate_ids.chunks(CHUNK_SIZE) {
        let substates = sdk.substate_api().get_substates_from_network(chunk.to_vec()).await?;
        for (id, substate) in substates {
            let Some(addr) = id.as_validator_fee_pool_address() else {
                continue;
            };
            let Some(amount) = substate.substate_value().as_validator_fee_pool().map(|p| p.amount()) else {
                continue;
            };
            amounts.insert(addr, amount);
        }
    }

    let mut pool_amounts = Vec::with_capacity(fee_pool_addresses.len());
    let mut total: u64 = 0;
    for address in fee_pool_addresses {
        let amount = amounts.get(address).copied().unwrap_or(0);
        if amount == 0 {
            return Err(invalid_params(
                "shards",
                Some(format!("Fee pool {address} is empty or could not be fetched")),
            ));
        }
        total = total
            .checked_add(amount)
            .ok_or_else(|| invalid_params("shards", Some("Total claimable fee pool amount overflows u64")))?;
        pool_amounts.push((*address, amount));
    }

    if total <= max_fee {
        return Err(invalid_params(
            "max_fee",
            Some(format!(
                "max_fee ({max_fee}) is not less than the total claimable fee pool amount ({total}); reduce max_fee \
                 or include shards with larger balances"
            )),
        ));
    }

    let stealth_amount = total - max_fee;

    let memo = Memo::new_message("Validator fees claimed to stealth").expect("valid memo");
    let mask = sdk.key_manager_api().next_key(KeyBranch::StealthMask)?;
    let (nonce, output_public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());

    let encrypted_data = sdk.stealth_crypto_api().encrypt_value_and_mask(
        stealth_amount,
        &mask.key,
        view_only.public_key(),
        &nonce,
        Some(&memo),
    )?;

    let tag = sdk.stealth_crypto_api().derive_stealth_output_tag(
        network,
        &nonce,
        view_only.public_key(),
        &STEALTH_TARI_RESOURCE_ADDRESS,
    );

    let stealth_owner_public_key =
        sdk.stealth_crypto_api()
            .derive_stealth_owner_public_key(network, account_owner.public_key(), &nonce);

    let output_witness = StealthOutputWitness {
        witness: OutputWitness {
            amount: stealth_amount,
            mask: mask.key,
            sender_public_nonce: output_public_nonce,
            minimum_value_promise: 0,
            encrypted_data,
            resource_view_key: None,
        },
        auth: SpendAuthorization::Key(stealth_owner_public_key.to_byte_type()),
        tag,
    };

    let statement = sdk.stealth_crypto_api().generate_transfer_statement(
        iter::empty::<StealthInputWitness>(),
        stealth_amount,
        iter::once(&output_witness),
        0,
    )?;

    Ok((statement, pool_amounts))
}
