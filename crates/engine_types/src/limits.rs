//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

pub struct WasmLimits {
    /// Maximum number of function arguments
    pub max_function_arguments: usize,
    /// Maximum length function names
    pub max_function_name_length: usize,
    /// Maximum number of a functions
    pub max_functions: usize,
    /// Maximum memory size in pages (64KiB each)
    pub max_memory_pages: usize,
    /// Maximum number of globals a module may declare. Each one occupies a slot in the instance's
    /// VM context, which is built at every instantiation, while declaring one costs a handful of
    /// bytes — so the count is bounded rather than left to the binary size. `rustc`'s wasm32 output
    /// declares a handful, its stack pointer among them.
    pub max_globals: usize,
    /// Maximum number of tables a module may declare. Each table's elements are bounded by
    /// [`WasmLimits::max_table_elements`], so together the two bound the host storage a module's
    /// tables can claim at instantiation. `rustc`'s wasm32 output declares one table, its
    /// `__indirect_function_table`.
    pub max_tables: usize,
    /// Maximum number of elements in a table. Every table a module declares is capped at this many
    /// entries, whether or not the module declares a maximum of its own: a table's storage is a
    /// host-side `Vec` of function references, so an uncapped `table.grow` is a host allocation
    /// sized by a guest operand. The cap is well above `max_functions`, which bounds what a
    /// template's own `__indirect_function_table` needs.
    pub max_table_elements: u32,
}

pub const WASM_LIMITS: WasmLimits = WasmLimits {
    max_function_arguments: 32,
    max_function_name_length: 256,
    max_functions: 8192,
    max_memory_pages: 32, // ~2MiB = 32 * 64KiB
    max_globals: 1024,
    max_tables: 4,
    max_table_elements: 16_384,
};

/// Maximum Wasmer metering points a single template invocation may consume. Enforced by the
/// metering middleware compiled into the engine (see `tari_engine::wasm::module::create_engine`):
/// exceeding it traps the call with an out-of-gas error.
pub const MAX_WASM_POINTS_PER_CALL: u64 = 250_000_000;

/// Maximum Wasmer metering points a whole transaction may consume, summed across every template
/// invocation it makes (top-level instructions and nested cross-template calls). Each invocation
/// otherwise gets a fresh per-call budget, so without this a transaction could multiply its
/// execution time by stacking instructions or recursing to `ENGINE_LIMITS.max_call_depth`. Enforced
/// in `WasmProcess::invoke` by capping each call's allowance to the budget remaining for the
/// transaction. Kept equal to the per-call cap: a transaction gets one compute budget, shared across
/// all its calls. The aggregate across a *block* still needs a separate per-block budget.
///
/// ~30ms of validator CPU at the rate [`NativeExecutionPoints`] is calibrated against (~8.4M
/// points/ms). Two bounds meet here.
///
/// From below, a template must be able to carry cryptography heavy enough to be worth writing in
/// WASM at all. A Groth16/BN254 verification costs ~96M points at one public input and ~176M at
/// sixteen (`cargo run -p tari_engine --release --example zk_points_calibrate`), so this admits one
/// with room for the contract logic around it. A transaction may already spend
/// [`MAX_NATIVE_POINTS_PER_TRANSACTION`] (~286ms) on native verification, so a WASM ceiling far
/// below that is an asymmetry with nothing behind it — both are real CPU on every replica and both
/// are priced identically.
///
/// From above, no single transaction should be able to claim an outsized share of a block. This is
/// a fraction of `max_block_execution_points`, so a block always admits at least
/// `MIN_MAX_COMPUTE_TRANSACTIONS_PER_BLOCK` transactions running flat out — raising it further
/// trades that granularity away, and buys nothing: the next tier of proving system (PLONK, FRI)
/// does not fit in WASM at any ceiling the block budget could support.
pub const MAX_WASM_POINTS_PER_TRANSACTION: u64 = 250_000_000;

