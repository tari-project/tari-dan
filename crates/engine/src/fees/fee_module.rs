//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::encoded_len;
use tari_engine_types::{fees::FeeSource, substate::SubstateValue};
use tari_template_lib::types::TemplateAddress;

use super::FeeTable;
use crate::{
    runtime::{ChargeableState, RuntimeEvent, RuntimeModule, RuntimeModuleError, StateTracker},
    state_store::StateReader,
};

pub struct FeeModule {
    initial_cost: u64,
    fee_table: FeeTable,
}

impl FeeModule {
    pub const fn new(initial_cost: u64, fee_table: FeeTable) -> Self {
        Self {
            initial_cost,
            fee_table,
        }
    }

    /// Metrics for persisting a published template's `binary_len`-byte binary: the bytes priced at
    /// the per-byte storage rate (the first `template_size_premium_free_bytes`, so small templates
    /// cost the same as ordinary storage), and the microtari charged on top — the flat per-publish
    /// cost plus the quadratic premium on each whole unit of the excess above the free allowance.
    /// The byte count is returned raw — not pre-divided — so the caller can accumulate across the
    /// transaction and apply `storage_cost_divisor` once at finalization, matching how the other
    /// byte/point charges round against per-transaction totals.
    fn template_publish_metrics(&self, binary_len: usize) -> Result<(u64, u64), RuntimeModuleError> {
        let size = binary_len as u64;
        let free = self.fee_table.template_size_premium_free_bytes();

        let base_bytes = size.min(free);

        let units = size.saturating_sub(free) / self.fee_table.template_size_premium_unit_bytes();
        let charge = units
            .checked_mul(units)
            .and_then(|squared| squared.checked_mul(self.fee_table.per_template_size_premium_unit_cost()))
            .and_then(|premium| premium.checked_add(self.fee_table.per_template_publish_cost()))
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating template publish premium".to_string()))?;

        Ok((base_bytes, charge))
    }

    /// Charges everything that is a function of the state being persisted: the per-byte storage
    /// tally, the template-publish premium and the per-substate creation premium.
    ///
    /// Assigned rather than accumulated, so running it again over a different state replaces the
    /// result instead of doubling it — see [`RuntimeModule::on_before_persist`].
    fn charge_state_fees<TStore: StateReader>(
        &self,
        state: &mut ChargeableState<'_, TStore>,
    ) -> Result<(), RuntimeModuleError> {
        let mut substate_bytes = 0usize;
        let mut template_base_bytes = 0u64;
        let mut template_charge = 0u64;
        for substate in state.substates_to_persist().values() {
            // A published template's binary is priced by the dedicated base + quadratic publish
            // model, so keep it out of the flat per-byte storage tally. Accumulate the raw
            // metrics here and apply the storage divisor once below.
            if let SubstateValue::Template(template) = substate {
                let (tpl_base_bytes, tpl_charge) = self.template_publish_metrics(template.binary.len())?;
                template_base_bytes = template_base_bytes.checked_add(tpl_base_bytes).ok_or_else(|| {
                    RuntimeModuleError::Overflow("Overflow accumulating template base bytes".to_string())
                })?;
                template_charge = template_charge.checked_add(tpl_charge).ok_or_else(|| {
                    RuntimeModuleError::Overflow("Overflow accumulating template publish premium".to_string())
                })?;
                continue;
            }
            substate_bytes = substate_bytes
                .checked_add(encoded_len(substate))
                .ok_or_else(|| RuntimeModuleError::Overflow("Overflow accumulating storage bytes".to_string()))?;
        }

        // Finalization persists the transaction receipt on top of the mutated substates. It carries
        // the transaction's events, so leaving it out of the tally would make that payload — the
        // largest caller-controlled contribution to permanent state after the substates themselves —
        // free.
        let receipt_bytes = state
            .transaction_receipt_size()
            .map_err(|e| RuntimeModuleError::Runtime(e.to_string()))?;
        let total_storage = substate_bytes
            .checked_add(receipt_bytes)
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow accumulating storage bytes".to_string()))?;

        let cost = self
            .fee_table
            .per_byte_storage_cost()
            .checked_mul(total_storage as u64)
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating storage cost".to_string()))?;
        let storage_cost = cost / self.fee_table.storage_cost_divisor();

        let template_base_cost = self
            .fee_table
            .per_byte_storage_cost()
            .checked_mul(template_base_bytes)
            .ok_or_else(|| {
                RuntimeModuleError::Overflow("Overflow calculating template base storage cost".to_string())
            })? /
            self.fee_table.storage_cost_divisor();
        let template_publish_cost = template_base_cost
            .checked_add(template_charge)
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating template publish cost".to_string()))?;

        // The receipt occupies a slot of its own — it is always newly created, since it is addressed
        // by a transaction id that can only be finalized once.
        let new_substate_count = state
            .count_newly_created_substates()
            .map_err(|e| RuntimeModuleError::Runtime(e.to_string()))?
            .saturating_add(1);
        let create_cost = (new_substate_count as u64)
            .checked_mul(self.fee_table.per_substate_create_cost())
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating substate create cost".to_string()))?;

        let fee_state = state.fee_state_mut();
        fee_state.set_charge(FeeSource::Storage, storage_cost);
        fee_state.set_charge(FeeSource::TemplatePublish, template_publish_cost);
        fee_state.set_charge(FeeSource::SubstateCreate, create_cost);

        Ok(())
    }

