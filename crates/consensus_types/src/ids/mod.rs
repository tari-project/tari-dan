//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod block_id;
mod macros;

pub use block_id::*;
use tari_sidechain::FixedHash;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    borsh::BorshSerialize,
)]
pub enum QcId {
    PcId(PcId),
    TcId(TcId),
}

impl QcId {
    pub const fn is_proposal_certificate(&self) -> bool {
        matches!(self, Self::PcId(_))
    }

    pub const fn is_timeout_certificate(&self) -> bool {
        matches!(self, Self::TcId(_))
    }

    pub const fn hash(&self) -> &FixedHash {
        match self {
            Self::PcId(pc_id) => pc_id.hash(),
            Self::TcId(tc_id) => tc_id.hash(),
        }
    }

    /// Returns the bytes of the inner id type. i.e. these bytes do not include any enum discriminant.
    pub fn as_inner_bytes(&self) -> &[u8] {
        match self {
            Self::PcId(pc_id) => pc_id.as_bytes(),
            Self::TcId(tc_id) => tc_id.as_bytes(),
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Self::PcId(pc_id) => pc_id.is_zero(),
            Self::TcId(tc_id) => tc_id.is_zero(),
        }
    }

    pub fn into_array(self) -> [u8; 32] {
        match self {
            Self::PcId(pc_id) => pc_id.into_array(),
            Self::TcId(tc_id) => tc_id.into_array(),
        }
    }
}

impl AsRef<[u8]> for QcId {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::PcId(pc_id) => pc_id.as_ref(),
            Self::TcId(tc_id) => tc_id.as_ref(),
        }
    }
}

impl std::fmt::Display for QcId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PcId(pc_id) => write!(f, "pc_{}", pc_id),
            Self::TcId(tc_id) => write!(f, "tc_{}", tc_id),
        }
    }
}

impl From<PcId> for QcId {
    fn from(pc_id: PcId) -> Self {
        Self::PcId(pc_id)
    }
}

impl From<TcId> for QcId {
    fn from(tc_id: TcId) -> Self {
        Self::TcId(tc_id)
    }
}

crate::create_hash_type!(
    ///The ID of a Proposal Certificate
    PcId
);

crate::create_hash_type!(
    ///The ID of a Timeout Certificate
    TcId
);
