//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

pub use tari_ootle_light_client::{TrustedValidatorSet, TrustedValidatorSetHash};
use time::PrimitiveDateTime;

/// How the wallet came to trust a validator set without a handover from the set before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ValidatorSetAnchor {
    /// The first validator set the wallet trusted, named by the latest checkpoint its indexer served.
    FirstUse,
    /// A validator set trusted in place of walking to it, named by the latest checkpoint its indexer served. The wallet
    /// re-anchors when its trusted set is too far behind to walk, or when asked to.
    Reanchor,
}

impl ValidatorSetAnchor {
    pub fn as_key_str(&self) -> &'static str {
        match self {
            Self::FirstUse => "FirstUse",
            Self::Reanchor => "Reanchor",
        }
    }
}

impl FromStr for ValidatorSetAnchor {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "FirstUse" => Ok(Self::FirstUse),
            "Reanchor" => Ok(Self::Reanchor),
            _ => Err(()),
        }
    }
}

/// A validator set the wallet anchored on.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnchoredValidatorSet {
    pub validator_set: TrustedValidatorSetHash,
    pub anchor: ValidatorSetAnchor,
    pub anchored_at: PrimitiveDateTime,
}
