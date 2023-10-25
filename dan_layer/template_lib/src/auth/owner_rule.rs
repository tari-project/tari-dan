//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{auth::AccessRule, crypto::RistrettoPublicKeyBytes};

#[derive(Debug, Clone, Copy)]
pub struct Ownership<'a> {
    pub owner_key: &'a RistrettoPublicKeyBytes,
    pub owner_rule: &'a OwnerRule,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum OwnerRule {
    #[default]
    OwnedBySigner,
    None,
    ByAccessRule(AccessRule),
}
