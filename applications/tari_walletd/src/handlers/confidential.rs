//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use axum_extra::headers::authorization::Bearer;
use axum_jrpc::error::{JsonRpcError, JsonRpcErrorReason};
use log::*;
use ootle_byte_type::{FromByteType, ToByteType};
use serde_json::json;
use tari_crypto::{commitment::HomomorphicCommitmentFactory, keys::PublicKey as _, ristretto::RistrettoPublicKey};
use tari_engine_types::{
    confidential::MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
    crypto::get_commitment_factory,
    substate::SubstateId,
};
use tari_ootle_common_types::{displayable::Displayable, optional::Optional};
use tari_ootle_wallet_crypto::OutputWitness;
use tari_ootle_wallet_sdk::{
    models::{ConfidentialOutputModel, KeyBranch, OutputStatus},
    storage::{WalletStoreWriter, WriteableWalletStore},
};
use tari_ootle_walletd_client::{
    permissions::{Crud, Permission},
    types::{
        ConfidentialCreateOutputProofRequest,
        ConfidentialCreateOutputProofResponse,
        ConfidentialOutputInfo,
        ConfidentialOutputsListRequest,
        ConfidentialOutputsListResponse,
        ConfidentialViewVaultBalanceRequest,
        ConfidentialViewVaultBalanceResponse,
        ProofsCancelRequest,
        ProofsCancelResponse,
        ProofsFinalizeRequest,
        ProofsFinalizeResponse,
        ProofsGenerateRequest,
        ProofsGenerateResponse,
    },
};
use tari_template_lib_types::{Amount, ConfidentialOutputAddress};
use tokio::{task::spawn_blocking, time::Instant};

use crate::handlers::{
    HandlerContext,
    auth::jwt::enforce_scopes,
    helpers::{get_account_or_default, invalid_params, invalid_request},
    transaction_requests::ensure_free_locks_of,
    value_lookup::{AbortQueuedOnDropGuard, ValueRangeRequest},
};

const LOG_TARGET: &str = "tari::ootle::wallet_daemon::json_rpc::confidential";

/// Largest number of confidential commitments `confidential.view_vault_balance` will recover in one
/// request.
///
/// A vault's commitment set is whatever the indexer returned for the vault the caller named, so this
/// is what bounds one request's fan-out: a `ConfidentialOutput` fetch per commitment, and the output
/// bodies held in memory while they are scanned. The request covers a whole vault in one call, so the
/// ceiling sits well above any realistic vault and a vault that reaches it is refused with an error
/// naming its size.
const MAX_VAULT_COMMITMENTS_PER_REQUEST: usize = 1_000;