/// The granularity floor [`MAX_WASM_POINTS_PER_TRANSACTION`] is sized against:
/// `max_block_execution_points` must admit at least this many transactions each running the WASM
/// ceiling flat out, so a leader packing max-compute transactions cannot starve a block of
/// everything else. Asserted against the shipped consensus constants in `tari_consensus`.
///
/// Scoped to the WASM ceiling, which is what it bounds. A transaction's native verification is
/// governed separately by [`MAX_NATIVE_POINTS_PER_TRANSACTION`] and the structural caps in
/// [`STEALTH_LIMITS`] and [`CONFIDENTIAL_LIMITS`]; that ceiling is large enough that a block of
/// native-heavy transactions admits far fewer than this many.
pub const MIN_MAX_COMPUTE_TRANSACTIONS_PER_BLOCK: u64 = 18;

/// Maximum native-verification points (priced by [`NativeExecutionPoints`]) a whole transaction may consume. The
/// native counterpart of [`MAX_WASM_POINTS_PER_TRANSACTION`], enforced in `StateTracker::charge_native_execution`.
///
/// Both per-transaction ceilings exist so the block execution budget has a bounded overshoot: a leader only learns
/// a transaction's cost after executing it, so an honest block may exceed the propose budget by one transaction's
/// worth, and `max_block_validation_execution_points` must leave room for it. Without this cap the native half of
/// that overshoot is bounded only by the structural limits, which permit ~2.2e9 points of stealth verification —
/// and, for `ClaimBurn`, only by transaction weight, which permits ~2.9e9. Either would exceed the headroom and
/// get honest blocks rejected.
///
/// Sized just above the most expensive statement set the structural caps allow ([`STEALTH_LIMITS`]: 64 transfers,
/// 256 outputs each carrying the view-key surcharge, 1024 inputs ≈ 2.23e9), so no transaction the other limits
/// admit can trip this one. Tightening it means tightening those caps first.
pub const MAX_NATIVE_POINTS_PER_TRANSACTION: u64 = 2_400_000_000;

/// Execution metering points the fee intent may consume, whatever it has paid. A transaction sources
/// its fee in the fee intent (withdraw, claim-burn, AMM swap to TARI, stealth transfer, …) and only
/// then calls `pay_fee`, so it must be allowed to run some compute on credit; this bounds that
/// credit. Each WASM call's metering allowance is capped to what remains of it
/// (`WasmProcess::invoke`), and native verification pre-charges its point cost against the same
/// figure, so a fee intent that exceeds it traps out-of-gas rather than consuming the full
/// [`MAX_WASM_POINTS_PER_TRANSACTION`] (or unmetered native crypto). This is the bound on total free
/// compute — WASM and native — a transaction can extract from a validator.
///
/// The credit is flat: a payment does not raise it. A fee intent that fails leaves no checkpoint to
/// fall back to, so the transaction settles as a rejection that collects nothing — compute funded by
/// a payment there would be work done and never paid for, repeatably, with the same funds. Anything
/// needing more than the credit belongs in the main instructions, where the fee already paid funds
/// it (`StateTracker::compute_allowance`) and a failure still commits the fee intent.
///
/// Sized at ~3x the most expensive legitimate fee-sourcing flow: paying a fee from stealth UTXOs
/// (one transfer: fixed cost + 1 stealth change output + up to 64 dust inputs ≈ 10.8M points at
/// the calibrated native prices below — fees are TARI, which has no view key, so the flow prices
/// at the base output rate). The other fee-sourcing flows are far cheaper: a burn claim is
/// [`NativeExecutionPoints::PER_CLAIM_BURN`] and an AMM swap to TARI is ~143k WASM points
/// (guarded by `tari_engine`'s `complex_fee_payment` test). Re-derive with
/// `cargo run -p tari_engine --example native_points_calibrate --release`.
pub const FREE_COMPUTE_GRACE_POINTS: u64 = 32_000_000;

/// Metering-point prices for native (non-WASM) verification work, charged against the same
/// payment-funded allowance as WASM execution ([`FREE_COMPUTE_GRACE_POINTS`] of credit, then
/// payments fund the rest). Native crypto runs outside the Wasmer meter, so these price it by
/// wall-clock equivalence: measured milliseconds × the measured points-per-millisecond rate of
/// real metered WASM on the same hardware. Both sides are CPU-bound, so the ratio holds across
/// validator classes. Values from `cargo run -p tari_engine --example native_points_calibrate
/// --release` (~8.4M points/ms), rounded up.
pub struct NativeExecutionPoints;

