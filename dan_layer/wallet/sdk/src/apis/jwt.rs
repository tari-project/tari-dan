//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jsonwebtoken::{decode, encode, errors, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::prelude::{ComponentAddress, ResourceAddress};

use crate::storage::{WalletStorageError, WalletStore, WalletStoreWriter};

pub struct JwtApi<'a, TStore> {
    store: &'a TStore,
    duration: Duration,
    auth_secret_key: String,
    jwt_secret_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
pub enum JrpcPermission {
    AccountInfo,
    NftGetOwnershipProof(Option<ResourceAddress>),
    AccountBalance(SubstateAddress),
    AccountList(Option<ComponentAddress>),
    TransactionSend(SubstateAddress),
    // This can't be set via cli, after we agree on the permissions I can add the from_str.
    GetNft(Option<SubstateAddress>, Option<ResourceAddress>),
    // User should never grant this permission, it will be generated only by the UI to start the webrtc session.
    StartWebrtc,
    Admin,
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid permissions '{0}'")]
pub struct InvalidJrpcPermissionsFormat(String);

impl FromStr for JrpcPermission {
    type Err = InvalidJrpcPermissionsFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // First the empty and optional
        match s.split_once('_') {
            Some(("NftGetOwnershipProof", addr)) => Ok(JrpcPermission::NftGetOwnershipProof(Some(
                ResourceAddress::from_str(addr).map_err(|e| InvalidJrpcPermissionsFormat(e.to_string()))?,
            ))),
            Some(("AccountBalance", addr)) => Ok(JrpcPermission::AccountBalance(
                SubstateAddress::from_str(addr).map_err(|e| InvalidJrpcPermissionsFormat(e.to_string()))?,
            )),
            Some(("AccountList", addr)) => Ok(JrpcPermission::AccountList(Some(
                ComponentAddress::from_str(addr).map_err(|e| InvalidJrpcPermissionsFormat(e.to_string()))?,
            ))),
            Some(("TransactionSend", addr)) => Ok(JrpcPermission::TransactionSend(
                SubstateAddress::from_str(addr).map_err(|e| InvalidJrpcPermissionsFormat(e.to_string()))?,
            )),
            Some(_) => Err(InvalidJrpcPermissionsFormat(s.to_string())),
            None => match s {
                "AccountInfo" => Ok(JrpcPermission::AccountInfo),
                "NftGetOwnershipProof" => Ok(JrpcPermission::NftGetOwnershipProof(None)),
                "AccountList" => Ok(JrpcPermission::AccountList(None)),
                "GetNft" => Ok(JrpcPermission::GetNft(None, None)),
                "StartWebrtc" => Ok(JrpcPermission::StartWebrtc),
                "Admin" => Ok(JrpcPermission::Admin),
                _ => Err(InvalidJrpcPermissionsFormat(s.to_string())),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JrpcPermissions(pub Vec<JrpcPermission>);

impl FromStr for JrpcPermissions {
    type Err = InvalidJrpcPermissionsFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(JrpcPermissions(
            s.split(',').map(JrpcPermission::from_str).collect::<Result<_, _>>()?,
        ))
    }
}

impl JrpcPermissions {
    pub fn no_permissions(&self) -> bool {
        self.0.is_empty()
    }

    pub fn check_permission(&self, permission: &JrpcPermission) -> Result<(), JwtApiError> {
        if self.0.contains(permission) || self.0.contains(&JrpcPermission::Admin) {
            Ok(())
        } else {
            Err(JwtApiError::InsufficientPermissions {
                required: permission.clone(),
            })
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    id: u64,
    permissions: JrpcPermissions,
    exp: usize,
}

// This is used when you request permission.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthClaims {
    id: u64,
    permissions: JrpcPermissions,
    exp: usize,
}

impl<'a, TStore: WalletStore> JwtApi<'a, TStore> {
    pub(crate) fn new(store: &'a TStore, duration: Duration, secret_key: String) -> Self {
        Self {
            store,
            duration,
            auth_secret_key: format!("auth-{secret_key}"),
            jwt_secret_key: format!("jwt-{secret_key}"),
        }
    }

    // Get and also increment index. We could probably use random id here.
    pub fn get_index(&self) -> Result<u64, JwtApiError> {
        let mut tx = self.store.create_write_tx()?;
        let index = tx.jwt_add_empty_token()?;
        tx.commit()?;
        Ok(index)
    }

    pub fn generate_auth_token(&self, permissions: JrpcPermissions) -> Result<String, JwtApiError> {
        let id = self.get_index()?;

        let my_claims = AuthClaims {
            id,
            permissions,
            exp: (SystemTime::now() + self.duration)
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as usize,
        };
        let auth_token = encode(
            &Header::default(),
            &my_claims,
            &EncodingKey::from_secret(self.auth_secret_key.as_ref()),
        )?;
        Ok(auth_token)
    }

    fn check_auth_token(&self, auth_token: &str) -> Result<AuthClaims, JwtApiError> {
        let auth_token_data = decode::<AuthClaims>(
            auth_token,
            &DecodingKey::from_secret(self.auth_secret_key.as_ref()),
            &Validation::default(),
        )?;
        Ok(auth_token_data.claims)
    }

    fn get_permissions(&self, token: &str) -> Result<JrpcPermissions, JwtApiError> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret_key.as_ref()),
            &Validation::default(),
        )?;
        Ok(token_data.claims.permissions)
    }

    pub fn grant(&mut self, auth_token: String) -> Result<String, JwtApiError> {
        let auth_claims = self.check_auth_token(auth_token.as_ref())?;
        let my_claims = Claims {
            id: auth_claims.id,
            permissions: auth_claims.permissions,
            exp: (SystemTime::now() + self.duration)
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as usize,
        };
        let permissions_token = encode(
            &Header::default(),
            &my_claims,
            &EncodingKey::from_secret(self.jwt_secret_key.as_ref()),
        )?;
        let mut tx = self.store.create_write_tx()?;
        println!("Storing ID {}", auth_claims.id);
        tx.jwt_store_decision(auth_claims.id, Some(permissions_token.clone()))?;
        tx.commit()?;
        Ok(permissions_token)
    }

    pub fn deny(&mut self, auth_token: String) -> Result<(), JwtApiError> {
        let auth_claims = self.check_auth_token(auth_token.as_ref())?;
        let mut tx = self.store.create_write_tx()?;
        tx.jwt_store_decision(auth_claims.id, None)?;
        tx.commit()?;
        Ok(())
    }

    fn is_token_revoked(&self, token: &str) -> Result<bool, JwtApiError> {
        let mut tx = self.store.create_write_tx()?;
        let revoked = tx.jwt_is_revoked(token)?;
        tx.commit()?;
        Ok(revoked)
    }

    pub fn check_auth(&self, token: Option<String>, req_permissions: &[JrpcPermission]) -> Result<(), JwtApiError> {
        let token = token.ok_or(JwtApiError::TokenMissing)?;
        if self.is_token_revoked(&token)? {
            return Err(JwtApiError::TokenRevoked {});
        }
        let permissions = self.get_permissions(&token)?;
        for permission in req_permissions {
            permissions.check_permission(permission)?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JwtApiError {
    #[error("Store error: {0}")]
    StoreError(#[from] WalletStorageError),
    #[error("JWT error : {0}")]
    JwtError(#[from] errors::Error),
    #[error("Token missing")]
    TokenMissing,
    #[error("Insufficient permissions. Required '{required:?}'")]
    InsufficientPermissions { required: JrpcPermission },
    #[error("Token revoked")]
    TokenRevoked,
}
