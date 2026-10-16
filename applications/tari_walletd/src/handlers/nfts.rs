//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, slice};

use anyhow::anyhow;
use axum_extra::headers::authorization::Bearer;
use log::{info, warn};
use ootle_byte_type::ToByteType;
use tari_engine_types::{
    component::derive_component_address_from_public_key,
    json_cbor::convert_json_to_cbor,
    substate::SubstateId,
};
use tari_ootle_common_types::{InputDeclaration, SubstateRequirement, optional::Optional};
use tari_ootle_transaction::args;
use tari_ootle_wallet_sdk::{apis::substate::ValidatorScanResult, models::TransactionContext};
use tari_ootle_walletd_client::{
    permissions::{Crud, Permission},
    types::{
        GetNftRequest,
        GetNftResponse,
        ListNftsRequest,
        ListNftsResponse,
        MintFaucetNftRequest,
        MintFaucetNftResponse,
        TransferNftRequest,
        TransferNftResponse,
    },
};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib_types::{
    ComponentAddress,
    ResourceAddress,
    constants::{NFT_FAUCET_COMPONENT_ADDRESS, NFT_FAUCET_RESOURCE_ADDRESS},
};

use super::{context::HandlerContext, helpers::get_account_or_default};
use crate::{
    handlers::{
        auth::jwt::enforce_scopes,
        helpers::{
            application_error,
            get_account,
            get_account_with_inputs,
            invalid_params,
            wait_for_result_and_account,
        },
    },
    server::ApplicationErrorCode,
};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::handlers::nfts";

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: GetNftRequest,
) -> Result<GetNftResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    context.authorize(token, &[Permission::Nfts(Crud::Read, Some(req.resource_address))])?;

    let non_fungible_api = sdk.non_fungible_api();

    let non_fungible = non_fungible_api
        .get(req.resource_address, req.nft_id)
        .map_err(|e| anyhow!("Failed to get non fungible token, with error: {}", e))?;

    Ok(non_fungible)
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ListNftsRequest,
) -> Result<ListNftsResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Nfts(Crud::Read, None)])?;
    let ListNftsRequest { account, limit, offset } = req;
    let sdk = context.wallet_sdk();
    let account = get_account_or_default(account.as_ref(), &sdk.accounts_api())?;
    let account = account.account;

    let non_fungible_api = sdk.non_fungible_api();

    let non_fungibles = non_fungible_api
        .get_all(account.component_address, limit.into(), offset.into())
        .map_err(|e| anyhow!("Failed to list all non fungibles, with error: {}", e))?;
    Ok(ListNftsResponse { nfts: non_fungibles })
}

pub async fn handle_mint_faucet(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: MintFaucetNftRequest,
) -> Result<MintFaucetNftResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    // Faucet operation: testnet-only mass-mint. Stays admin-only.
    context.authorize(token, &[Permission::Admin])?;

    let account = get_account(&req.account, &sdk.accounts_api())?;
    let account = account.account;

    let account_owner_key_id = account
        .owner_key_id
        .ok_or_else(|| invalid_params("account", Some("The account does not have an owner key ID")))?;

    info!(target: LOG_TARGET, "🎮 Minting new NFT with metadata {}", req.mutable_data);

    let mutable_data = convert_json_to_cbor(req.mutable_data).map_err(|e| invalid_params("mutable_data", Some(e)))?;

    if req.number_to_mint == 0 {
        return Err(invalid_params("number_to_mint", Some("number_to_mint is zero")));
    }

    let inputs = sdk
        .substate_api()
        .locate_dependent_substates(slice::from_ref(&account.component_address.into()), true)
        .await?;
    // Minting writes the NFT, its resource and the account's vault, and pays for the receipt.
    // The default is set well clear of that; the unspent remainder is refunded.
    let fee = req.max_fee.unwrap_or(50_000);
    context.enforce_max_fee_limit(fee)?;
    let transaction = context
        .transaction_builder()
        .await?
        .pay_fee_from_component(account.component_address, fee)
        .call_method(NFT_FAUCET_COMPONENT_ADDRESS, "mint", args![
            req.number_to_mint,
            mutable_data
        ])
        .put_last_instruction_output_on_workspace("tokens")
        .call_method(account.component_address, "deposit", args![Workspace("tokens")])
        .with_inputs(
            inputs
                .into_iter()
                .map(|input| InputDeclaration::write(input.into_substate_id())),
        )
        .add_input(NFT_FAUCET_COMPONENT_ADDRESS)
        .add_input(NFT_FAUCET_RESOURCE_ADDRESS)
        .finish();

    let transaction = sdk.signer_api().sign(account_owner_key_id, transaction)?;

    let mut events = context.notifier().subscribe();
    // Link the transaction to the account receiving the minted NFTs so it appears in the
    // account-filtered transaction list.
    let tx_id = context
        .transaction_service()
        .submit_transaction_with_opts(
            transaction,
            Some(TransactionContext::with_accounts([account.component_address])),
            None,
        )
        .await?;

    // Wait for the account monitor to observe the newly deposited NFT before returning.
    let (finalize_event, _) = wait_for_result_and_account(&mut events, &tx_id, &account.component_address).await?;
    if let Some(reject) = finalize_event.finalize.any_reject() {
        return Err(application_error(
            ApplicationErrorCode::TransactionRejected,
            format!("Mint new NFT using account {} was rejected: {}", account, reject),
        ));
    }

    Ok(MintFaucetNftResponse {
        transaction_id: tx_id,
        finalize: finalize_event.finalize,
        fee: finalize_event.final_fee,
    })
}

