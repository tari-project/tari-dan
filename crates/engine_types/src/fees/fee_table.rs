//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use super::{FeeRates, LITERAL_BYTE_DIVISOR};

/// The narrowest `max_fee` a run can meter at is `1`, which a dry run clamps to. `Amount` encodes as
/// a native CBOR integer, so that is a single byte.
const NARROWEST_FEE_LITERAL_BYTES: u64 = 1;
/// The widest `max_fee` a submission can carry is `u64::MAX` — `FeeState::add_fee_payment_checked`
/// rejects a payment above it — which encodes as a leading byte over eight payload bytes.
const WIDEST_FEE_LITERAL_BYTES: u64 = 9;

/// The span of encoded widths a `max_fee` can take.
const FEE_LITERAL_BYTE_SPAN: u64 = WIDEST_FEE_LITERAL_BYTES - NARROWEST_FEE_LITERAL_BYTES;

/// The most transaction-weight units the `max_fee` literal's encoded width can move a transaction.
///
/// `calc_args_weight` prices an instruction's literals as `literal_bytes / LITERAL_BYTE_DIVISOR`, so
/// widening the amount by `d` bytes lifts that quotient by at most `ceil(d / LITERAL_BYTE_DIVISOR)`,
/// attained when the narrow encoding sits just above a multiple of the divisor.
const MAX_FEE_LITERAL_WEIGHT_DRIFT: u64 = FEE_LITERAL_BYTE_SPAN.div_ceil(LITERAL_BYTE_DIVISOR);

/// The most bytes `max_fee` can move the storage tally.
///
/// The tally byte-counts the fee vault as it stands when finalization charges, which is before the
/// unspent payment is returned — so it counts `balance - max_fee`, whose width moves with `max_fee`
/// as surely as the literal's does. A payment must fit in `u64`
/// (`FeeState::add_fee_payment_checked`), so `max_fee` can only shift that residual across encoding
/// widths while the balance is itself in `u64` range; above it, every payment leaves a residual of
/// the same width. The span is therefore the literal's.
const MAX_FEE_RESIDUAL_BYTE_DRIFT: u64 = FEE_LITERAL_BYTE_SPAN;

