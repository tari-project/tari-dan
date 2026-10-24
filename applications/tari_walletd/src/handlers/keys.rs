//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum_extra::headers::authorization::Bearer;
use ootle_byte_type::ToByteType;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey, tari_utilities::hex::Hex};
use tari_ootle_wallet_sdk::{
    apis::key_manager::KeyManagerApiError,
    models::{KeyBranch, KeyId},
};
use tari_ootle_walletd_client::{
    permissions::{Crud, Permission},
    types::{
        KeysCreateRequest,
        KeysCreateResponse,
        KeysImportRequest,
        KeysImportResponse,
        KeysListRequest,
        KeysListResponse,
        KeysSetActiveRequest,
        KeysSetActiveResponse,
    },
};

use super::{
    context::HandlerContext,
    helpers::{invalid_params, invalid_request},
};

pub async fn handle_create(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: KeysCreateRequest,
) -> Result<KeysCreateResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    context.authorize(token, &[Permission::Keys(Crud::Create)])?;
    ensure_not_mask_branch(req.branch)?;
    let key_manager = sdk.key_manager_api();
    let key = req
        .specific_index
        .map(|idx| key_manager.get_public_key(KeyId::derived(req.branch, idx)))
        .unwrap_or_else(|| key_manager.next_public_key(req.branch))?;
    Ok(KeysCreateResponse {
        id: key.key_id.derived_index().expect("Key is derived"),
        public_key: key.public_key.to_byte_type(),
    })
}

pub async fn handle_import(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: KeysImportRequest,
) -> Result<KeysImportResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    context.authorize(token, &[Permission::Keys(Crud::Create)])?;
    let public_key = RistrettoPublicKey::from_secret_key(&req.secret_key);
    let key_id = sdk
        .key_manager_api()
        .import_key(&public_key.to_hex(), &req.secret_key, req.key_type)
        .map_err(|e| match e {
            KeyManagerApiError::KeyAlreadyImportedWithDifferentType { .. } => invalid_request(e),
            e => e.into(),
        })?;
    Ok(KeysImportResponse {
        key_id,
        public_key: public_key.to_byte_type(),
    })
}

pub async fn handle_list(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: KeysListRequest,
) -> Result<KeysListResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    context.authorize(token, &[Permission::Keys(Crud::Read)])?;
    ensure_not_mask_branch(req.branch)?;
    let keys = sdk.key_manager_api().get_all_derived_keys(req.branch)?;
    Ok(KeysListResponse {
        keys: keys
            .into_iter()
            .map(|key| (key.key_id(), key.public_key().to_byte_type(), key.is_active()))
            .collect(),
    })
}

pub async fn handle_set_active(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: KeysSetActiveRequest,
) -> Result<KeysSetActiveResponse, anyhow::Error> {
    let sdk = context.wallet_sdk();
    context.authorize(token, &[Permission::Keys(Crud::Update)])?;
    let km = sdk.key_manager_api();
    km.set_active_key(KeyBranch::Account, req.index)?;
    let key = km.get_active_key(KeyBranch::Account)?;

    Ok(KeysSetActiveResponse {
        public_key: key.to_public_key().to_byte_type(),
    })
}

/// Mask public keys unblind the wallet's output commitments, so the key API never serves them.
fn ensure_not_mask_branch(branch: KeyBranch) -> Result<(), anyhow::Error> {
    if branch.is_mask() {
        return Err(invalid_params(
            "branch",
            Some(format!("{branch} keys never leave the wallet")),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::test_support::TestDaemon;

    #[tokio::test]
    async fn mask_branch_public_keys_are_not_served() {
        let daemon = TestDaemon::start().await;
        let session = Some(&daemon.session);

        for branch in [KeyBranch::ConfidentialMask, KeyBranch::StealthMask] {
            let err = handle_list(&daemon.context, session, KeysListRequest { branch })
                .await
                .unwrap_err();
            assert!(err.to_string().contains("branch"), "{err}");

            for specific_index in [None, Some(0)] {
                let err = handle_create(&daemon.context, session, KeysCreateRequest { branch, specific_index })
                    .await
                    .unwrap_err();
                assert!(err.to_string().contains("branch"), "{err}");
            }
        }

        handle_create(&daemon.context, session, KeysCreateRequest {
            branch: KeyBranch::Account,
            specific_index: None,
        })
        .await
        .unwrap();
        let listed = handle_list(&daemon.context, session, KeysListRequest {
            branch: KeyBranch::Account,
        })
        .await
        .unwrap();
        assert!(!listed.keys.is_empty());
    }
}
