//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt,
    fmt::{Display, Formatter},
};

use serde::{Deserialize, Serialize};
use tari_bor::BorTag;
use tari_template_lib::{models::BinaryTag, Hash, HashParseError};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{events::Event, fees::FeeReceipt, logs::LogEntry};

const TAG: u64 = BinaryTag::TransactionReceipt.as_u64();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionReceiptAddress(#[cfg_attr(feature = "ts", ts(type = "string"))] BorTag<Hash, TAG>);

impl TransactionReceiptAddress {
    pub const fn new(address: Hash) -> Self {
        Self(BorTag::new(address))
    }

    pub fn hash(&self) -> &Hash {
        self.0.inner()
    }

    pub fn from_hex(hex: &str) -> Result<Self, HashParseError> {
        let hash = Hash::from_hex(hex)?;
        Ok(Self::new(hash))
    }
}

impl<T: Into<Hash>> From<T> for TransactionReceiptAddress {
    fn from(address: T) -> Self {
        Self::new(address.into())
    }
}

impl Display for TransactionReceiptAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "txreceipt_{}", self.hash())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct TransactionReceipt {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub transaction_hash: Hash,
    pub events: Vec<Event>,
    pub logs: Vec<LogEntry>,
    pub fee_receipt: FeeReceipt,
}