impl NativeExecutionPoints {
    /// One Minotari burn-claim proof: a Schnorr ownership proof plus commitment arithmetic (the
    /// same primitives as [`Self::PER_STATEMENT`]) and a bounded kernel-MMR inclusion proof
    /// (Blake2b hashes, microseconds). Priced as the statement cost with headroom.
    pub const PER_CLAIM_BURN: u64 = 3_200_000;
    /// One ElGamal (DLEQ) value proof. It verifies two Schnorr-style equations over four decompressed
    /// points rather than one, so it is priced at double the mask-knowledge variant.
    pub const PER_ELGAMAL_VALUE_PROOF: u64 = 1_200_000;
    /// Fixed cost of a hash invocation, charged on top of [`Self::PER_HASH_BYTE`].
    pub const PER_HASH: u64 = 10_000;
    /// Each byte fed to a hash.
    pub const PER_HASH_BYTE: u64 = 30;
    /// One stealth/confidential input commitment: decompress + point aggregation (~4.8µs measured).
    /// Substate access is charged separately by the fee module.
    pub const PER_INPUT: u64 = 42_000;
    /// One stealth/confidential output: its share of the aggregated bulletproof range proof
    /// (~0.68ms measured marginal, no view key). Outputs on a resource with a view key add
    /// [`Self::PER_OUTPUT_VIEWABLE_SURCHARGE`] each.
    pub const PER_OUTPUT: u64 = 6_000_000;
    /// Added to [`Self::PER_OUTPUT`] for each output on a resource with a view key: the ElGamal
    /// viewable-balance proof (~0.26ms measured marginal). Charged only once the resource's view
    /// key presence is known — a cheap substate read that precedes all proof crypto.
    pub const PER_OUTPUT_VIEWABLE_SURCHARGE: u64 = 2_000_000;
    /// Fixed cost of a multi-scalar multiplication, before its per-term charge.
    pub const PER_RISTRETTO_MSM: u64 = 100_000;
    /// Each term of a multi-scalar multiplication. Below [`Self::PER_RISTRETTO_MUL`] because the
    /// terms are evaluated by one Straus/Pippenger multiscalar multiplication
    /// (`RistrettoPublicKey::batch_mul`) rather than multiplied one at a time, so the windowed
    /// tables and the accumulation are shared across them. Charging the full multiplication rate per
    /// term would price work the batch does not do; a per-term rate is only defensible while the
    /// implementation actually batches.
    pub const PER_RISTRETTO_MSM_TERM: u64 = 250_000;
    /// One variable-base Ristretto scalar multiplication, decompression included.
    pub const PER_RISTRETTO_MUL: u64 = 550_000;
    /// One fixed-base multiplication of the Ristretto basepoint. Cheaper than the variable-base
    /// case: the basepoint's multiples are precomputed and there is no point to decompress.
    pub const PER_RISTRETTO_MUL_BASE: u64 = 300_000;
    /// One Ristretto group operation: decompress the operands, one addition or negation, recompress.
    pub const PER_RISTRETTO_OP: u64 = 50_000;
    /// One scalar field operation. No point decompression, so orders of magnitude below a
    /// multiplication on the group.
    pub const PER_SCALAR_OP: u64 = 3_000;
    /// One Ristretto Schnorr signature verification.
    pub const PER_SCHNORR_VERIFY: u64 = 1_100_000;
    /// Fixed per-statement cost: balance-proof Schnorr verification, bulletproof base cost and
    /// basic validations (~0.24ms measured).
    pub const PER_STATEMENT: u64 = 2_100_000;
    /// One mask-knowledge value proof (supply-tracked mints and burns): a Schnorr verification plus
    /// commitment arithmetic (~60µs class), priced with headroom.
    pub const PER_VALUE_PROOF: u64 = 600_000;
}

