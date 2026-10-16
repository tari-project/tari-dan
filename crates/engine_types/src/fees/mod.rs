//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use indexmap::{IndexMap, map::Entry};
use serde::{Deserialize, Serialize};

mod burn_rate_schedule;
mod fee_table;
pub use burn_rate_schedule::{ExhaustBurnRateSchedule, ExhaustBurnRateSource, resolve_exhaust_burn_rate};
pub use fee_table::{FeeTable, WasmMeteringRate};

/// Inline literal args carry their bytes directly in the instruction, so they are priced by size,
/// consistent with blob/log byte costing. Applied once across an instruction's whole literal
/// payload.
///
/// Public because the dry-run fee allowance is derived from it: the encoded width of the `max_fee`
/// literal is the one term that can make a real run weigh more than the dry run that estimated it.
pub const LITERAL_BYTE_DIVISOR: u64 = 3;

/// The highest exhaust burn rate a network can be configured with, in basis points.
///
/// Re-exported from `tari_template_lib_types` so that the governance template, which cannot depend
/// on this crate, holds the council to the same ceiling the engine does.
pub use tari_template_lib::types::governance::MAX_EXHAUST_BURN_RATE_BPS;

/// An exhaust burn rate in basis points, at or below [`MAX_EXHAUST_BURN_RATE_BPS`].
///
/// The share of the fees a transaction paid that is burned rather than paid to leaders. A rate
/// reaches consensus only through this type, so no network can be configured to burn more than
/// was collected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExhaustBurnRate(u16);

impl ExhaustBurnRate {
    /// Panics if `bps` is above [`MAX_EXHAUST_BURN_RATE_BPS`]. The network constants are const
    /// items, so for them that panic is a compile error.
    pub const fn new(bps: u16) -> Self {
        assert!(
            bps <= MAX_EXHAUST_BURN_RATE_BPS,
            "exhaust burn rate is above MAX_EXHAUST_BURN_RATE_BPS"
        );
        Self(bps)
    }

    /// `bps` as a rate, or `None` if it is above [`MAX_EXHAUST_BURN_RATE_BPS`].
    ///
    /// The checked counterpart of [`Self::new`], for a rate that arrives from outside this binary —
    /// a block header, a governance component, a configuration file — where being above the ceiling
    /// is something to reject rather than a programming error.
    pub const fn try_new(bps: u16) -> Option<Self> {
        if bps > MAX_EXHAUST_BURN_RATE_BPS {
            return None;
        }
        Some(Self(bps))
    }

    pub const fn as_bps(self) -> u16 {
        self.0
    }
}

/// The allowance a dry-run estimate carries on top of what it metered, so that a real run of the
/// same transaction at a different `max_fee` can never cost more than the estimate.
///
/// `max_fee` is itself an input to the cost, so the two runs meter differently. Two charges read it
/// back, and they move in opposite directions, so the allowance covers both. Transaction weight
/// prices the fee instruction's literal args by their encoded bytes, so a wider `max_fee` pushes
/// `literal_bytes / LITERAL_BYTE_DIVISOR` up by a bounded number of steps. The storage tally
/// byte-counts the fee vault before the unspent payment is returned, so it counts
/// `balance - max_fee`, and a wider `max_fee` narrows that. A dry run meters at whatever `max_fee`
/// the caller submitted — nothing narrows it — so either direction is reachable between a dry run
/// and the submission built from it.
///
/// The value is derived rather than chosen: [`FeeTable::fee_estimate_allowance`] computes it from
/// `per_transaction_weight_cost`, `per_byte_storage_cost`, `storage_cost_divisor` and
/// [`LITERAL_BYTE_DIVISOR`]. `fee_estimate_allowance_covers_every_shipped_network`, which sits with
/// the shipped tables, asserts this value covers every one of them.
pub const FEE_ESTIMATE_ALLOWANCE: u64 = 12;