#[derive(Debug, Clone)]
pub struct FeeTable {
    pub per_transaction_weight_cost: u64,
    pub per_module_call_cost: u64,
    pub per_byte_storage_cost: u64,
    pub per_template_load_cost_unit: u64,
    /// Flat cost charged once per newly-created substate, on top of `per_byte_storage_cost`.
    /// Reflects the slot-allocation cost of adding a new entry to permanent state, separate from
    /// the byte cost of its contents.
    pub per_substate_create_cost: u64,
    /// Cost charged per `wasm_points_cost_divisor` Wasmer metering points consumed during template
    /// execution. The opcode → point mapping lives in `tari_engine`'s `wasm/metering.rs` (most ops cost 1 point;
    /// calls 4; heavy ops up to 40). Set to 0 to disable WASM execution metering.
    pub per_wasm_point_cost: u64,
    /// Divisor applied to the raw per-byte storage cost (`per_byte_storage_cost × bytes /
    /// storage_cost_divisor`). Tunes how much of the byte cost is reflected in fees without
    /// changing the rest of the table. Must be non-zero; a zero divisor is treated as `1`.
    pub storage_cost_divisor: u64,
    /// Divisor applied to bytes-loaded when computing template-load units
    /// (`per_template_load_cost_unit × (bytes_loaded / template_load_bytes_cost_divisor)`). Lower
    /// values increase the load fee. Must be non-zero; a zero divisor is treated as `1`.
    pub template_load_bytes_cost_divisor: u64,
    /// Divisor applied to a log message's bytes when charging for it
    /// (`per_byte_storage_cost × message_bytes / log_bytes_cost_divisor`). A log is retained in
    /// each validator's record of the execution rather than in consensus state, and is prunable, so
    /// it is priced well below a persisted byte. Must be non-zero; a zero divisor is treated as
    /// `1`.
    pub log_bytes_cost_divisor: u64,
    /// Divisor applied to the encoded bytes of a transaction's declared inputs when charging for
    /// loading them (`per_byte_storage_cost × input_bytes / input_load_bytes_cost_divisor`). Every
    /// executing validator reads and decodes each input once, so this prices a one-off read rather
    /// than a retained byte. Must be non-zero; a zero divisor is treated as `1`.
    pub input_load_bytes_cost_divisor: u64,
    /// Divisor applied to the encoded bytes of a transaction's declared inputs, multiplied by the
    /// number of other shard groups they are pledged to, when charging for that pledge traffic
    /// (`per_byte_storage_cost × input_bytes × input_fanout / input_fanout_bytes_cost_divisor`).
    /// Pledged bytes draw on each block's bounded pledge budget. Must be non-zero; a zero divisor is
    /// treated as `1`.
    pub input_fanout_bytes_cost_divisor: u64,
    /// Divisor applied to consumed Wasmer points when computing WASM execution units
    /// (`per_wasm_point_cost × (points_consumed / wasm_points_cost_divisor)`). Lower values make
    /// metering more aggressive. Must be non-zero; a zero divisor is treated as `1`.
    pub wasm_points_cost_divisor: u64,
    /// Free allowance, in bytes, of a published template's binary. The first this-many bytes are
    /// priced at `per_byte_storage_cost` (matching ordinary storage); only bytes beyond it incur
    /// the quadratic publish premium.
    pub template_size_premium_free_bytes: u64,
    /// Size, in bytes, of one template-publish premium unit. Excess bytes above
    /// `template_size_premium_free_bytes` are divided by this to get the unit count that is then
    /// squared. Must be non-zero; a zero value is treated as `1`.
    pub template_size_premium_unit_bytes: u64,
    /// Cost, in microtari, charged per squared premium unit when publishing a template
    /// (`per_template_size_premium_unit_cost × units²`, `units = excess_bytes /
    /// template_size_premium_unit_bytes`). Set to 0 to disable the publish premium.
    pub per_template_size_premium_unit_cost: u64,
    /// Flat cost charged once per published template, independent of its size. A template's binary
    /// is mostly fixed `template_lib` overhead rather than author content, so size alone cannot
    /// price a small publish above the noise floor; this sets that floor. Set to 0 to charge for
    /// size only.
    pub per_template_publish_cost: u64,
}

impl FeeTable {
    pub fn zero_rated() -> Self {
        Self {
            per_transaction_weight_cost: 0,
            per_module_call_cost: 0,
            per_byte_storage_cost: 0,
            per_template_load_cost_unit: 0,
            per_substate_create_cost: 0,
            per_wasm_point_cost: 0,
            storage_cost_divisor: 1,
            template_load_bytes_cost_divisor: 1,
            log_bytes_cost_divisor: 1,
            input_load_bytes_cost_divisor: 1,
            input_fanout_bytes_cost_divisor: 1,
            wasm_points_cost_divisor: 1,
            template_size_premium_free_bytes: 0,
            template_size_premium_unit_bytes: 1,
            per_template_size_premium_unit_cost: 0,
            per_template_publish_cost: 0,
        }
    }

    /// The allowance a dry-run estimate must carry so that a real run of the same transaction at a
    /// different `max_fee` cannot cost more than the estimate.
    ///
    /// Two charges read `max_fee` back, and they move in opposite directions, so both are counted.
    /// The weight term is the fee literal's width drift at this table's weight cost. The storage
    /// term is the fee vault's residual width drift at this table's byte cost, plus the rounding
    /// boundary the storage divisor can land on.
    ///
    /// `tari_engine_types::fees::FEE_ESTIMATE_ALLOWANCE` is the value this yields, restated where
    /// `FeeReceipt::required_fees` can reach it.
    pub const fn fee_estimate_allowance(&self) -> u64 {
        let weight_drift = MAX_FEE_LITERAL_WEIGHT_DRIFT.saturating_mul(self.per_transaction_weight_cost);
        let storage_drift = MAX_FEE_RESIDUAL_BYTE_DRIFT.saturating_mul(self.per_byte_storage_cost) /
            non_zero(self.storage_cost_divisor) +
            1;
        weight_drift.saturating_add(storage_drift)
    }

