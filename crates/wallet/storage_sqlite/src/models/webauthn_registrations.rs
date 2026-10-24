// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_wallet_sdk::models::{WebauthnRegistrationModel, WebauthnRegistrationPasskeyModel};
use time::PrimitiveDateTime;
use webauthn_rs::prelude::Passkey;

use crate::schema::{webauthn_registration_passkeys, webauthn_registrations};

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = webauthn_registrations)]
pub struct WebauthnRegistration {
    pub id: i32,
    pub username: String,
    pub created_at: PrimitiveDateTime,
    pub updated_at: PrimitiveDateTime,
}

impl From<WebauthnRegistration> for WebauthnRegistrationModel {
    fn from(reg: WebauthnRegistration) -> Self {
        Self {
            id: reg.id as u32,
            username: reg.username,
        }
    }
}

#[derive(Debug, Clone, Queryable, Identifiable, Selectable)]
#[diesel(table_name = webauthn_registration_passkeys)]
pub struct WebauthnRegistrationPasskey {
    pub id: i32,
    pub registration_id: i32,
    pub label: String,
    pub passkey: Vec<u8>,
    pub created_at: PrimitiveDateTime,
    pub last_used_at: Option<PrimitiveDateTime>,
    pub updated_at: PrimitiveDateTime,
}

impl TryFrom<&WebauthnRegistrationPasskey> for WebauthnRegistrationPasskeyModel {
    type Error = serde_json::Error;

    fn try_from(value: &WebauthnRegistrationPasskey) -> Result<Self, Self::Error> {
        let passkey: Passkey = serde_json::from_slice(&value.passkey)?;
        Ok(WebauthnRegistrationPasskeyModel {
            id: value.id as u32,
            label: value.label.clone(),
            passkey,
            created_at: value.created_at,
            last_used_at: value.last_used_at,
            updated_at: value.updated_at,
        })
    }
}
