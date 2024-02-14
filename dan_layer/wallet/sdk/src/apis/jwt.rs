//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Display, Formatter},
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jsonwebtoken::{decode, encode, errors, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_template_lib::prelude::{ComponentAddress, ResourceAddress};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::storage::{WalletStorageError, WalletStore, WalletStoreReader, WalletStoreWriter};

pub struct JwtApi<'a, TStore> {
    store: &'a TStore,
    default_expiry: Duration,
    auth_secret_key: String,
    jwt_secret_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum JrpcPermission {
    AccountInfo,
    NftGetOwnershipProof(Option<ResourceAddress>),
    AccountBalance(SubstateId),
    AccountList(Option<ComponentAddress>),
    SubstatesRead,
    TemplatesRead,
    KeyList,
    TransactionGet,
    TransactionSend(Option<SubstateId>),
    // This can't be set via cli, after we agree on the permissions I can add the from_str.
    GetNft(Option<SubstateId>, Option<ResourceAddress>),
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
                SubstateId::from_str(addr).map_err(|e| InvalidJrpcPermissionsFormat(e.to_string()))?,
            )),
            Some(("AccountList", addr)) => Ok(JrpcPermission::AccountList(Some(
                ComponentAddress::from_str(addr).map_err(|e| InvalidJrpcPermissionsFormat(e.to_string()))?,
            ))),
            Some(("TransactionSend", addr)) => Ok(JrpcPermission::TransactionSend(Some(
                SubstateId::from_str(addr).map_err(|e| InvalidJrpcPermissionsFormat(e.to_string()))?,
            ))),
            Some(_) => Err(InvalidJrpcPermissionsFormat(s.to_string())),
            None => match s {
                "AccountInfo" => Ok(JrpcPermission::AccountInfo),
                "NftGetOwnershipProof" => Ok(JrpcPermission::NftGetOwnershipProof(None)),
                "AccountList" => Ok(JrpcPermission::AccountList(None)),
                "SubstatesRead" => Ok(JrpcPermission::SubstatesRead),
                "TemplatesRead" => Ok(JrpcPermission::TemplatesRead),
                "KeyList" => Ok(JrpcPermission::KeyList),
                "GetNft" => Ok(JrpcPermission::GetNft(None, None)),
                "TransactionGet" => Ok(JrpcPermission::TransactionGet),
                "TransactionSend" => Ok(JrpcPermission::TransactionSend(None)),
                "StartWebrtc" => Ok(JrpcPermission::StartWebrtc),
                "Admin" => Ok(JrpcPermission::Admin),
                _ => Err(InvalidJrpcPermissionsFormat(s.to_string())),
            },
        }
    }
}