#[allow(clippy::too_many_lines)]
pub async fn handle_create_transfer_proof(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ProofsGenerateRequest,
) -> Result<ProofsGenerateResponse, anyhow::Error> {
    let auth = context.check_auth_with_identity(token)?;
    let granted = &auth.permissions;
    let sdk = context.wallet_sdk();

    if req.reveal_amount.is_negative() {
        return Err(invalid_request(format!(
            "Amount to send must be positive. Revealed amount was {}",
            req.reveal_amount
        )));
    }

    let account = get_account_or_default(req.account.as_ref(), &sdk.accounts_api())?;
    enforce_scopes(granted, &[Permission::Confidential(
        Crud::Create,
        Some(*account.component_address()),
    )])?;
    account
        .owner_key_id()
        .ok_or_else(|| invalid_request("Account does not have an owner key"))?;
    let vault = sdk
        .accounts_api()
        .get_vault_by_resource(account.component_address(), &req.resource_address)?;
    let lock = sdk
        .locks_api()
        .create_owned_lock_with_timeout(Duration::from_secs(5 * 60), auth.lock_owner())?;

    let amount_to_transfer = req.confidential_amount.checked_add(req.reveal_amount).ok_or_else(|| {
        invalid_request(format!(
            "Amount to send must be greater than or equal to the amount to reveal. Amount = {}, Revealed = {}",
            req.confidential_amount, req.reveal_amount
        ))
    })?;
    // Lock inputs we're going to spend
    let (inputs, total_input_value) =
        sdk.confidential_outputs_api()
            .lock_outputs_by_amount(lock.id(), &vault.id, amount_to_transfer)?;

    info!(
        target: LOG_TARGET,
        "Locked {} inputs for proof {} worth {} µT",
        inputs.len(),
        lock.id(),
        total_input_value
    );

    // TODO: Any errors from here need to unlock the outputs, ideally just roll back (refactor required but doable).

    // TODO: Wrap up key/encrypted data handling in the wallet SDK
    let output_mask = sdk.key_manager_api().next_key(KeyBranch::ConfidentialMask)?;
    let (nonce_secret, public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());

    let confidential_amount = req.confidential_amount.to_u64_checked().ok_or_else(|| {
        invalid_request(format!(
            "Confidential amount exceeds the maximum value supported in a single UTXO. Amount: {}",
            req.confidential_amount
        ))
    })?;

    // Outputs must be encrypted to the recipient's view key: that is the key a receiving wallet scans with,
    // and any other key leaves it unable to recover the value and mask.
    let destination_view_key: RistrettoPublicKey = req
        .destination_address
        .view_only_key()
        .try_from_byte_type()
        .map_err(|e| invalid_params("destination_address", Some(format!("Invalid view key: {e}"))))?;
    let encrypted_data = sdk.confidential_crypto_api().encrypt_value_and_mask(
        confidential_amount,
        &output_mask.key,
        &destination_view_key,
        &nonce_secret,
        req.output_memo.as_ref(),
    )?;

    let resource = sdk.substate_api().fetch_resource(req.resource_address).await?;
    let resource_view_key = resource.to_view_key_public_key().map_err(|_| {
        JsonRpcError::new(
            JsonRpcErrorReason::InvalidRequest,
            "Invalid resource address".to_string(),
            json!({}),
        )
    })?;

    let change_amount = total_input_value.checked_sub(amount_to_transfer).ok_or_else(|| {
        invalid_request(format!(
            "Insufficient funds to send {}. Total input value = {}",
            amount_to_transfer, total_input_value
        ))
    })?;
    let change_amount_u64 = change_amount.to_u64_checked().ok_or_else(|| {
        invalid_request(format!(
            "Change value exceeds the maximum value supported in a single UTXO. Change: {}. Total input value = {}",
            change_amount, total_input_value
        ))
    })?;

    let output_statement = OutputWitness {
        amount: confidential_amount,
        mask: output_mask.key,
        sender_public_nonce: public_nonce,
        minimum_value_promise: MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
        encrypted_data,
        resource_view_key: resource_view_key.clone(),
    };

    let maybe_change_statement = if change_amount_u64 > 0 {
        let change_mask = sdk.key_manager_api().next_key(KeyBranch::ConfidentialMask)?;
        let (nonce_secret, public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());

        // Change returns to this account, so it is encrypted to this account's own view key, which is the
        // key the output is recorded against below.
        let account_view_key = sdk.key_manager_api().get_key(account.view_only_key_id())?;
        let encrypted_data = sdk.confidential_crypto_api().encrypt_value_and_mask(
            change_amount_u64,
            &change_mask.key,
            &account_view_key.to_public_key(),
            &nonce_secret,
            None,
        )?;

        sdk.confidential_outputs_api().add_output(ConfidentialOutputModel {
            account_address: *account.component_address(),
            vault_id: vault.id,
            commitment: get_commitment_factory()
                .commit_value(&change_mask.key, change_amount_u64)
                .to_byte_type(),
            value: change_amount,
            sender_public_nonce: Some(public_nonce.to_byte_type()),
            view_only_key_id: account.view_only_key_id(),
            owner_key_id: account.owner_key_id(),
            encrypted_data: encrypted_data.clone(),
            memo: None,
            public_asset_tag: None,
            status: OutputStatus::LockedUnconfirmed,
            lock_id: Some(lock.id()),
        })?;

        Some(OutputWitness {
            amount: change_amount_u64,
            mask: change_mask.key,
            sender_public_nonce: public_nonce,
            encrypted_data,
            minimum_value_promise: MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
            resource_view_key,
        })
    } else {
        None
    };

    let inputs = sdk.confidential_outputs_api().resolve_output_masks(inputs)?;

    let proof = sdk.confidential_crypto_api().generate_withdraw_proof(
        &inputs,
        // TODO: support for using revealed funds as input for proof generation
        Amount::zero(),
        Some(&output_statement).filter(|o| o.amount > 0),
        req.reveal_amount,
        maybe_change_statement.as_ref(),
        Amount::zero(),
    )?;

    let lock_id = lock.keep_locked();

    Ok(ProofsGenerateResponse {
        proof_id: lock_id,
        proof,
    })
}

