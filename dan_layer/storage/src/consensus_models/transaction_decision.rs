//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHashSizeError;
use tari_dan_common_types::serde_with;
use tari_utilities::hex::Hex;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Decision {
    /// Decision to COMMIT the transaction
    Commit,
    /// Decision to ABORT the transaction
    Abort,
}

impl Decision {
    pub fn is_commit(&self) -> bool {
        matches!(self, Decision::Commit)
    }

    pub fn is_abort(&self) -> bool {
        matches!(self, Decision::Abort)
    }
}

impl Display for Decision {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Decision::Commit => write!(f, "Commit"),
            Decision::Abort => write!(f, "Abort"),
        }
    }
}

impl FromStr for Decision {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Commit" => Ok(Decision::Commit),
            "Abort" => Ok(Decision::Abort),
            _ => Err(()),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct TransactionId {
    #[serde(with = "serde_with::hex")]
    id: [u8; 32],
}

impl TransactionId {
    pub fn new(id: [u8; 32]) -> Self {
        Self { id }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.id.as_slice()
    }

    pub fn into_array(self) -> [u8; 32] {
        self.id
    }

    pub fn from_array(data: [u8; 32]) -> Self {
        Self { id: data }
    }
}

impl AsRef<[u8]> for TransactionId {
    fn as_ref(&self) -> &[u8] {
        self.id.as_slice()
    }
}

impl Display for TransactionId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id.to_hex())
    }
}

impl TryFrom<Vec<u8>> for TransactionId {
    type Error = FixedHashSizeError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::try_from(value.as_slice())
    }
}

impl TryFrom<&[u8]> for TransactionId {
    type Error = FixedHashSizeError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() != 32 {
            return Err(FixedHashSizeError);
        }
        let mut id = [0u8; 32];
        id.copy_from_slice(value);
        Ok(TransactionId::new(id))
    }
}