impl Display for JrpcPermission {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            JrpcPermission::AccountInfo => f.write_str("AccountInfo"),
            JrpcPermission::NftGetOwnershipProof(Some(a)) => f.write_str(&format!("NftGetOwnershipProof_{}", a)),
            JrpcPermission::NftGetOwnershipProof(None) => f.write_str("NftGetOwnershipProof"),
            JrpcPermission::AccountBalance(a) => f.write_str(&format!("AccountBalance_{}", a)),
            JrpcPermission::AccountList(None) => f.write_str("AccountList"),
            JrpcPermission::AccountList(Some(a)) => f.write_str(&format!("AccountList_{}", a)),
            JrpcPermission::KeyList => f.write_str("KeyList"),
            JrpcPermission::TransactionGet => f.write_str("TransactionGet"),
            JrpcPermission::TransactionSend(None) => f.write_str("TransactionSend"),
            JrpcPermission::TransactionSend(Some(s)) => f.write_str(&format!("TransactionSend_{}", s)),
            JrpcPermission::GetNft(_, _) => f.write_str("GetNft"),
            JrpcPermission::StartWebrtc => f.write_str("StartWebrtc"),
            JrpcPermission::Admin => f.write_str("Admin"),
            JrpcPermission::SubstatesRead => f.write_str("SubstatesRead"),
            JrpcPermission::TemplatesRead => f.write_str("TemplatesRead"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
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

impl TryFrom<&[String]> for JrpcPermissions {
    type Error = InvalidJrpcPermissionsFormat;

    fn try_from(value: &[String]) -> Result<Self, Self::Error> {
        let mut permissions = Vec::new();
        for permission in value {
            permissions.push(JrpcPermission::from_str(permission)?);
        }
        Ok(JrpcPermissions(permissions))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Claims {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub id: u64,
    pub name: String,
    pub permissions: JrpcPermissions,
    pub exp: usize,
}

// This is used when you request permission.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthClaims {
    id: u64,
    permissions: JrpcPermissions,
    exp: usize,
}

impl<'a, TStore: WalletStore> JwtApi<'a, TStore> {
    pub(crate) fn new(store: &'a TStore, default_expiry: Duration, secret_key: String) -> Self {
        Self {
            store,
            default_expiry,
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

    pub fn generate_auth_token(
        &self,
        permissions: JrpcPermissions,
        duration: Option<Duration>,
    ) -> Result<(String, SystemTime), JwtApiError> {
        let id = self.get_index()?;
        let valid_till = SystemTime::now() + duration.unwrap_or(self.default_expiry);
        let my_claims = AuthClaims {
            id,
            permissions,
            exp: valid_till.duration_since(UNIX_EPOCH).unwrap().as_secs() as usize,
        };
        let auth_token = encode(
            &Header::default(),
            &my_claims,
            &EncodingKey::from_secret(self.auth_secret_key.as_ref()),
        )?;
        Ok((auth_token, valid_till))
    }

    fn check_auth_token(&self, auth_token: &str) -> Result<AuthClaims, JwtApiError> {
        let auth_token_data = decode::<AuthClaims>(
            auth_token,
            &DecodingKey::from_secret(self.auth_secret_key.as_ref()),
            &Validation::default(),
        )?;
        Ok(auth_token_data.claims)
    }

    fn get_token_claims(&self, token: &str) -> Result<Claims, JwtApiError> {
        let claims = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret_key.as_ref()),
            &Validation::default(),
        )
        .map(|token_data| token_data.claims)?;
        Ok(claims)
    }

    fn get_permissions(&self, token: &str) -> Result<JrpcPermissions, JwtApiError> {
        self.get_token_claims(token).map(|claims| claims.permissions)
    }

    pub fn grant(&self, name: String, auth_token: String) -> Result<String, JwtApiError> {
        let auth_claims = self.check_auth_token(auth_token.as_ref())?;
        let my_claims = Claims {
            id: auth_claims.id,
            name,
            permissions: auth_claims.permissions,
            exp: auth_claims.exp,
        };
        let permissions_token = encode(
            &Header::default(),
            &my_claims,
            &EncodingKey::from_secret(self.jwt_secret_key.as_ref()),
        )?;
        let mut tx = self.store.create_write_tx()?;

        tx.jwt_store_decision(auth_claims.id, Some(permissions_token.clone()))?;
        tx.commit()?;
        Ok(permissions_token)
    }

    pub fn deny(&self, auth_token: String) -> Result<(), JwtApiError> {
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

    pub fn revoke(&self, token_id: i32) -> Result<(), JwtApiError> {
        let mut tx = self.store.create_write_tx()?;
        tx.jwt_revoke(token_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_tokens(&self) -> Result<Vec<Claims>, JwtApiError> {
        let mut tx = self.store.create_read_tx()?;
        let tokens = tx.jwt_get_all()?;
        let mut res = Vec::new();
        for (_, token) in tokens.iter().filter(|(_, token)| token.is_some()) {
            if let Ok(claims) = self.get_token_claims(token.as_ref().unwrap().as_str()) {
                res.push(claims);
            }
        }
        Ok(res)
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