pub struct EngineLimits {
    pub max_substate_outputs: usize,
    pub max_substate_size: usize,
    pub max_call_size: usize,
    pub max_internal_call_size: usize,
    pub max_logs: usize,
    pub max_log_size_bytes: usize,
    /// Maximum number of `tari_debug` messages one template instance may write. Debug output is
    /// validator I/O that never reaches the transaction result, so it carries its own budget rather
    /// than drawing on [`EngineLimits::max_logs`].
    pub max_debug_messages: usize,
    pub max_events: usize,
    /// Maximum CBOR-encoded size of a single event.
    ///
    /// Every event a transaction emits is carried in its transaction receipt, which is persisted as a
    /// substate like any other but is built after fees settle, so it cannot be rejected for being
    /// oversized. Its event payload is bounded here instead: `max_events * max_event_size_bytes` must
    /// stay under [`EngineLimits::max_substate_size`] with room for the receipt's diff summary
    /// (up to [`EngineLimits::max_substate_outputs`] entries) and fee breakdown.
    pub max_event_size_bytes: usize,
    pub max_panic_message_size: usize,
    pub max_template_binary_size_bytes: usize,
    pub max_template_name_length: usize,
    pub max_call_depth: usize,
    pub max_random_bytes_len: usize,
}

pub const ENGINE_LIMITS: EngineLimits = EngineLimits {
    max_substate_outputs: 1000,
    max_substate_size: 1024 * 1024,      // 1 MiB
    max_call_size: 128 * 1024,           // 128 KiB
    max_internal_call_size: 1024 * 1024, // 1 MiB
    max_logs: 256,
    max_log_size_bytes: 32 * 1024, // 32 KiB
    max_debug_messages: 256,
    max_events: 256,
    max_event_size_bytes: 2 * 1024, // 2 KiB; 256 * 2 KiB = 512 KiB of the 1 MiB substate budget
    max_panic_message_size: 32 * 1024, // 32 KiB
    max_template_binary_size_bytes: 3 * 512 * 1024, // 1.5 MiB
    max_template_name_length: 64,
    max_call_depth: 10,
    max_random_bytes_len: 1024, // 1 KiB per call
};

pub const MAX_DIVISIBILITY: u8 = 18;

pub const MAX_TOKEN_SYMBOL_LEN: usize = 10;

/// Maximum number of `PublishTemplate` instructions a single transaction may contain.
///
/// Publishing a template registers a new global substate and carries a WASM binary up to
/// [`ENGINE_LIMITS`]`.max_template_binary_size_bytes`. Capping at one keeps each publishing transaction to a single,
/// bounded template registration; multiple publishes would stack several large binaries and their validation/storage
/// cost into one transaction with no benefit a caller cannot get from separate transactions. The engine enforces this
/// during execution — a consensus rule applied uniformly by every validator — and the mempool mirrors it to reject
/// such transactions at ingress.
pub const MAX_PUBLISH_TEMPLATES_PER_TRANSACTION: usize = 1;

