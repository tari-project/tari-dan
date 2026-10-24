// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use time::PrimitiveDateTime;
use webauthn_rs::prelude::Passkey;

#[derive(Debug, Clone)]
pub struct WebauthnRegistrationModel {
    pub id: u32,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct WebauthnRegistrationPasskeyModel {
    pub id: u32,
    pub label: String,
    pub passkey: Passkey,
    pub created_at: PrimitiveDateTime,
    pub last_used_at: Option<PrimitiveDateTime>,
    pub updated_at: PrimitiveDateTime,
}