/// The rates a transaction's fee charges are computed from, as a fee estimator needs them.
///
/// [`FeeTable`] is the authority on these rates and carries the terms only an executing engine
/// charges. An estimator needs the rates alone, so [`FeeTable::to_rates`] projects them onto this
/// type: a plain record where every field an estimate reads is visible where it is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeRates {
    pub per_transaction_weight_cost: u64,
    pub per_module_call_cost: u64,
    pub per_byte_storage_cost: u64,
    pub per_substate_create_cost: u64,
    pub per_wasm_point_cost: u64,
    /// Must be non-zero; a zero divisor reads as `1`, matching `FeeTable`.
    pub storage_cost_divisor: u64,
    /// Must be non-zero; a zero divisor reads as `1`, matching `FeeTable`.
    pub wasm_points_cost_divisor: u64,
}

impl FeeRates {
    pub const fn per_transaction_weight_cost(&self) -> u64 {
        self.per_transaction_weight_cost
    }

    pub const fn per_module_call_cost(&self) -> u64 {
        self.per_module_call_cost
    }

    pub const fn per_byte_storage_cost(&self) -> u64 {
        self.per_byte_storage_cost
    }

    pub const fn per_substate_create_cost(&self) -> u64 {
        self.per_substate_create_cost
    }

    /// The storage charge for `bytes` of persisted state.
    pub fn storage_cost(&self, bytes: u64) -> u64 {
        self.per_byte_storage_cost.saturating_mul(bytes) / non_zero(self.storage_cost_divisor)
    }

    /// The execution charge for `points` of metering, priced the same whether the points came from
    /// WASM or from native verification.
    pub fn execution_cost(&self, points: u64) -> u64 {
        (points / non_zero(self.wasm_points_cost_divisor)).saturating_mul(self.per_wasm_point_cost)
    }
}

const fn non_zero(divisor: u64) -> u64 {
    if divisor == 0 { 1 } else { divisor }
}

#[derive(Debug, Clone, Default)]
pub struct FeeReceiptBuilder {
    /// The total amount of the fee payment(s)
    pub total_fee_payment: u64,
    /// Total fees paid after refunds
    pub total_fees_paid: u64,
    /// The amount of non-refundable fees which the user overpaid. Fees cannot be refunded when paying purely with a
    /// stealth reveal (since we do not know the account/vault to refund).
    pub total_fee_overcharge: u64,
    /// Breakdown of fee costs
    pub cost_breakdown: FeeBreakdown,
    /// The share of `total_fees_paid` that is burned rather than paid to leaders
    pub exhaust_burn: u64,
}

impl FeeReceiptBuilder {
    pub fn with_total_fee_payment(mut self, amount: u64) -> Self {
        self.total_fee_payment = amount;
        self
    }

    pub fn with_total_fees_paid(mut self, amount: u64) -> Self {
        self.total_fees_paid = amount;
        self
    }

    pub fn with_total_fee_overcharge(mut self, amount: u64) -> Self {
        self.total_fee_overcharge = amount;
        self
    }

    pub fn with_cost_breakdown(mut self, breakdown: FeeBreakdown) -> Self {
        self.cost_breakdown = breakdown;
        self
    }

    pub fn with_exhaust_burn(mut self, amount: u64) -> Self {
        self.exhaust_burn = amount;
        self
    }

