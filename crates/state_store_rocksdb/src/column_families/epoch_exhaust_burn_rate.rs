//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_ootle_common_types::Epoch;

use crate::{
    codecs::{EpochCodec, KeyPrefix, NumberCodec},
    column_families::cf_names,
    prefixed,
    traits::Cf,
};

prefixed!(EpochExhaustBurnRatePrefix, KeyPrefix::EpochExhaustBurnRates);

/// Per epoch, the exhaust burn rate in basis points that this node opened the epoch at. One small
/// record per epoch, kept for good so that it outlives every foreign proposal that can name the epoch.
pub struct EpochExhaustBurnRateCf;

impl Cf for EpochExhaustBurnRateCf {
    type Key = Epoch;
    type KeyCodec = EpochCodec;
    type Prefix = EpochExhaustBurnRatePrefix;
    type Value = u16;
    type ValueCodec = NumberCodec<Self::Value>;

    fn name() -> &'static str {
        cf_names::CHAIN_METADATA
    }
}
