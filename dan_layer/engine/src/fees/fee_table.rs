//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(Debug, Clone)]
pub struct FeeTable {
    pub per_module_call_cost: u64,
    pub per_byte_storage_cost: u64,
    pub per_event_cost: u64,
    pub per_log_cost: u64,
}

impl FeeTable {
    pub fn zero_rated() -> Self {
        Self {
            per_module_call_cost: 0,
            per_byte_storage_cost: 0,
            per_event_cost: 0,
            per_log_cost: 0,
        }
    }

    pub fn per_module_call_cost(&self) -> u64 {
        self.per_module_call_cost
    }

    pub fn per_byte_storage_cost(&self) -> u64 {
        self.per_byte_storage_cost
    }

    pub fn per_event_cost(&self) -> u64 {
        self.per_event_cost
    }

    pub fn per_log_cost(&self) -> u64 {
        self.per_log_cost
    }
}