    pub fn build(self) -> FeeReceipt {
        FeeReceipt {
            total_fee_payment: self.total_fee_payment,
            total_fees_paid: self.total_fees_paid,
            total_fee_overcharge: self.total_fee_overcharge,
            cost_breakdown: self.cost_breakdown,
            exhaust_burn: self.exhaust_burn,
        }
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
    Serialize,
    Deserialize,
    borsh::BorshSerialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FeeReceipt {
    /// The total amount of the fee payment(s)
    #[n(0)]
    total_fee_payment: u64,
    /// Total fees paid after refunds
    #[n(1)]
    total_fees_paid: u64,
    /// The amount of non-refundable fees which the user overpaid. Fees cannot be refunded when paying purely with a
    /// stealth reveal (since we do not know the account/vault to refund).
    #[n(2)]
    total_fee_overcharge: u64,
    /// Breakdown of fee costs
    #[n(3)]
    cost_breakdown: FeeBreakdown,
    /// The share of `total_fees_paid` that is burned rather than paid to leaders: `⌊paid × rate / 10_000⌋` at
    /// the exhaust burn rate in force for the execution epoch. Settled over what was collected, so it is never
    /// charged to the payer and never appears in `cost_breakdown`.
    #[n(4)]
    exhaust_burn: u64,
}

impl FeeReceipt {
    pub fn builder() -> FeeReceiptBuilder {
        FeeReceiptBuilder::default()
    }

    /// The widest form this type can encode to: every amount at full varint width and a breakdown
    /// entry for every [`FeeSource`]. Bounds the encoded size of a receipt whose fees are not yet
    /// settled — see `TransactionReceipt::encoded_size_upper_bound`.
    pub fn widest() -> Self {
        let mut cost_breakdown = FeeBreakdown::default();
        for source in FeeSource::ALL {
            cost_breakdown.add(source, u64::MAX);
        }
        Self {
            total_fee_payment: u64::MAX,
            total_fees_paid: u64::MAX,
            total_fee_overcharge: u64::MAX,
            cost_breakdown,
            exhaust_burn: u64::MAX,
        }
    }

    pub fn to_cost_breakdown(&self) -> FeeCostBreakdown {
        FeeCostBreakdown {
            total_fees_charged: self.total_fees_charged(),
            required_fees: self.required_fees(),
            breakdown: self.cost_breakdown.clone(),
        }
    }

    pub fn fee_breakdown(&self) -> &FeeBreakdown {
        &self.cost_breakdown
    }

    /// The total amount of fees charged. This may be more than total_fees_paid if the user paid an insufficient amount.
    pub fn total_fees_charged(&self) -> u64 {
        self.cost_breakdown.get_total()
    }

    /// The minimum fee to submit with, given what a dry run metered.
    ///
    /// A submission cannot simply use `total_fees_charged`: the `max_fee` it carries is itself an
    /// input to the cost, so a real run meters slightly differently from the dry run that produced
    /// the estimate. The allowance covers the whole of that difference.
    ///
    /// Two charges read `max_fee` back, in opposite directions. The transaction weight prices the fee
    /// instruction's literal args by their encoded bytes, so it steps whenever the amount's width
    /// crosses a multiple of the literal divisor. The storage tally byte-counts the fee vault before
    /// the unspent payment is returned, so a wider `max_fee` leaves a narrower residual there.
    /// [`FEE_ESTIMATE_ALLOWANCE`] bounds the pair.
    ///
    /// This is a floor, not a recommendation. Overpayment is returned to the paying vault, so a
    /// caller with a vault to refund to loses nothing by submitting above it — and one paying purely
    /// by stealth reveal, where the overpayment is not refundable, has reason to sit on it.
    ///
    /// Reads what this receipt was charged. When only the fee intent committed, that is the fee
    /// intent's own cost rather than what the transaction needed, so anything telling a caller what
    /// to resubmit with wants [`crate::commit_result::FinalizeResult::required_fees`] instead.
    pub fn required_fees(&self) -> u64 {
        self.total_fees_charged().saturating_add(FEE_ESTIMATE_ALLOWANCE)
    }

    /// The total amount of fees refunded to the respective vaults
    pub fn total_refunded(&self) -> u64 {
        self.total_fee_payment
            .checked_sub(self.total_fees_charged())
            // Minus overcharge (funds that cannot be refunded)
            .and_then(|v| v.checked_sub(self.total_fee_overcharge))
            .unwrap_or_default()
    }

    /// The total amount of fees allocated to the transaction, before refunds
    pub fn total_allocated_fee_payments(&self) -> u64 {
        self.total_fee_payment
    }

    /// The total amount of fees paid after refunds
    pub fn total_fees_paid(&self) -> u64 {
        self.total_fees_paid
    }

    /// The total amount of the fee payment(s) before refunds.
    pub fn total_fee_payment(&self) -> u64 {
        self.total_fee_payment
    }

    /// The amount of unpaid fees
    pub fn unpaid_debt(&self) -> u64 {
        self.total_fees_charged().saturating_sub(self.total_fees_paid())
    }

    /// Returns true if the total fees charged is less than or equal to the total fees paid, otherwise false
    pub fn is_paid_in_full(&self) -> bool {
        self.unpaid_debt() == 0
    }

    /// The amount of non-refundable fees which the user overpaid. Fees cannot be refunded when paying purely with a
    /// stealth reveal (since we do not know the account/vault to refund).
    pub fn total_fee_overcharge(&self) -> u64 {
        self.total_fee_overcharge
    }

    /// The share of `total_fees_paid` that is burned rather than paid to leaders.
    pub fn exhaust_burn(&self) -> u64 {
        self.exhaust_burn
    }

    /// The share of `total_fees_paid` that flows to leaders: what was collected less the exhaust burn.
    pub fn pre_burn_fees_paid(&self) -> u64 {
        self.total_fees_paid().saturating_sub(self.exhaust_burn())
    }
}

impl Default for FeeReceipt {
    fn default() -> Self {
        FeeReceiptBuilder::default().build()
    }
}

#[repr(u8)]
#[derive(
    Debug,
    Clone,
    Copy,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
    Serialize,
    Deserialize,
    Hash,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    borsh::BorshSerialize,
)]
#[borsh(use_discriminant = true)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum FeeSource {
    #[n(0)]
    Initial = 0,
    /// Engine host calls: a flat per-call cost, plus the per-byte cost of a log's message. Two
    /// unlike prices share this source because a source of its own would widen
    /// [`FeeReceipt::widest`] and so the receipt size bound on every transaction.
    #[n(1)]
    RuntimeCall = 1,
    #[n(2)]
    Storage = 2,
    #[n(3)]
    TransactionWeight = 3,
    #[n(4)]
    TemplateLoad = 4,
    #[n(5)]
    SubstateCreate = 5,
    /// WASM execution metering, charged in proportion to consumed Wasmer metering points.
    #[n(6)]
    WasmExecution = 6,
    /// Cost of publishing a template's binary, replacing the flat per-byte `Storage` charge for
    /// that binary: the first `template_size_premium_free_bytes` are priced at the per-byte storage
    /// rate, and every whole unit beyond that is charged quadratically to discourage oversized
    /// templates.
    #[n(7)]
    TemplatePublish = 7,
    /// Native verification metering — stealth transfers, confidential withdraws, burn claims, and
    /// the intrinsics a template invokes — priced in the same points as `WasmExecution` via
    /// wall-clock equivalence and charged at the same per-point rate.
    ///
    /// Every kind of native work shares this source deliberately. A source per kind would widen
    /// [`FeeReceipt::widest`], and so the receipt size bound on every transaction, to itemise a
    /// breakdown the per-point rate already makes comparable.
    #[n(8)]
    NativeExecution = 8,
}