async fn try_find_target_account(
    context: &HandlerContext,
    inputs: &mut HashSet<SubstateRequirement>,
    target_account_address: ComponentAddress,
    target_resource_address: ResourceAddress,
) -> anyhow::Result<bool> {
    let sdk = context.wallet_sdk();
    let existing_account = sdk
        .substate_api()
        .fetch_substate_from_network(&SubstateId::Component(target_account_address), None)
        .await
        .optional()?;

    let Some(ValidatorScanResult { id: address, substate }) = existing_account else {
        return Ok(false);
    };
    inputs.insert(address.into());

    // Figure out which vault to add as an input
    let Some(component) = substate.component() else {
        return Err(anyhow::anyhow!(
            "The target account {} is not a component. This is unexpected.",
            target_account_address
        ));
    };
    let indexed = component.body.to_indexed_well_known_types()?;
    let mut found_dest_vault = None;
    for vault_id in indexed.vault_ids() {
        // Local vault?
        match sdk.accounts_api().get_vault(vault_id).optional()? {
            Some(vault) => {
                if vault.resource_address != target_resource_address {
                    // Continue searching for a vault for the resource address
                    continue;
                }
                // Found it - we're sending to our own vault
                found_dest_vault = Some(*vault_id);
                break;
            },
            None => {
                // TODO(perf): slow with lots of vaults
                let vault = sdk
                    .substate_api()
                    .fetch_substate_from_network(&SubstateId::Vault(*vault_id), None)
                    .await
                    .optional()?;

                let Some(vault) = vault.and_then(|scan| scan.substate.into_vault()) else {
                    warn!(
                        target: LOG_TARGET,
                        "❓️ The target account {target_account_address} contains a vault {vault_id} that was not found. This is unexpected.",
                    );
                    continue;
                };

                if *vault.resource_address() != target_resource_address {
                    // Continue searching for a vault for the resource address
                    continue;
                }

                // Found it
                found_dest_vault = Some(*vault_id);
                break;
            },
        }
    }

    if let Some(found) = found_dest_vault {
        inputs.insert(SubstateRequirement::unversioned(found));
    }

    Ok(true)
}

