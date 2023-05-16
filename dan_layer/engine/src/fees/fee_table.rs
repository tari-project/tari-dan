//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(Debug, Clone)]
pub struct FeeTable {
    per_module_call_cost: u64,
    per_byte_storage_cost: u64,
}

impl FeeTable {
    pub fn new(per_module_call_cost: u64, per_byte_storage_cost: u64) -> Self {
        Self {
            per_module_call_cost,
            per_byte_storage_cost,
        }
    }

    pub fn zero_rated() -> Self {
        Self::new(0, 0)
    }

    pub fn per_module_call_cost(&self) -> u64 {
        self.per_module_call_cost
    }

    pub fn per_byte_storage_cost(&self) -> u64 {
        self.per_byte_storage_cost
    }
}