impl FeeSource {
    /// Every variant. `fee_source_all_is_exhaustive` fails to compile if a variant is added without
    /// being listed here.
    pub const ALL: [Self; 9] = [
        Self::Initial,
        Self::RuntimeCall,
        Self::Storage,
        Self::TransactionWeight,
        Self::TemplateLoad,
        Self::SubstateCreate,
        Self::WasmExecution,
        Self::TemplatePublish,
        Self::NativeExecution,
    ];
}

#[derive(
    Debug,
    Clone,
    minicbor::Encode,
    minicbor::Decode,
    minicbor::CborLen,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    Eq,
    borsh::BorshSerialize,
)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FeeBreakdown {
    #[n(0)]
    #[cbor(with = "tari_bor::adapters::indexmap_codec")]
    breakdown: IndexMap<FeeSource, u64>,
}

impl FeeBreakdown {
    pub fn add(&mut self, source: FeeSource, amount: u64) {
        match self.breakdown.entry(source) {
            Entry::Occupied(entry) => {
                let total = entry.into_mut();
                // A breakdown row is a display total, so a charge that would take it past `u64::MAX` pins it there
                // rather than wrapping. Whether the fee itself is affordable is decided elsewhere.
                *total = total.saturating_add(amount);
            },
            Entry::Vacant(entry) => {
                entry.insert(amount);
                self.breakdown.sort_keys();
            },
        }
    }