    /// The rates a fee estimator needs, projected onto [`FeeRates`]. An estimator — a wallet, say —
    /// prices a transaction from those alone; the remaining entries here only mean something to an
    /// engine that executes one.
    pub fn to_rates(&self) -> FeeRates {
        FeeRates {
            per_transaction_weight_cost: self.per_transaction_weight_cost(),
            per_module_call_cost: self.per_module_call_cost(),
            per_byte_storage_cost: self.per_byte_storage_cost(),
            per_substate_create_cost: self.per_substate_create_cost(),
            per_wasm_point_cost: self.per_wasm_point_cost(),
            storage_cost_divisor: self.storage_cost_divisor(),
            wasm_points_cost_divisor: self.wasm_points_cost_divisor(),
            input_load_bytes_cost_divisor: self.input_load_bytes_cost_divisor(),
            input_fanout_bytes_cost_divisor: self.input_fanout_bytes_cost_divisor(),
        }
    }

    pub fn per_transaction_weight_cost(&self) -> u64 {
        self.per_transaction_weight_cost
    }

    pub fn per_module_call_cost(&self) -> u64 {
        self.per_module_call_cost
    }

    pub fn per_byte_storage_cost(&self) -> u64 {
        self.per_byte_storage_cost
    }

    pub fn per_template_load_cost_unit(&self) -> u64 {
        self.per_template_load_cost_unit
    }

    pub fn per_substate_create_cost(&self) -> u64 {
        self.per_substate_create_cost
    }

    pub fn per_wasm_point_cost(&self) -> u64 {
        self.per_wasm_point_cost
    }

    pub fn storage_cost_divisor(&self) -> u64 {
        non_zero(self.storage_cost_divisor)
    }

    pub fn template_load_bytes_cost_divisor(&self) -> u64 {
        non_zero(self.template_load_bytes_cost_divisor)
    }

    pub fn wasm_points_cost_divisor(&self) -> u64 {
        non_zero(self.wasm_points_cost_divisor)
    }

    pub fn log_bytes_cost_divisor(&self) -> u64 {
        non_zero(self.log_bytes_cost_divisor)
    }

    pub fn input_load_bytes_cost_divisor(&self) -> u64 {
        non_zero(self.input_load_bytes_cost_divisor)
    }

    pub fn input_fanout_bytes_cost_divisor(&self) -> u64 {
        non_zero(self.input_fanout_bytes_cost_divisor)
    }

    pub fn template_size_premium_free_bytes(&self) -> u64 {
        self.template_size_premium_free_bytes
    }

    pub fn template_size_premium_unit_bytes(&self) -> u64 {
        non_zero(self.template_size_premium_unit_bytes)
    }

    pub fn per_template_size_premium_unit_cost(&self) -> u64 {
        self.per_template_size_premium_unit_cost
    }

    pub fn per_template_publish_cost(&self) -> u64 {
        self.per_template_publish_cost
    }
}

const fn non_zero(divisor: u64) -> u64 {
    if divisor == 0 { 1 } else { divisor }
}

/// The WASM-execution fee rate extracted from a [`FeeTable`], plus the conversion from fees paid
/// into the compute budget they unlock. Lives outside [`FeeTable`] so the execution core
/// (`StateTracker`) can enforce the per-transaction compute budget without depending on the
/// `FeeModule`, which is an optional, observer-style runtime module.
#[derive(Debug, Clone, Copy)]
pub struct WasmMeteringRate {
    per_point_cost: u64,
    points_divisor: u64,
}

