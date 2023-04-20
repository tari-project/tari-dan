//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

/// Default fee loan, this barely allows a user to create an account.
pub const DEFAULT_FEE_LOAN: u64 = 400;

#[derive(Debug, Clone)]
pub struct FeeTable {
    per_module_call_cost: u64,
    per_byte_storage_cost: u64,
    loan: u64,
}

impl FeeTable {
    pub fn new(per_module_call_cost: u64, per_byte_storage_cost: u64, loan: u64) -> Self {
        Self {
            per_module_call_cost,
            per_byte_storage_cost,
            loan,
        }
    }

    pub fn zero_rated() -> Self {
        Self::new(0, 0, DEFAULT_FEE_LOAN)
    }

    pub fn per_module_call_cost(&self) -> u64 {
        self.per_module_call_cost
    }

    pub fn per_byte_storage_cost(&self) -> u64 {
        self.per_byte_storage_cost
    }

    pub fn loan(&self) -> u64 {
        self.loan
    }
}