pub async fn handle_finalize_transfer(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ProofsFinalizeRequest,
) -> Result<ProofsFinalizeResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    context.authorize(token, &[Permission::Confidential(Crud::Update, None)])?;
    let transaction = sdk
        .transaction_api()
        .get(req.transaction_id)
        .optional()?
        .ok_or_else(|| {
            invalid_params(
                "transaction_id",
                Some("No such transaction in wallet to finalize proof for"),
            )
        })?;
    let lock_id = sdk
        .locks_api()
        .get_lock_by_transaction_id(req.transaction_id)
        .optional()?;
    if lock_id != Some(req.lock_id) {
        return Err(invalid_params(
            "lock_id",
            Some("Lock not associated with this transaction"),
        ));
    }

    match transaction.finalized_diff() {
        Some(diff) => {
            info!(
                target: LOG_TARGET,
                "Finalizing locked proof {} for transaction {}",
                req.lock_id,
                req.transaction_id
            );
            sdk.locks_api().finalize_lock(req.lock_id, diff)?;
        },
        None => {
            return Err(invalid_params(
                "transaction_id",
                Some(format!(
                    "Transaction is not finalized (status = {}, reason = {})",
                    transaction.status,
                    transaction.failure_reason_as_string().display()
                )),
            ));
        },
    }

    Ok(ProofsFinalizeResponse {})
}

pub async fn handle_cancel_transfer(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ProofsCancelRequest,
) -> Result<ProofsCancelResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    let auth = context.authorize_with_identity(token, &[Permission::Confidential(Crud::Delete, None)])?;
    sdk.store().with_write_tx(|tx| {
        ensure_free_locks_of(&mut **tx, auth.lock_owner(), &[req.proof_id], "proof_id")?;
        tx.locks_release(req.proof_id)?;
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(ProofsCancelResponse {})
}

pub async fn handle_create_output_proof(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ConfidentialCreateOutputProofRequest,
) -> Result<ConfidentialCreateOutputProofResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    context.authorize(token, &[Permission::Confidential(Crud::Create, None)])?;

    if req.amount < MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE {
        return Err(invalid_params(
            "amount",
            Some(format!(
                "A confidential output must hold at least {MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE}"
            )),
        ));
    }

    let destination_view_key: RistrettoPublicKey = req
        .destination_address
        .view_only_key()
        .try_from_byte_type()
        .map_err(|e| invalid_params("destination_address", Some(format!("Invalid view key: {e}"))))?;
    let resource = sdk.substate_api().fetch_resource(req.resource_address).await?;
    let resource_view_key = resource
        .to_view_key_public_key()
        .map_err(|e| invalid_params("resource_address", Some(format!("Invalid resource view key: {e}"))))?;

    let output_mask = sdk.key_manager_api().next_key(KeyBranch::ConfidentialMask)?;
    let (nonce_secret, public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let encrypted_data = sdk.confidential_crypto_api().encrypt_value_and_mask(
        req.amount,
        &output_mask.key,
        &destination_view_key,
        &nonce_secret,
        req.output_memo.as_ref(),
    )?;

    let statement = OutputWitness {
        amount: req.amount,
        mask: output_mask.key,
        sender_public_nonce: public_nonce,
        minimum_value_promise: MIN_CONFIDENTIAL_OUTPUT_VALUE_PROMISE,
        encrypted_data,
        resource_view_key,
    };
    let proof = sdk
        .confidential_crypto_api()
        .generate_output_proof(&statement, Amount::zero())?;
    Ok(ConfidentialCreateOutputProofResponse { proof })
}

pub async fn handle_list_outputs(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ConfidentialOutputsListRequest,
) -> Result<ConfidentialOutputsListResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Confidential(Crud::Read, req.account_address)])?;

    let outputs = context.wallet_sdk().confidential_outputs_api().outputs_get_many(
        &req.resource_address,
        req.account_address.as_ref(),
        req.filter_by_status,
    )?;

    Ok(ConfidentialOutputsListResponse {
        outputs: outputs
            .into_iter()
            .map(|o| ConfidentialOutputInfo {
                address: ConfidentialOutputAddress::new(req.resource_address, o.commitment),
                vault_id: o.vault_id,
                value: o.value,
                status: o.status,
                memo: o.memo,
            })
            .collect(),
    })
}