    /// Charges everything that is a function of the state being persisted, plus the metering of WASM
    /// and native execution.
    ///
    /// Every charge here is *assigned*, not accumulated, so that running this again against a
    /// different state replaces the result rather than doubling it. That is what lets a transaction
    /// be gated on the cost of the state it asked to commit and then billed for the state that is
    /// actually persisted — see [`RuntimeModule::on_before_persist`].
    fn charge_finalization_fees<TStore: StateReader>(
        &self,
        state: &mut ChargeableState<'_, TStore>,
    ) -> Result<(), RuntimeModuleError> {
        self.charge_state_fees(state)?;

        // WASM execution: charge once against the transaction's accumulated points so the divisor
        // rounds against the total. Per-call rounding would let a transaction split work into
        // sub-divisor chunks and pay zero for any single one (each `points/divisor` is `0`), even
        // though the summed work is non-trivial.
        let units = state.fee_state().accumulated_wasm_points() / self.fee_table.wasm_points_cost_divisor();
        let wasm_cost = units
            .checked_mul(self.fee_table.per_wasm_point_cost())
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating WASM execution cost".to_string()))?;

        // Native verification is priced in the same points and charged at the same rate, under its
        // own source so the breakdown distinguishes crypto verification from template execution.
        let native_units = state.fee_state().accumulated_native_points() / self.fee_table.wasm_points_cost_divisor();
        let native_cost = native_units
            .checked_mul(self.fee_table.per_wasm_point_cost())
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating native execution cost".to_string()))?;

        let fee_state = state.fee_state_mut();
        fee_state.set_charge(FeeSource::WasmExecution, wasm_cost);
        fee_state.set_charge(FeeSource::NativeExecution, native_cost);

        Ok(())
    }

    #[cfg(test)]
    fn template_publish_cost(&self, binary_len: usize) -> Result<u64, RuntimeModuleError> {
        let (base_bytes, charge) = self.template_publish_metrics(binary_len)?;
        let base = self
            .fee_table
            .per_byte_storage_cost()
            .checked_mul(base_bytes)
            .ok_or_else(|| {
                RuntimeModuleError::Overflow("Overflow calculating template base storage cost".to_string())
            })? /
            self.fee_table.storage_cost_divisor();
        base.checked_add(charge)
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating template publish cost".to_string()))
    }
}

impl<TStore: StateReader> RuntimeModule<TStore> for FeeModule {
    fn on_initialize(&self, track: &StateTracker<TStore>) -> Result<(), RuntimeModuleError> {
        track.add_fee_charge(FeeSource::Initial, self.initial_cost);
        let transaction_weight = track.get_transaction_weight();
        let transaction_weight_cost = transaction_weight
            .as_u64()
            .checked_mul(self.fee_table.per_transaction_weight_cost())
            .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating transaction weight cost".to_string()))?;
        track.add_fee_charge(FeeSource::TransactionWeight, transaction_weight_cost);

        // Charged before the fee intent runs, so a transaction pays for loading and pledging its
        // inputs even when its main intent is rejected: the pledges have been made either way.
        let input_load = track.declared_input_load();
        let rates = self.fee_table.to_rates();
        track.add_fee_charge(FeeSource::InputLoad, rates.input_load_cost(input_load.bytes));
        track.add_fee_charge(
            FeeSource::InputFanout,
            rates.input_fanout_cost(input_load.bytes, input_load.fanout),
        );

        Ok(())
    }

    fn on_runtime_call(&self, track: &StateTracker<TStore>, _call: &'static str) -> Result<(), RuntimeModuleError> {
        track.add_fee_charge(FeeSource::RuntimeCall, self.fee_table.per_module_call_cost());
        Ok(())
    }

