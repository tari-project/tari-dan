//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::io;

use indexmap::IndexMap;
use tari_bor::encode_into;
use tari_engine_types::{
    fees::FeeSource,
    substate::{SubstateAddress, SubstateValue},
};

use super::FeeTable;
use crate::runtime::{RuntimeModule, RuntimeModuleError, StateTracker};

pub struct FeeModule {
    initial_cost: u64,
    fee_table: FeeTable,
}

impl FeeModule {
    pub fn new(initial_cost: u64, fee_table: FeeTable) -> Self {
        Self {
            initial_cost,
            fee_table,
        }
    }
}

impl RuntimeModule for FeeModule {
    fn on_initialize(&self, track: &StateTracker) -> Result<(), RuntimeModuleError> {
        track.add_fee_charge(FeeSource::Initial, self.initial_cost);
        Ok(())
    }

    fn on_runtime_call(&self, track: &StateTracker, _call: &'static str) -> Result<(), RuntimeModuleError> {
        track.add_fee_charge(FeeSource::RuntimeCall, self.fee_table.per_module_call_cost());
        Ok(())
    }

    fn on_before_finalize(
        &self,
        track: &StateTracker,
        changes: &IndexMap<SubstateAddress, SubstateValue>,
    ) -> Result<(), RuntimeModuleError> {
        let total_storage = changes
            .values()
            .map(|substate| {
                let mut counter = ByteCounter::new();
                encode_into(substate, &mut counter)?;
                Ok(counter.get())
            })
            .sum::<Result<usize, RuntimeModuleError>>()?;

        track.add_fee_charge(
            FeeSource::Storage,
            // Divide by 3 to account for CBOR
            self.fee_table.per_byte_storage_cost() * total_storage as u64 / 3,
        );

        track.add_fee_charge(FeeSource::Logs, track.num_logs() as u64 * self.fee_table.per_log_cost());

        track.add_fee_charge(
            FeeSource::Events,
            track.num_events() as u64 * self.fee_table.per_event_cost(),
        );

        Ok(())
    }
}

// TODO: This may become available in tari_utilities in future
#[derive(Debug, Clone, Default)]
struct ByteCounter {
    count: usize,
}

impl ByteCounter {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn get(&self) -> usize {
        self.count
    }
}

impl io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len();
        self.count += len;
        Ok(len)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