pub async fn handle_view_vault_balance(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: ConfidentialViewVaultBalanceRequest,
) -> Result<ConfidentialViewVaultBalanceResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    context.authorize(token, &[Permission::Confidential(Crud::Read, None)])?;

    let substate = sdk
        .substate_api()
        .fetch_substate_from_network(&req.vault_id.into(), None)
        .await?;
    let vault = substate
        .substate
        .as_vault()
        .ok_or_else(|| anyhow::anyhow!("Indexer returned a non-vault substate when scanning for a vault address"))?;

    let commitments = vault
        .get_confidential_commitments()
        .ok_or_else(|| invalid_params("vault_id", Some("Vault does not contain a confidential resource")))?;
    if commitments.len() > MAX_VAULT_COMMITMENTS_PER_REQUEST {
        return Err(invalid_params(
            "vault_id",
            Some(format!(
                "This vault holds {} confidential commitments; at most {MAX_VAULT_COMMITMENTS_PER_REQUEST} can be \
                 viewed in one request",
                commitments.len(),
            )),
        ));
    }
    let resource_address = *vault.resource_address();

    // Get view secret key
    let view_key = sdk.key_manager_api().get_elgamal_encrypted_view_key(req.view_key_id)?;

    // Confidential OutputBodies live in separate ConfidentialOutput substates, so they are fetched in one batch
    // by their commitment-derived addresses to obtain the viewable-balance proofs.
    let addresses = commitments
        .iter()
        .map(|commitment| SubstateId::ConfidentialOutput(ConfidentialOutputAddress::new(resource_address, *commitment)))
        .collect::<Vec<_>>();
    let mut substates = sdk.substate_api().get_substates_from_network(addresses).await?;

    let mut viewable = Vec::new();
    for commitment in commitments {
        let id = SubstateId::ConfidentialOutput(ConfidentialOutputAddress::new(resource_address, *commitment));
        // An output can be spent between the vault fetch and this fetch; it then no longer exists and no
        // longer contributes to the balance, so it is skipped rather than failing the whole view.
        let Some(substate) = substates.remove(&id) else {
            warn!(
                target: LOG_TARGET,
                "Confidential output {} was not returned by the indexer. Skipping.",
                id
            );
            continue;
        };
        let output = substate
            .into_substate_value()
            .into_confidential_output()
            .ok_or_else(|| anyhow::anyhow!("Indexer returned a non-confidential-output substate for {}", id))?;
        if let Some(proof) = output.into_output().viewable_balance {
            viewable.push((*commitment, proof));
        }
    }

    let elgamal_proofs = viewable.iter().map(|(_, proof)| proof.clone()).collect::<Vec<_>>();
    let value_range = ValueRangeRequest::resolve(req.minimum_expected_value, req.maximum_expected_value);

    let timer = Instant::now();
    let lookup_file = context.config().value_lookup_table_file.clone();
    let scan_sdk = sdk.clone();
    let handle = spawn_blocking(move || {
        crate::handlers::value_lookup::brute_force_viewable_balances(
            &scan_sdk.viewable_balance_api(),
            lookup_file.as_deref(),
            &view_key.key,
            &elgamal_proofs,
            value_range,
        )
    });
    let _drop_guard = AbortQueuedOnDropGuard::new(handle.abort_handle());
    let recovery = handle.await??;

    info!(target: LOG_TARGET, "Brute force balance lookup took {:.2?}", timer.elapsed());

    Ok(ConfidentialViewVaultBalanceResponse {
        balances: viewable
            .iter()
            .map(|(commitment, _)| *commitment)
            .zip(recovery.balances)
            .collect(),
        searched: recovery.searched,
    })
}

#[cfg(test)]
mod tests {
    use tari_engine_types::resource::Resource;
    use tari_ootle_address::{Network, OotleAddress};
    use tari_ootle_common_types::{Epoch, optional::IsNotFoundError};
    use tari_ootle_transaction::UnsignedTransaction;
    use tari_ootle_wallet_sdk::{
        crypto::memo::Memo,
        models::{KeyId, LockOwner, WalletLockId},
        storage::{ReadableWalletStore, WalletStoreReader},
    };
    use tari_ootle_walletd_client::types::TransactionRequestCreateRequest;
    use tari_template_lib_types::{
        Metadata,
        ResourceAddress,
        ResourceType,
        SubstateOwnerRule,
        access_rules::ResourceAccessRules,
    };

    use super::*;
    use crate::handlers::test_support::TestDaemon;

