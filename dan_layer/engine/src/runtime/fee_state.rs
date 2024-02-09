//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::{fees::FeeBreakdown, resource_container::ResourceContainer};
use tari_template_lib::models::{Amount, VaultId};

#[derive(Debug, Clone, Default)]
pub struct FeeState {
    pub fee_payments: Vec<(ResourceContainer, VaultId)>,
    pub fee_charges: Vec<FeeBreakdown>,
}

impl FeeState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total_charges(&self) -> u64 {
        self.fee_charges.iter().map(|breakdown| breakdown.amount).sum()
    }

    pub fn total_payments(&self) -> Amount {
        self.fee_payments.iter().map(|(resx, _)| resx.amount()).sum()
    }
}
