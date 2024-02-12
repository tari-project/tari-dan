//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tari_template_lib::models::{Amount, VaultId};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::resource_container::ResourceContainer;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct FeeReceipt {
    /// The total amount of the fee payment(s)
    pub total_fee_payment: Amount,
    /// Total fees paid after refunds
    pub total_fees_paid: Amount,
    /// Breakdown of fee costs
    pub cost_breakdown: Vec<FeeBreakdown>,
}

impl FeeReceipt {
    pub fn to_cost_breakdown(&self) -> FeeCostBreakdown {
        FeeCostBreakdown {
            total_fees_charged: self.total_fees_charged(),
            breakdown: self.cost_breakdown.clone(),
        }
    }

    /// The total amount of fees charged. This may be more than total_fees_paid if the user paid an insufficient amount.
    pub fn total_fees_charged(&self) -> Amount {
        Amount::try_from(
            self.cost_breakdown
                .iter()
                .map(|breakdown| breakdown.amount)
                .sum::<u64>(),
        )
        .unwrap()
    }

    pub fn total_refunded(&self) -> Amount {
        self.total_fee_payment
            .checked_sub_positive(self.total_fees_charged())
            .unwrap_or_default()
    }

    /// The total amount of fees allocated to the transaction, before refunds
    pub fn total_allocated_fee_payments(&self) -> Amount {
        self.total_fee_payment
    }

    /// The total amount of fees paid after refunds
    pub fn total_fees_paid(&self) -> Amount {
        self.total_fees_paid
    }

    /// The amount of unpaid fees
    pub fn unpaid_debt(&self) -> Amount {
        self.total_fees_charged()
            .checked_sub_positive(self.total_fees_paid())
            .unwrap_or_default()
    }

    /// Returns true if the total fees charged is equal to the total fees paid, otherwise false
    pub fn is_paid_in_full(&self) -> bool {
        self.unpaid_debt().is_zero()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub enum FeeSource {
    Initial,
    RuntimeCall,
    Storage,
    Events,
    Logs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct FeeBreakdown {
    pub source: FeeSource,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct FeeCostBreakdown {
    pub total_fees_charged: Amount,
    pub breakdown: Vec<FeeBreakdown>,
}

#[derive(Debug)]
pub struct FeePayment {
    pub resource: ResourceContainer,
    pub breakdown: HashMap<VaultId, Amount>,
}
