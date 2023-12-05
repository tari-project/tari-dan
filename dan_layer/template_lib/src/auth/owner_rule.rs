//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use crate::{auth::AccessRule, crypto::RistrettoPublicKeyBytes};

/// Data that is needed to represent ownership of a resource
#[derive(Debug, Clone, Copy)]
pub struct Ownership<'a> {
    pub owner_key: &'a RistrettoPublicKeyBytes,
    pub owner_rule: &'a OwnerRule,
}

/// An enum for all possible ways to specify ownership of resources
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum OwnerRule {
    #[default]
    OwnedBySigner,
    None,
    ByAccessRule(AccessRule),
}
