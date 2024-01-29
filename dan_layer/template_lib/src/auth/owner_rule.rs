//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{auth::AccessRule, crypto::RistrettoPublicKeyBytes};

/// Data that is needed to represent ownership of a value (resource or component method).
/// Owners are the only ones allowed to update the values's access rules after creation
#[derive(Debug, Clone, Copy)]
pub struct Ownership<'a> {
    pub owner_key: &'a RistrettoPublicKeyBytes,
    pub owner_rule: &'a OwnerRule,
}

/// An enum for all possible ways to specify ownership of values
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum OwnerRule {
    #[default]
    OwnedBySigner,
    None,
    ByAccessRule(AccessRule),
}
