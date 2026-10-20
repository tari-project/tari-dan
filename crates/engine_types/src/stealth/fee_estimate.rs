//    Copyright 2026 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

//! Static pricing for a stealth transfer that sources its own fee.
//!
//! A wallet cannot choose a transfer's shape without knowing what the shape costs: input selection
//! targets `amount + max_fee`, so the fee decides which UTXOs are spent and whether any change is
//! left over, and a change output is another stealth output with its own verification and storage.
//! Learning the cost by dry-running resolves that fixed point over the network, a round trip per
//! iteration. [`MergedStealthTransferShape::estimate_fee`] resolves it locally instead: a builder
//! rebuilds and re-prices until a build pays for itself, paying in local proof generation rather
//! than in round trips.
//!
//! What it prices is an upper bound, never an under-estimate: a builder that settles under the real
//! charge submits a transaction that is rejected for underpayment and, a fee intent leaving no
//! checkpoint to fall back to, collects nothing. The bound is close enough to be reported as-is —
//! everything but the receipt's epoch is measured rather than stood in for, which leaves a handful
//! of microtari, and a builder that reveals the figure it settled on cannot get that back.
//! `stealth_fee_estimate.rs` in `tari_engine`'s test suite holds both properties against real
//! executions across a matrix of shapes.
//!
//! [`ClaimBurnShape::estimate_fee`] prices an L1 burn claim the same way, so a claimant has a figure
//! of its own to bound the fee it reveals by. `claim_burn.rs` in the same suite holds it.

use tari_template_lib::types::{
    ClaimedOutputTombstoneAddress,
    ObjectKey,
    ResourceAddress,
    UtxoAddress,
    UtxoId,
    crypto::RistrettoPublicKeyBytes,
    stealth::StealthUnspentOutput,
};

use crate::{
    Epoch,
    UtxoOutput,
    confidential::ClaimedOutputTombstone,
    crypto::{ElgamalVerifiableBalanceBytes, OutputBody},
    fees::FeeRates,
    limits::NativeExecutionPoints,
    stealth::transfer_native_points_for_shape,
    substate::{SubstateId, SubstateValue},
    transaction_receipt::TransactionReceipt,
    utxo::Utxo,
};

/// The shape of a stealth transfer that both moves funds and reveals the fee that pays for it,
/// creating only stealth outputs.
///
/// This is the transaction a wallet builds for the common send: one stealth transfer statement in
/// the fee intent, its revealed remainder paid straight to `pay_fee`. Nothing in it reads the fee
/// amount back — the amount lives inside the statement, which weighs by input count rather than by
/// the amounts it carries — so the cost is a function of this shape alone and
/// [`Self::estimate_fee`] can settle it without executing anything.
///
/// A transfer that pays a recipient in revealed funds, sources its fee from a separate statement,
/// or calls a template is a different shape with charges this does not model, and must not be
/// priced with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergedStealthTransferShape {
    /// Stealth UTXOs the statement spends. A spent UTXO leaves state altogether rather than being
    /// rewritten, so it costs verification but no storage.
    pub num_inputs: usize,
    /// Stealth UTXOs the statement creates, the change output included.
    pub num_outputs: usize,
    /// Bytes the created UTXOs occupy in persisted state, summed across the outputs. Measured with
    /// [`persisted_utxo_bytes`] from the outputs the builder generated, so the estimate prices the
    /// payload they actually carry — a memo, an authorization, a viewable balance — rather than a
    /// stand-in width for each.
    pub persisted_output_bytes: usize,
    /// Whether the resource carries a view key, which adds a viewable-balance proof to each output
    /// — verification to pay for, and bytes to store.
    pub has_view_key: bool,
    /// The weight of the transaction this shape builds into. Weight is defined by
    /// `tari_ootle_transaction`, downstream of this crate, so the builder supplies it.
    pub transaction_weight: u64,
}

