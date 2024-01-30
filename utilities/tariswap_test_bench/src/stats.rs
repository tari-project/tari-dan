//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

#[derive(Debug, Default)]
pub struct Stats {
    num_substates_created: usize,
    total_execution_time: Duration,
    total_time_to_finalize: Duration,
    num_transactions: usize,
}
impl Stats {
    pub fn num_substates_created(&self) -> usize {
        self.num_substates_created
    }

    pub fn add_substate_created(&mut self, n: usize) {
        self.num_substates_created += n;
    }

    pub fn inc_transaction(&mut self) {
        self.num_transactions += 1;
    }

    pub fn num_transactions(&self) -> usize {
        self.num_transactions
    }

    pub fn total_execution_time(&self) -> Duration {
        self.total_execution_time
    }

    pub fn add_execution_time(&mut self, duration: Duration) {
        self.total_execution_time += duration;
    }

    pub fn total_time_to_finalize(&self) -> Duration {
        self.total_time_to_finalize
    }

    pub fn add_time_to_finalize(&mut self, duration: Duration) {
        self.total_time_to_finalize += duration;
    }
}
