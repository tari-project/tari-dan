//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};
use tari_ootle_common_types::Epoch;
use tari_ootle_storage::time::PrimitiveDateTime;
use tari_ootle_transaction::TransactionId;

use crate::{
    codecs::{DefaultCodec, EpochCodec, KeyPrefix, TransactionIdCodec, UnitCodec},
    column_families::cf_names,
    prefixed,
    traits::{Cf, QueryCf},
};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, CborLen)]
pub struct FinalizedTransactionLinkData {
    // PrimitiveDateTime (from `time` crate) only implements serde, so bridge it.
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::serde_bridge")]
    pub finalized_at: PrimitiveDateTime,
    /// The epoch of the block that finalized the transaction. Keyed into [`EpochIndex`] so epoch GC
    /// can prune finalized transaction bookkeeping by age.
    #[n(1)]
    pub epoch: Epoch,
}

prefixed!(FinalizedTransactionLinkPrefix, KeyPrefix::FinalizedTransactionLinks);

pub struct FinalizedTransactionLinkCf;

impl Cf for FinalizedTransactionLinkCf {
    type Key = TransactionId;
    type KeyCodec = TransactionIdCodec;
    type Prefix = FinalizedTransactionLinkPrefix;
    type Value = FinalizedTransactionLinkData;
    type ValueCodec = DefaultCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::TRANSACTIONS
    }
}

prefixed!(
    FinalizedTransactionEpochIndexPrefix,
    KeyPrefix::FinalizedTransactionEpochIndex
);

/// Index of finalized transaction ids by the epoch they finalized in, used by epoch GC to prune
/// transaction bookkeeping by age. Exactly one entry exists per finalized id — re-finalizing an id
/// (a previously aborted transaction sequenced again) moves its entry to the latest epoch.
pub struct EpochIndex;

impl Cf for EpochIndex {
    type Key = (Epoch, TransactionId);
    type KeyCodec = (EpochCodec, TransactionIdCodec);
    type Prefix = FinalizedTransactionEpochIndexPrefix;
    type Value = ();
    type ValueCodec = UnitCodec;

    fn name() -> &'static str {
        cf_names::TRANSACTIONS
    }
}

/// Used to range-query [`EpochIndex`] up to a prune epoch.
pub struct ByEpochQuery;

impl QueryCf for ByEpochQuery {
    type Cf = EpochIndex;
    type Key = Epoch;
    type KeyCodec = EpochCodec;
}