impl MergedStealthTransferShape {
    /// Engine host calls this shape's instruction sequence makes: the stealth transfer, the
    /// workspace put that carries its revealed bucket, and the fee payment. Fixed rather than
    /// supplied, because a sequence that makes any other call is not this shape — a badge proof or a
    /// withdraw would bring a template load and WASM execution with it, neither of which is priced
    /// here. `the_runtime_call_count_matches_the_instruction_sequence` holds it to what the engine
    /// counts.
    pub const RUNTIME_CALLS: u64 = 3;

    /// An upper bound, in microtari, on what the engine charges a transaction of this shape.
    ///
    /// Sums the charges the engine takes over a shape — transaction weight, host calls, native
    /// verification, persisted storage and substate creation. The exhaust burn is a share of what
    /// is paid, not a charge, so it does not enter the price.
    pub fn estimate_fee(&self, rates: &FeeRates) -> u64 {
        let weight_cost = self
            .transaction_weight
            .saturating_mul(rates.per_transaction_weight_cost());
        let runtime_call_cost = Self::RUNTIME_CALLS.saturating_mul(rates.per_module_call_cost());
        let native_cost = rates.execution_cost(transfer_native_points_for_shape(
            self.num_inputs,
            self.num_outputs,
            self.has_view_key,
        ));
        let storage_cost = rates.storage_cost(self.persisted_bytes_upper_bound() as u64);
        // The receipt occupies a created slot of its own, on top of one per stealth output. Spent
        // inputs already exist, so they allocate nothing.
        let create_cost = (self.num_outputs as u64)
            .saturating_add(1)
            .saturating_mul(rates.per_substate_create_cost());

        weight_cost
            .saturating_add(runtime_call_cost)
            .saturating_add(native_cost)
            .saturating_add(storage_cost)
            .saturating_add(create_cost)
    }

    /// An upper bound on the bytes of permanent state this shape persists: the UTXOs it creates and
    /// the transaction receipt that records the whole thing.
    ///
    /// A spent UTXO leaves state rather than being rewritten as spent, so its only footprint is its
    /// downed entry in the receipt's diff summary.
    fn persisted_bytes_upper_bound(&self) -> usize {
        self.persisted_output_bytes
            .saturating_add(self.receipt_bytes_upper_bound())
    }

    /// An upper bound on the receipt this shape finalizes into. Each created UTXO takes one upped
    /// diff-summary entry and each spent one a downed entry; the receipt is absent from its own
    /// summary, and the transfer emits no events and withdraws no validator fees.
    fn receipt_bytes_upper_bound(&self) -> usize {
        let upped = vec![SubstateId::Utxo(widest_utxo_address()); self.num_outputs];
        let downed = vec![SubstateId::Utxo(widest_utxo_address()); self.num_inputs];
        // The epoch is measured at its widest rather than taken from the caller: a receipt priced
        // for one epoch would otherwise come in under a transaction that lands in a wider one.
        TransactionReceipt::encoded_size_upper_bound(&[], &[], upped.iter(), downed.iter(), Epoch(u64::MAX))
    }
}

/// The shape of an L1 burn claim that pays its own fee out of the burn it claims.
///
/// This is the transaction a wallet builds to claim a burn: the claim mints the burnt value as a UTXO,
/// one stealth transfer spends it into a single stealth output and reveals the fee, and the revealed
/// bucket is paid straight to `pay_fee`. A fee paid from a bucket is not refunded, so whatever the
/// claim reveals is gone even where the claim costs a fraction of it. [`Self::estimate_fee`] prices
/// the shape locally, giving a claimant the most the claim can cost without trusting anyone else's
/// figure for it.
///
/// Everything but the stored output and the transaction weight is fixed by the shape. The UTXO the
/// claim mints is spent by the same transaction, so it never reaches state and costs only the
/// verification of spending it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimBurnShape {
    /// Bytes the claimed value's stealth output occupies in persisted state, measured with
    /// [`persisted_utxo_bytes`] from the output the builder generated.
    pub persisted_output_bytes: usize,
    /// The weight of the transaction this shape builds into, which the builder supplies for the same
    /// reason [`MergedStealthTransferShape::transaction_weight`] does.
    pub transaction_weight: u64,
}