    /// Replaces whatever `source` has been charged so far.
    ///
    /// Charges accrued during execution accumulate with [`Self::add`], but the charges computed at
    /// finalization are absolute functions of the state being persisted. They are recomputed once
    /// that state is known, so they must be assignable rather than additive.
    ///
    /// A charge of zero leaves the source absent rather than recording a zero against it. The
    /// breakdown is persisted inside every transaction receipt and rendered by every wallet, so a
    /// source that never charged anything should not occupy a row. [`Self::get`] reads an absent
    /// source as zero, so the two are indistinguishable to a reader.
    pub fn set(&mut self, source: FeeSource, amount: u64) {
        if amount == 0 {
            self.breakdown.shift_remove(&source);
            return;
        }
        match self.breakdown.entry(source) {
            Entry::Occupied(entry) => {
                *entry.into_mut() = amount;
            },
            Entry::Vacant(entry) => {
                entry.insert(amount);
                self.breakdown.sort_keys();
            },
        }
    }

    /// Returns an iterator over the fee breakdown in a canonical order.
    pub fn iter(&self) -> impl Iterator<Item = (&FeeSource, &u64)> {
        self.breakdown.iter()
    }

    /// Saturating, so a breakdown that somehow exceeds `u64` reports the ceiling rather than
    /// wrapping to a total below the charges it is made of. Individual charges are checked as they
    /// are computed, so reaching it means something upstream already went wrong.
    pub fn get_total(&self) -> u64 {
        self.breakdown
            .values()
            .fold(0u64, |acc, amount| acc.saturating_add(*amount))
    }

    pub fn get(&self, source: FeeSource) -> u64 {
        self.breakdown.get(&source).copied().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FeeCostBreakdown {
    pub total_fees_charged: u64,
    pub required_fees: u64,
    pub breakdown: FeeBreakdown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burn_rate_holds_every_value_up_to_the_ceiling() {
        assert_eq!(ExhaustBurnRate::new(0).as_bps(), 0);
        assert_eq!(
            ExhaustBurnRate::new(MAX_EXHAUST_BURN_RATE_BPS).as_bps(),
            MAX_EXHAUST_BURN_RATE_BPS
        );
    }

    #[test]
    #[should_panic(expected = "exhaust burn rate is above MAX_EXHAUST_BURN_RATE_BPS")]
    fn a_burn_rate_above_the_ceiling_panics_where_const_evaluation_cannot_catch_it() {
        ExhaustBurnRate::new(MAX_EXHAUST_BURN_RATE_BPS + 1);
    }

    #[test]
    fn fee_source_all_is_exhaustive() {
        for source in FeeSource::ALL {
            // An added variant fails to compile here until it is listed in `FeeSource::ALL`.
            match source {
                FeeSource::Initial |
                FeeSource::RuntimeCall |
                FeeSource::Storage |
                FeeSource::TransactionWeight |
                FeeSource::TemplateLoad |
                FeeSource::SubstateCreate |
                FeeSource::WasmExecution |
                FeeSource::TemplatePublish |
                FeeSource::NativeExecution => {},
            }
        }
    }

    #[test]
    fn widest_bounds_every_other_receipt() {
        let widest = minicbor::len(FeeReceipt::widest());

        let mut breakdown = FeeBreakdown::default();
        breakdown.add(FeeSource::Initial, 1000);
        breakdown.add(FeeSource::Storage, u64::MAX);
        let realistic = FeeReceipt::builder()
            .with_total_fee_payment(u64::MAX)
            .with_total_fees_paid(u64::MAX)
            .with_total_fee_overcharge(u64::MAX)
            .with_cost_breakdown(breakdown)
            .with_exhaust_burn(u64::MAX)
            .build();

        assert!(minicbor::len(&realistic) <= widest);
        assert!(minicbor::len(FeeReceipt::default()) <= widest);
    }
}
