//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    cmp::Ordering,
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHashSizeError;
use tari_dan_common_types::serde_with;
use tari_utilities::hex::Hex;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct TransactionDecision {
    pub transaction_id: TransactionId,
    pub decision: Decision,
    /// The fee for this transaction owed to this validator shard. `calculated_fee / num_shards`.
    pub per_shard_validator_fee: u64,
}

impl Ord for TransactionDecision {
    fn cmp(&self, other: &Self) -> Ordering {
        self.transaction_id.cmp(&other.transaction_id)
    }
}

impl PartialOrd for TransactionDecision {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.transaction_id.partial_cmp(&other.transaction_id)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum Decision {
    Accept,
    Reject,
}

impl Display for Decision {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Decision::Accept => write!(f, "Accept"),
            Decision::Reject => write!(f, "Reject"),
        }
    }
}

impl FromStr for Decision {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Accept" => Ok(Decision::Accept),
            "Reject" => Ok(Decision::Reject),
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