impl ClaimBurnShape {
    /// Engine host calls this shape's instruction sequence makes: the merged transfer's three, as the
    /// engine runs the claim instruction directly. `the_runtime_call_count_matches_the_claim_sequence` in
    /// `tari_engine`'s claim-burn tests holds it to what the engine counts.
    pub const RUNTIME_CALLS: u64 = MergedStealthTransferShape::RUNTIME_CALLS;

    /// An upper bound, in microtari, on what the engine charges a transaction of this shape.
    pub fn estimate_fee(&self, rates: &FeeRates) -> u64 {
        let weight_cost = self
            .transaction_weight
            .saturating_mul(rates.per_transaction_weight_cost());
        let runtime_call_cost = Self::RUNTIME_CALLS.saturating_mul(rates.per_module_call_cost());
        // The claim's proof verification, then a transfer spending the claimed UTXO into one output.
        // TARI carries no view key.
        let native_cost = rates.execution_cost(
            NativeExecutionPoints::PER_CLAIM_BURN.saturating_add(transfer_native_points_for_shape(1, 1, false)),
        );
        let storage_cost = rates.storage_cost(self.persisted_bytes_upper_bound() as u64);
        // The tombstone, the output and the receipt each occupy a created slot.
        let create_cost = 3u64.saturating_mul(rates.per_substate_create_cost());

        weight_cost
            .saturating_add(runtime_call_cost)
            .saturating_add(native_cost)
            .saturating_add(storage_cost)
            .saturating_add(create_cost)
    }

    /// An upper bound on the bytes of permanent state this shape persists: the stealth output, the
    /// tombstone that marks the burn claimed, and the receipt.
    fn persisted_bytes_upper_bound(&self) -> usize {
        let tombstone = encoded_len(&SubstateValue::ClaimedOutputTombstone(ClaimedOutputTombstone {
            value: u64::MAX,
        }));
        let upped = [
            SubstateId::ClaimedOutputTombstone(ClaimedOutputTombstoneAddress::new(ObjectKey::from_array(
                [0xff; ObjectKey::LENGTH],
            ))),
            SubstateId::Utxo(widest_utxo_address()),
        ];
        let receipt = TransactionReceipt::encoded_size_upper_bound(&[], &[], upped.iter(), [].iter(), Epoch(u64::MAX));
        self.persisted_output_bytes
            .saturating_add(tombstone)
            .saturating_add(receipt)
    }
}

/// The bytes the engine persists for the UTXO a stealth output becomes.
///
/// A statement's output is not what is stored. The commitment moves into the substate's address, and
/// the viewable balance is kept in place of the proof it was verified from — so the stored form has
/// to be reconstructed to be measured. Measuring it is what lets a builder price the outputs it
/// generated exactly, instead of at a width wide enough for any output.
pub fn persisted_utxo_bytes(output: &StealthUnspentOutput) -> usize {
    let utxo =
        Utxo::new(UtxoOutput {
            output: OutputBody {
                public_nonce: output.output.sender_public_nonce,
                encrypted_data: output.output.encrypted_data.clone(),
                minimum_value_promise: output.output.minimum_value_promise,
                // Both halves are fixed-width points, so a stand-in measures the same as the balance the
                // engine derives from this proof.
                viewable_balance: output.output.viewable_balance_proof.as_ref().map(|_| {
                    ElgamalVerifiableBalanceBytes {
                        encrypted: RistrettoPublicKeyBytes::zero(),
                        public_nonce: RistrettoPublicKeyBytes::zero(),
                    }
                }),
            },
            auth: output.auth.clone(),
            tag: output.tag,
        });
    encoded_len(&SubstateValue::Utxo(utxo))
}

/// A UTXO address at the width every UTXO address takes — its parts are all fixed-size arrays, so
/// the content is immaterial and only the shape of the encoding matters.
fn widest_utxo_address() -> UtxoAddress {
    UtxoAddress::new(
        ResourceAddress::new([0xff; 32].into()),
        UtxoId::from_array([0xff; UtxoId::LENGTH]),
    )
}