    fn on_template_loaded(
        &self,
        track: &StateTracker<TStore>,
        template_address: &TemplateAddress,
        bytes_loaded: usize,
    ) -> Result<(), RuntimeModuleError> {
        // Dedupe per template per transaction: the validator's compile/deserialise cost is paid
        // once per template per process (in-memory + on-disk caches), so subsequent loads within
        // the same transaction (cross-template calls, repeated method invocations on the same
        // component, etc.) carry no incremental load cost. Per-call dispatch overhead is already
        // captured by `per_module_call_cost`.
        if !track.record_template_load_charge(*template_address) {
            return Ok(());
        }

        let template_load_cost_unit =
            u64::try_from(bytes_loaded).unwrap_or(u64::MAX) / self.fee_table.template_load_bytes_cost_divisor();

        let fee_charge = template_load_cost_unit
            .checked_mul(self.fee_table.per_template_load_cost_unit())
            .ok_or_else(|| {
                RuntimeModuleError::Overflow("Overflow calculating template load weight cost".to_string())
            })?;

        track.add_fee_charge(FeeSource::TemplateLoad, fee_charge);
        Ok(())
    }

    fn on_before_finalize(&self, track: &StateTracker<TStore>) -> Result<(), RuntimeModuleError> {
        track.with_chargeable_state(|state| self.charge_finalization_fees(state))
    }

    fn on_fee_checkpoint(&self, state: &mut ChargeableState<'_, TStore>) -> Result<(), RuntimeModuleError> {
        // Only the state-derived charges. Execution metering is charged once at finalization, over
        // the whole transaction's accumulated points, so pricing it here would be replaced anyway.
        self.charge_state_fees(state)
    }

    fn on_before_persist(&self, state: &mut ChargeableState<'_, TStore>) -> Result<(), RuntimeModuleError> {
        self.charge_finalization_fees(state)
    }

    fn on_runtime_event(&self, track: &StateTracker<TStore>, call: &RuntimeEvent) -> Result<(), RuntimeModuleError> {
        match call {
            RuntimeEvent::LogEmitted { size_bytes } => {
                // Charged under the same source as the host call that emitted the log: the flat
                // per-call cost prices the call, this prices what it carried. Division per call
                // forgoes less than a divisor's worth of bytes per log, bounded by
                // `ENGINE_LIMITS.max_logs`.
                let cost = self
                    .fee_table
                    .per_byte_storage_cost()
                    .checked_mul(*size_bytes as u64)
                    .ok_or_else(|| RuntimeModuleError::Overflow("Overflow calculating log cost".to_string()))? /
                    self.fee_table.log_bytes_cost_divisor();
                track.add_fee_charge(FeeSource::RuntimeCall, cost);
            },
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fee_table() -> FeeTable {
        FeeTable {
            per_byte_storage_cost: 1,
            storage_cost_divisor: 1,
            template_size_premium_free_bytes: 30 * 1024,
            template_size_premium_unit_bytes: 1024,
            per_template_size_premium_unit_cost: 100,
            per_template_publish_cost: 500,
            ..FeeTable::zero_rated()
        }
    }

    fn publish_cost(binary_len: usize) -> u64 {
        FeeModule::new(0, fee_table())
            .template_publish_cost(binary_len)
            .unwrap()
    }

    #[test]
    fn the_flat_cost_is_charged_whatever_the_size() {
        assert_eq!(publish_cost(0), 500);
    }

    #[test]
    fn at_or_below_the_free_allowance_is_flat_plus_linear() {
        assert_eq!(publish_cost(20 * 1024), 500 + 20 * 1024);
        assert_eq!(publish_cost(30 * 1024), 500 + 30 * 1024);
    }

    #[test]
    fn above_the_free_allowance_adds_a_quadratic_premium() {
        // base = first 30 KiB linear; premium = units² × 100, units = excess_KiB.
        assert_eq!(publish_cost(38 * 1024), 500 + 30 * 1024 + 8 * 8 * 100);
        assert_eq!(publish_cost(64 * 1024), 500 + 30 * 1024 + 34 * 34 * 100);
    }

    #[test]
    fn cost_is_monotonic_across_the_threshold() {
        assert!(publish_cost(30 * 1024) <= publish_cost(31 * 1024));
        assert!(publish_cost(31 * 1024) <= publish_cost(32 * 1024));
        assert!(publish_cost(32 * 1024) < publish_cost(64 * 1024));
    }
}