impl WasmMeteringRate {
    pub fn from_fee_table(fee_table: &FeeTable) -> Self {
        Self {
            per_point_cost: fee_table.per_wasm_point_cost(),
            points_divisor: fee_table.wasm_points_cost_divisor(),
        }
    }

    /// A rate that does not price WASM execution, so no payment-funded compute bound applies (only
    /// the per-transaction hard cap). Used when fees are disabled.
    pub fn unmetered() -> Self {
        Self {
            per_point_cost: 0,
            points_divisor: 1,
        }
    }

    /// Whether this rate prices WASM execution at all. A rate that does not carries no compute
    /// bound beyond the per-transaction hard cap, since there is nothing for a payment to fund.
    pub fn prices_execution(&self) -> bool {
        self.per_point_cost > 0
    }

    /// The WASM metering points that `fees_paid` microtari pre-fund: the inverse of the fee module's
    /// charge (`points / divisor * per_point_cost`). `None` when WASM execution is not priced
    /// (`per_point_cost == 0`) — payment cannot fund what is not charged, so no payment-derived
    /// bound applies.
    pub fn points_funded_by(&self, fees_paid: u64) -> Option<u64> {
        if self.per_point_cost == 0 {
            return None;
        }
        let funded =
            u128::from(fees_paid).saturating_mul(u128::from(self.points_divisor)) / u128::from(self.per_point_cost);
        Some(u64::try_from(funded).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use tari_template_lib::types::Amount;

    use super::{super::FEE_ESTIMATE_ALLOWANCE, *};

    /// The drift is computed from constant widths, so this holds the encoder to them.
    #[test]
    fn the_fee_literal_widths_match_the_encoder() {
        assert_eq!(minicbor::len(Amount::from(1u64)) as u64, NARROWEST_FEE_LITERAL_BYTES);
        assert_eq!(minicbor::len(Amount::from(u64::MAX)) as u64, WIDEST_FEE_LITERAL_BYTES);
        // The width the instruction actually carries, which is what `calc_args_weight` prices.
        assert_eq!(
            tari_bor::encoded_len_via_writer(&Amount::from(u64::MAX)).unwrap() as u64,
            WIDEST_FEE_LITERAL_BYTES
        );
        assert_eq!(MAX_FEE_LITERAL_WEIGHT_DRIFT, 3);
    }

    /// A table priced like the shipped ones: one microtari per weight unit and per stored byte.
    fn shipped_like() -> FeeTable {
        let mut table = FeeTable::zero_rated();
        table.per_transaction_weight_cost = 1;
        table.per_byte_storage_cost = 1;
        table.storage_cost_divisor = 1;
        table
    }

    #[test]
    fn the_allowance_covers_both_directions_max_fee_moves() {
        let mut table = shipped_like();

        // Weight drift (3 units) + storage drift (8 bytes) + the storage rounding boundary.
        assert_eq!(table.fee_estimate_allowance(), 3 + 8 + 1);

        // The storage divisor scales the residual term down.
        table.storage_cost_divisor = 4;
        assert_eq!(table.fee_estimate_allowance(), 3 + 8 / 4 + 1);

        // Each rate multiplies its own term.
        table.storage_cost_divisor = 1;
        table.per_transaction_weight_cost = 2;
        assert_eq!(table.fee_estimate_allowance(), 6 + 8 + 1);
        table.per_byte_storage_cost = 2;
        assert_eq!(table.fee_estimate_allowance(), 6 + 16 + 1);
    }

    /// `FEE_ESTIMATE_ALLOWANCE` is stated in `tari_engine_types`, which cannot see a `FeeTable`. It
    /// holds for a table priced like the shipped ones.
    #[test]
    fn the_restated_allowance_matches_the_derivation() {
        assert_eq!(FEE_ESTIMATE_ALLOWANCE, shipped_like().fee_estimate_allowance());
    }
}