pub struct StealthLimits {
    /// Maximum stealth inputs in a single transfer statement.
    pub max_inputs: usize,
    /// Maximum stealth outputs in a single transfer statement. Bounded by
    /// [`crate::crypto::MAX_LAZY_BP_AGG_FACTORS`], the largest aggregated range proof the engine can verify.
    ///
    /// A statement's outputs share one aggregated bulletproof, so verification cost per output falls as the
    /// statement grows while the proof itself grows only logarithmically. Splitting the same outputs across
    /// statements instead pays [`NativeExecutionPoints::PER_STATEMENT`] and an extra change output each time, so
    /// this cap shapes a transaction rather than bounding its cost —
    /// [`StealthLimits::max_total_outputs_per_transaction`] does that.
    pub max_outputs: usize,
    /// Maximum number of conditions in a single `SpendCondition` conjunction (TIP-0006). A revealed leaf is
    /// evaluated in full at spend time, and a builtin predicate (e.g. a covenant balance proof or a hashlock) runs
    /// native, unmetered work — so an unbounded conjunction would be a denial-of-service amplifier. This caps the
    /// worst-case work of evaluating one leaf. The condition tree itself supplies breadth (a spender reveals only one
    /// leaf plus a logarithmic inclusion proof), so the tree's size is not a spend-time cost and is not bounded here.
    pub max_conditions_per_conjunction: usize,
    /// Maximum size, in bytes, of the witness `data` blob a script-path spend may supply (`SpendWitness::ScriptPath`).
    /// The blob is processed natively by the revealed leaf's predicates, so it is bounded to cap that work. A hashlock
    /// preimage or a signature is far smaller; this leaves room for a small CBOR structure a `TemplateFunction`
    /// decodes.
    pub max_witness_data_len: usize,
    /// Maximum number of sibling hashes in a script-path inclusion proof (`MerkleProof`). The proof is
    /// spender-supplied and each sibling costs one native hash in `verify_inclusion`, so its length is bounded to
    /// keep that work constant. A proof of length `n` corresponds to a condition tree of up to `2^n` leaves, so
    /// this ceiling is far beyond any real tree (whose breadth is otherwise unbounded — see
    /// `max_conditions_per_conjunction`).
    pub max_inclusion_proof_len: usize,
    /// Maximum number of stealth transfers across a whole transaction.
    pub max_transfers_per_transaction: usize,
    /// Maximum number of stealth transfers the fee intent may perform.
    ///
    /// The fee intent runs on [`FREE_COMPUTE_GRACE_POINTS`] of credit before any payment, so whatever it contains is
    /// the transaction's free-execution surface. Sourcing a fee needs one transfer statement — inputs producing the
    /// revealed fee amount plus a stealth change output — so one is what the fee intent gets. Further transfers
    /// belong in the main intent, where the fee just paid funds them.
    ///
    /// Counts transfers *performed*, not `StealthTransfer` instructions: a template calling
    /// `ResourceManager::stealth_transfer` counts the same, since both routes reach
    /// `RuntimeInterfaceImpl::stealth_transfer`. Counting instructions alone would leave the WASM route uncapped and
    /// so make the costlier route — a WASM invocation and host call on top of the same verification — the way to
    /// exceed this limit.
    pub max_fee_intent_transfers: usize,
    /// Maximum total stealth inputs across a whole transaction.
    pub max_total_inputs_per_transaction: usize,
    /// Maximum total stealth outputs across a whole transaction.
    pub max_total_outputs_per_transaction: usize,
}

/// Verifying a stealth transfer is native work dominated by the per-output bulletproof range proof and ElGamal
/// viewable-balance proof (~1ms per output on x86-class hardware). It is priced in metering points by
/// [`NativeExecutionPoints`] and counted toward the per-block execution budget, so the block-level bound is the
/// budget rather than these caps. The per-transfer limits bound one statement and the per-transaction limits bound
/// the aggregate, capping how much verification a single transaction can stack — which keeps any one transaction
/// from consuming a whole block's budget by itself. The per-transaction caps are a consensus-relevant execution
/// rule enforced uniformly during execution, not just a mempool heuristic.
pub const STEALTH_LIMITS: StealthLimits = StealthLimits {
    max_inputs: 1000,
    max_outputs: 16,
    max_conditions_per_conjunction: 16,
    max_witness_data_len: 4096,
    max_inclusion_proof_len: 32,
    max_transfers_per_transaction: 64,
    max_fee_intent_transfers: 1,
    max_total_inputs_per_transaction: 1024,
    max_total_outputs_per_transaction: 256,
};

pub struct ConfidentialLimits {
    /// Maximum input commitments spent by a single confidential withdraw proof.
    pub max_inputs: usize,
    /// Maximum confidential withdraws in a single transaction.
    pub max_withdraws_per_transaction: usize,
    /// Maximum input commitments spent across all confidential withdraws in a single transaction.
    pub max_total_inputs_per_transaction: usize,
}

/// Spending confidential outputs is native, unmetered work: each input commitment is a separate substate that must be
/// locked and read plus folded into the balance-proof point aggregation, and each withdraw verifies a bulletproof range
/// proof over its (at most two) outputs. The per-withdraw limit bounds one proof; the per-transaction limits bound the
/// aggregate so a single transaction cannot stack enough native verification and substate access to stall the proposing
/// leader. These are consensus-relevant execution rules enforced uniformly during execution, not mempool heuristics.
pub const CONFIDENTIAL_LIMITS: ConfidentialLimits = ConfidentialLimits {
    max_inputs: 1000,
    max_withdraws_per_transaction: 64,
    max_total_inputs_per_transaction: 1024,
};
