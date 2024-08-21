//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fmt::{Display, Formatter},
};

use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHashSizeError;
use tari_crypto::tari_utilities::hex::{from_hex, Hex};
use tari_engine_types::{serde_with, transaction_receipt::TransactionReceiptAddress};
use tari_template_lib::Hash;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize, Default)]
#[serde(transparent)]
pub struct TransactionId {
    #[serde(with = "serde_with::hex")]
    id: [u8; 32],
}

impl TransactionId {
    pub const fn new(id: [u8; 32]) -> Self {
        Self { id }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.id.as_slice()
    }

    pub fn into_array(self) -> [u8; 32] {
        self.id
    }

    pub fn from_hex(hex: &str) -> Result<Self, FixedHashSizeError> {
        // TODO: This error isnt correct
        let bytes = from_hex(hex).map_err(|_| FixedHashSizeError)?;
        Self::try_from(bytes.as_slice())
    }

    pub const fn byte_size() -> usize {
        32
    }

    pub fn into_receipt_address(self) -> TransactionReceiptAddress {
        self.into_array().into()
    }

    pub fn is_empty(&self) -> bool {
        self.id.iter().all(|&b| b == 0)
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

impl From<[u8; 32]> for TransactionId {
    fn from(id: [u8; 32]) -> Self {
        Self::new(id)
    }
}

impl From<TransactionId> for Hash {
    fn from(id: TransactionId) -> Self {
        Hash::from(id.id)
    }
}