/// The bytes a value occupies in persisted state, measured the way the fee module tallies storage.
fn encoded_len<T: minicbor::Encode<()>>(value: &T) -> usize {
    tari_bor::encoded_len_via_writer(value).expect("encoding a canonical value into a byte counter cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::NativeExecutionPoints as P;

    /// Priced like the shipped tables, so the terms are legible in microtari.
    fn rates() -> FeeRates {
        FeeRates {
            per_transaction_weight_cost: 1,
            per_module_call_cost: 1,
            per_byte_storage_cost: 1,
            per_substate_create_cost: 250,
            per_wasm_point_cost: 1,
            storage_cost_divisor: 1,
            wasm_points_cost_divisor: 1000,
        }
    }

    /// A key-path output with no memo, the shape of an ordinary send's outputs.
    const TYPICAL_OUTPUT_BYTES: usize = 162;

    fn shape(num_inputs: usize, num_outputs: usize) -> MergedStealthTransferShape {
        MergedStealthTransferShape {
            num_inputs,
            num_outputs,
            persisted_output_bytes: num_outputs * TYPICAL_OUTPUT_BYTES,
            has_view_key: false,
            transaction_weight: 150,
        }
    }

    #[test]
    fn an_output_costs_its_verification_its_storage_and_its_slot() {
        let one = shape(1, 1).estimate_fee(&rates());
        let two = shape(1, 2).estimate_fee(&rates());

        let native = P::PER_OUTPUT / 1000;
        let slot = 250;
        assert!(
            two - one > native + slot,
            "an added output must cost its verification, its slot and the bytes of both the UTXO and its receipt entry"
        );
    }

    /// An input aggregates a commitment and then leaves state, leaving only its downed entry in the
    /// receipt; an output verifies a range proof and occupies a new slot for good.
    #[test]
    fn an_input_costs_its_verification_and_its_receipt_entry() {
        let extra_input = shape(2, 1).estimate_fee(&rates()) - shape(1, 1).estimate_fee(&rates());
        let downed_entry = shape(2, 1).receipt_bytes_upper_bound() - shape(1, 1).receipt_bytes_upper_bound();
        assert!(downed_entry > 0);
        assert_eq!(extra_input, P::PER_INPUT / 1000 + downed_entry as u64);

        let extra_output = shape(1, 2).estimate_fee(&rates()) - shape(1, 1).estimate_fee(&rates());
        assert!(extra_output > extra_input);
    }

    #[test]
    fn a_view_key_costs_its_proof_verification_per_output() {
        let plain = shape(1, 2);
        let viewable = MergedStealthTransferShape {
            has_view_key: true,
            ..plain
        };
        // The bytes a viewable balance adds are already in `persisted_output_bytes`, so what the
        // flag itself buys is the ElGamal proof verification on each output.
        assert_eq!(
            viewable.estimate_fee(&rates()) - plain.estimate_fee(&rates()),
            2 * P::PER_OUTPUT_VIEWABLE_SURCHARGE / 1000
        );
    }

    #[test]
    fn a_memo_is_paid_for_by_the_byte() {
        let plain = shape(1, 2);
        let with_memo = MergedStealthTransferShape {
            persisted_output_bytes: plain.persisted_output_bytes + 100,
            ..plain
        };
        assert_eq!(with_memo.estimate_fee(&rates()) - plain.estimate_fee(&rates()), 100);
    }

    /// The estimate has to move with the shape monotonically, or a settling loop that raises its fee
    /// could select a wider shape that prices lower and oscillate.
    #[test]
    fn a_larger_shape_never_prices_lower() {
        let mut previous = 0;
        for num_outputs in 1..=8 {
            for num_inputs in 0..=8 {
                let estimate = shape(num_inputs, num_outputs).estimate_fee(&rates());
                assert!(estimate > 0);
                if num_inputs == 0 {
                    assert!(estimate > previous, "adding an output must not price lower");
                }
                previous = previous.max(estimate);
            }
        }
    }
}