#[allow(clippy::too_many_lines)]
pub async fn handle_transfer(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TransferNftRequest,
) -> Result<TransferNftResponse, anyhow::Error> {
    let granted = context.check_auth(token)?;
    if !req.dry_run {
        context.enforce_max_fee_limit(req.max_fee)?;
    }
    let sdk = context.wallet_sdk();

    // fetch accounts and its inputs
    let (source_account, mut inputs) = get_account_with_inputs(Some(&req.source_account), sdk)?;
    enforce_scopes(&granted, &[Permission::Transfer(
        Crud::Create,
        Some(*source_account.component_address()),
    )])?;
    // The fee payer's key signs the transaction and its account pays the fee, so the caller needs the same
    // transfer scope on it as on the source account.
    let (fee_payer_account, fee_payer_account_inputs) = get_account_with_inputs(Some(&req.fee_payer_account), sdk)?;
    enforce_scopes(&granted, &[Permission::Transfer(
        Crud::Create,
        Some(*fee_payer_account.component_address()),
    )])?;
    let fee_payer_account = fee_payer_account.account;
    let fee_payer_key_id = fee_payer_account.owner_key_id.ok_or_else(|| {
        invalid_params(
            "fee_payer_account",
            Some("The fee payer account does not have an owner key ID"),
        )
    })?;
    let fee_payer_account_address = fee_payer_account.component_address;
    let account_owner_key_id = source_account.account.owner_key_id().ok_or_else(|| {
        invalid_params(
            "source_account",
            Some("The source account does not have an owner key ID"),
        )
    })?;
    inputs.extend(fee_payer_account_inputs);
    let source_account_address = *source_account.component_address();

    let target_account_address = derive_component_address_from_public_key(
        &ACCOUNT_TEMPLATE_ADDRESS,
        req.target_account_address.account_public_key(),
    );

    // TODO: this can be simplified
    let mut builder = context.transaction_builder().await?;
    // collect all instructions
    let non_fungible_api = sdk.non_fungible_api();

    if !try_find_target_account(context, &mut inputs, target_account_address, req.resource_address).await? {
        // We need to create the target account
        builder = builder.create_account(*req.target_account_address.account_public_key())
    }
    // add the input for the source account vault substate
    let src_vault = sdk
        .accounts_api()
        .get_vault_by_resource(source_account.component_address(), &req.resource_address)?;
    let src_vault_substate = sdk.substate_api().get_substate(&src_vault.id.into())?;
    inputs.insert(src_vault_substate.substate_id.into());
    inputs.insert(SubstateRequirement::unversioned(src_vault.resource_address));

    let nfts = req.nfts;
    // Validate every NFT exists on the source wallet before emitting any instructions.
    for nft_id in &nfts {
        let nft = non_fungible_api
            .get(req.resource_address, nft_id.clone())
            .optional()
            .map_err(|e| anyhow!("Failed to get non-fungible token: {}", e))?;
        if nft.is_none() {
            return Err(invalid_params(
                "nft_id",
                Some(format!(
                    "NFT with ID {} not found for resource {}",
                    nft_id, req.resource_address
                )),
            ));
        }
    }

    // Withdraw each NFT into a workspace bucket, merge them all into "joined" via put_into_bucket,
    // and deposit once on the target account. All NFTs share `req.resource_address`, so the join
    // is always resource-compatible. With N NFTs this trades (N-1) template `deposit` calls for
    // (N-1) native engine ops.
    let mut iter = nfts.into_iter();
    if let Some(first_nft) = iter.next() {
        builder = builder
            .call_method(source_account_address, "withdraw_non_fungible", args![
                req.resource_address,
                first_nft,
            ])
            .put_last_instruction_output_on_workspace("joined");
        for (i, nft_id) in iter.enumerate() {
            let tmp = format!("tmp{i}");
            builder = builder
                .call_method(source_account_address, "withdraw_non_fungible", args![
                    req.resource_address,
                    nft_id,
                ])
                .put_last_instruction_output_on_workspace(tmp.clone())
                .put_into_bucket(tmp, "joined");
        }
        builder = builder.call_method(target_account_address, "deposit", args![Workspace("joined")]);
    }

    let fee_owner_key = sdk.key_manager_api().get_public_key(fee_payer_key_id)?;

    let transaction = builder
        .with_dry_run(req.dry_run)
        .pay_fee_from_component(fee_payer_account_address, req.max_fee)
        .with_inputs(
            inputs
                .into_iter()
                .map(|input| InputDeclaration::write(input.into_substate_id())),
        )
        .finish();

    let transaction = sdk
        .signer_api()
        .with_context(&fee_owner_key.public_key().to_byte_type())
        .sign(account_owner_key_id, transaction)?;

    let transaction = sdk.signer_api().sign(fee_payer_key_id, transaction)?;

    // if dry run, we can return the result immediately
    if req.dry_run {
        let execute_result = context
            .transaction_service()
            .submit_dry_run_transaction(transaction)
            .await?;
        let transaction_id = execute_result.finalize.transaction_hash.into();
        let finalize = execute_result.finalize;
        return Ok(TransferNftResponse {
            transaction_id,
            // Dry-run callers commonly pass a placeholder max_fee that clamps `total_fees_paid`. Report
            // the uncapped estimate so the response is usable for fee selection.
            fee: finalize.required_fees(),
            fee_refunded: finalize
                .fee_receipt
                .total_fee_payment()
                .saturating_sub(finalize.fee_receipt.total_fees_paid()),
            result: finalize,
        });
    }

    // execute transaction
    // Link the source and fee payer accounts, and the target too if it is one of the wallet's own
    // accounts (so a self-transfer shows under both).
    let mut linked_accounts = vec![source_account_address];
    if !linked_accounts.contains(&fee_payer_account_address) {
        linked_accounts.push(fee_payer_account_address);
    }
    if !linked_accounts.contains(&target_account_address) &&
        sdk.accounts_api()
            .get_account_by_address(&target_account_address)
            .optional()?
            .is_some()
    {
        linked_accounts.push(target_account_address);
    }
    let mut events = context.notifier().subscribe();
    let tx_id = context
        .transaction_service()
        .submit_transaction_with_opts(
            transaction,
            Some(TransactionContext::with_accounts(linked_accounts.clone())),
            None,
        )
        .await?;

    let finalized = crate::handlers::helpers::wait_for_result(&mut events, tx_id).await?;

    if let Some(reject) = finalized.finalize.result.fee_reject() {
        return Err(anyhow::anyhow!("Fee transaction rejected: {}", reject));
    }
    if let Some(reason) = finalized.finalize.any_reject() {
        return Err(anyhow::anyhow!(
            "Fee transaction succeeded (fees charged) however the transaction failed: {}",
            reason
        ));
    }
    info!(
        target: LOG_TARGET,
        "✅ Transferring NFT transaction {} finalized. Fee: {}",
        finalized.transaction_id,
        finalized.final_fee
    );

    // `wait_for_result` returns on the finalize event that the account monitor also consumes to update the
    // local vault and NFT records, so those records are not yet current here. Refreshing each account the
    // transfer touched serialises behind the monitor's work, so a caller that re-reads `nfts.list` or the
    // account balances on return sees the post-transfer state.
    for account_address in linked_accounts {
        if let Err(err) = context.account_monitor().refresh_account(account_address).await {
            warn!(
                target: LOG_TARGET,
                "Failed to refresh account {} after NFT transfer {}: {}", account_address, tx_id, err
            );
        }
    }

    Ok(TransferNftResponse {
        transaction_id: tx_id,
        fee: finalized.final_fee,
        fee_refunded: req.max_fee.saturating_sub(finalized.final_fee),
        result: finalized.finalize,
    })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use axum_extra::headers::Authorization;
    use tari_ootle_address::Network;
    use tari_ootle_common_types::Epoch;
    use tari_ootle_wallet_sdk::{
        IndexerTrust,
        WalletSdkConfig,
        cipher_seed::CipherSeedRestore,
        models::{EpochBirthday, KeyBranch, KeyId},
    };
    use tari_ootle_wallet_sdk_services::{
        account_monitor::AccountMonitor,
        indexer_rest_api::IndexerRestApiNetworkInterface,
        notify::Notify,
        transaction_service::TransactionService,
        utxo_scanner::StealthUtxoScannerWorker,
    };
    use tari_ootle_wallet_storage_sqlite::SqliteWalletStore;
    use tari_ootle_walletd_client::{ComponentAddressOrName, permissions::Permissions};
    use tari_shutdown::Shutdown;
    use tari_utilities::SafePassword;

    use super::*;
    use crate::{
        WalletSdk,
        config::{WalletDaemonAuth, WalletDaemonConfig},
        handlers::{
            HandlerContext,
            auth::{create_authenticator, jwt::AuthError},
        },
    };

    /// A handler context holding two wallet accounts. The indexer URL points at a closed port, so a request that gets
    /// past authorization fails fast on its first network call. `_temp` keeps the SQLite directory alive.
    struct TransferTest {
        context: HandlerContext,
        account_a: ComponentAddress,
        account_b: ComponentAddress,
        _temp: tempfile::TempDir,
    }

    impl TransferTest {
        fn bearer(&self, permissions: &str) -> Bearer {
            let permissions = Permissions::from_str(permissions).unwrap();
            let claims = self.context.jwt_api().generate_auth_claims(permissions).unwrap();
            let token = self.context.jwt_api().grant(&claims).unwrap();
            Authorization::<Bearer>::bearer(&token).unwrap().0
        }

        fn request(&self, source: ComponentAddress, fee_payer: ComponentAddress) -> TransferNftRequest {
            let target = get_account(&self.account_a.into(), &self.context.wallet_sdk().accounts_api())
                .unwrap()
                .address()
                .clone();
            TransferNftRequest {
                resource_address: NFT_FAUCET_RESOURCE_ADDRESS,
                nfts: vec![],
                fee_payer_account: ComponentAddressOrName::ComponentAddress(fee_payer),
                source_account: ComponentAddressOrName::ComponentAddress(source),
                target_account_address: target,
                max_fee: 1000,
                dry_run: true,
            }
        }
    }

    async fn setup() -> TransferTest {
        let temp = tempfile::tempdir().unwrap();
        let store = SqliteWalletStore::try_open(temp.path().join("wallet.sqlite")).unwrap();
        store.run_migrations().unwrap();
        let mut sdk = WalletSdk::initialize_with_local_key_store(
            store.clone(),
            IndexerRestApiNetworkInterface::new("http://127.0.0.1:1"),
            WalletSdkConfig {
                network: Network::LocalNet,
                override_keyring_password: Some(SafePassword::from_str("test wallet password").unwrap()),
                indexer_trust: IndexerTrust::default(),
            },
            EpochBirthday::far_future(),
        )
        .unwrap();
        sdk.initialize_cipher_seed(CipherSeedRestore::CreateNewIfRequired)
            .unwrap();

        let account_a = ComponentAddress::from_array([0xaa; 32]);
        let account_b = ComponentAddress::from_array([0xbb; 32]);
        for (index, (name, address)) in [("a", account_a), ("b", account_b)].into_iter().enumerate() {
            sdk.accounts_api()
                .add_account(
                    Some(name),
                    &address,
                    KeyId::derived(KeyBranch::ViewOnlyKey, index as u64),
                    KeyId::derived(KeyBranch::Account, index as u64),
                    Epoch::zero(),
                    false,
                    index == 0,
                )
                .unwrap();
        }

        let notify = Notify::new(10);
        let shutdown = Shutdown::new();
        let (transaction_service, transaction_service_handle) =
            TransactionService::new(notify.clone(), sdk.clone(), shutdown.to_signal());
        let (utxo_worker, utxo_scanner_handle) = StealthUtxoScannerWorker::new(sdk.clone(), notify.clone()).spawn();
        let (account_monitor, account_monitor_handle) =
            AccountMonitor::new(notify.clone(), sdk.clone(), utxo_scanner_handle, shutdown.to_signal());
        let mut config = WalletDaemonConfig::default();
        config.network = Network::LocalNet;
        config.authentication = WalletDaemonAuth::None;
        let context = HandlerContext::new(
            sdk,
            notify,
            transaction_service_handle,
            account_monitor_handle,
            config.clone(),
            create_authenticator(&config, store).unwrap(),
            SafePassword::from_str("test jwt secret").unwrap(),
            shutdown.to_signal(),
        );

        // The background workers are not used by this handler; shut them down now.
        shutdown.trigger();
        drop(account_monitor);
        drop(transaction_service);
        utxo_worker.abort();
        drop(utxo_worker.await);

        TransferTest {
            context,
            account_a,
            account_b,
            _temp: temp,
        }
    }

    #[tokio::test]
    async fn transfer_requires_transfer_scope_on_the_fee_payer() {
        let test = setup().await;
        let bearer = test.bearer(&format!("transfer:create:{}", test.account_a));

        let err = handle_transfer(
            &test.context,
            Some(&bearer),
            test.request(test.account_a, test.account_b),
        )
        .await
        .expect_err("a fee payer outside the token's scope must be rejected");
        let auth_err = err
            .downcast_ref::<AuthError>()
            .unwrap_or_else(|| panic!("expected an AuthError, got: {err}"));
        assert!(
            matches!(
                auth_err,
                AuthError::InsufficientPermissions {
                    required: Permission::Transfer(Crud::Create, Some(addr)),
                } if *addr == test.account_b
            ),
            "the missing scope should be transfer:create on the fee payer: {auth_err}"
        );
    }

    #[tokio::test]
    async fn transfer_accepts_a_fee_payer_within_scope() {
        let test = setup().await;

        for (permissions, fee_payer) in [
            (format!("transfer:create:{}", test.account_a), test.account_a),
            (
                format!("transfer:create:{},transfer:create:{}", test.account_a, test.account_b),
                test.account_b,
            ),
        ] {
            let bearer = test.bearer(&permissions);
            let err = handle_transfer(&test.context, Some(&bearer), test.request(test.account_a, fee_payer))
                .await
                .expect_err("the offline indexer fails the request after authorization");
            assert!(
                err.downcast_ref::<AuthError>().is_none(),
                "fee payer {fee_payer} is within scope of '{permissions}', got: {err}"
            );
        }
    }
}