    fn lock_for(daemon: &TestDaemon, owner: LockOwner) -> WalletLockId {
        daemon
            .context
            .wallet_sdk()
            .locks_api()
            .create_owned_lock_with_timeout(Duration::from_secs(5 * 60), owner)
            .unwrap()
            .keep_locked()
    }

    fn lock_exists(daemon: &TestDaemon, lock_id: WalletLockId) -> bool {
        match daemon
            .context
            .wallet_sdk()
            .store()
            .with_read_tx(|tx| tx.locks_get(lock_id))
        {
            Ok(_) => true,
            Err(e) if e.is_not_found_error() => false,
            Err(e) => panic!("{e}"),
        }
    }

    #[tokio::test]
    async fn a_proof_can_only_be_cancelled_by_its_creator_while_nothing_holds_it() {
        let daemon = TestDaemon::start().await;
        let tool = daemon.api_key("proof-tool", "confidential:delete,transaction_requests:create");
        let other_tool = daemon.api_key("other-tool", "confidential:delete");
        let owner = |bearer| {
            daemon
                .context
                .check_auth_with_identity(Some(bearer))
                .unwrap()
                .lock_owner()
        };
        let cancel =
            |bearer, proof_id| handle_cancel_transfer(&daemon.context, Some(bearer), ProofsCancelRequest { proof_id });

        let wallets = lock_for(&daemon, LockOwner::Wallet);
        let others = lock_for(&daemon, owner(&other_tool));
        for lock_id in [wallets, others, 9999] {
            let err = cancel(&tool, lock_id).await.unwrap_err();
            assert!(err.to_string().contains("proof_id"), "{err}");
        }
        assert!(lock_exists(&daemon, wallets) && lock_exists(&daemon, others));

        let requested = lock_for(&daemon, owner(&tool));
        crate::handlers::transaction_requests::handle_create(
            &daemon.context,
            Some(&tool),
            TransactionRequestCreateRequest {
                transaction: UnsignedTransaction::new(Network::LocalNet.as_byte(), Epoch(100)),
                seal_signer: KeyId::derived(KeyBranch::Account, 0),
                other_signers: vec![],
                signatures: vec![],
                lock_ids: vec![requested],
                ttl_secs: Some(60 * 60),
            },
        )
        .await
        .unwrap();
        cancel(&tool, requested).await.unwrap_err();
        assert!(lock_exists(&daemon, requested), "a live request's lock was released");

        let own = lock_for(&daemon, owner(&tool));
        cancel(&tool, own).await.unwrap();
        assert!(!lock_exists(&daemon, own));
    }

    #[tokio::test]
    async fn an_output_proof_is_recoverable_by_the_destination_view_key() {
        let daemon = TestDaemon::start().await;
        let sdk = daemon.context.wallet_sdk();
        let view_key = sdk.key_manager_api().next_key(KeyBranch::ViewOnlyKey).unwrap();
        let account_key = sdk.key_manager_api().next_key(KeyBranch::Account).unwrap();
        let (_, resource_view_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let resource_address: ResourceAddress =
            "resource_0000000000000000000000000000000000000000000000000000000000000001"
                .parse()
                .unwrap();
        sdk.resources_api()
            .upsert_resource(
                &resource_address,
                &Resource::new(
                    ResourceType::Confidential,
                    SubstateOwnerRule::None,
                    ResourceAccessRules::new(),
                    Metadata::new(),
                    Some(resource_view_key.to_byte_type()),
                    None,
                    6,
                    false,
                ),
            )
            .unwrap();
        let memo = Memo::new_message("for you").unwrap();

        let resp = handle_create_output_proof(
            &daemon.context,
            Some(&daemon.session),
            ConfidentialCreateOutputProofRequest {
                amount: 1_000,
                resource_address,
                destination_address: OotleAddress::new(
                    Network::LocalNet,
                    view_key.to_public_key().to_byte_type(),
                    account_key.to_public_key().to_byte_type(),
                ),
                output_memo: Some(memo.clone()),
            },
        )
        .await
        .unwrap();

        let output = resp.proof.output.unwrap();
        assert!(output.viewable_balance_proof.is_some());
        let nonce: RistrettoPublicKey = output.sender_public_nonce.try_from_byte_type().unwrap();
        let decrypted = sdk
            .confidential_crypto_api()
            .unblind_output(&output.commitment, &output.encrypted_data, &view_key.key, &nonce, false)
            .unwrap();
        assert_eq!(decrypted.value(), 1_000);
        assert_eq!(decrypted.memo, Some(memo));
    }
}
