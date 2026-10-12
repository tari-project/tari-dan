//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

pub struct WasmLimits {
    /// Maximum number of function arguments
    pub max_function_arguments: usize,
    /// Maximum length function names
    pub max_function_name_length: usize,
    /// Maximum number of functions a template's ABI definition may list.
    pub max_functions: usize,
    /// Maximum number of functions a module may contain, imported and defined together. Every one
    /// of them is Cranelift-compiled at publish, at a cost per function that its bytes do not
    /// reflect: an empty function is four bytes of binary. `rustc`'s wasm32 output for the
    /// built-in templates holds 150 to 360.
    pub max_module_functions: usize,
    /// Maximum number of parameters and locals a module's functions may declare, summed over every
    /// function. Cranelift declares each as a variable of its own, while a run of locals is a
    /// single `(count, type)` pair of a few bytes. The built-in templates declare 1,000 to 3,300.
    pub max_module_variables: usize,
    /// Maximum number of metering checks the compile may emit into one function (see
    /// [`TEMPLATE_COMPILE_BYTES_PER_METERING_CHECK`]). Wasmer compiles each function on a single
    /// thread, at a cost that grows with the square of these, so a module's largest function is
    /// billed by [`TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE`], and this cap is the count at
    /// which that bill reaches the size cap. `rustc`'s wasm32 output for the built-in templates needs at most ~4,900
    /// in one function, the account template's generated `Account_main` dispatcher, which grows by
    /// ~160 for each of the 30 functions in the template's ABI.
    pub max_function_metering_checks: usize,
    /// Maximum number of `loop`s one function may contain. Each one costs Cranelift far more than
    /// the other operators a metering check follows, and that cost grows with how many share the
    /// function. The built-in templates hold at most 66 in one function.
    pub max_function_loops: usize,
    /// Maximum number of `loop`s a module may contain, summed over its functions. A `loop` and its
    /// metering checks compile in ~55 µs, the byte price of ~220 bytes against the three bytes it
    /// takes, and wasmer parallelises a compile only across ~33 KB runs of function bytes, so a
    /// module of loops compiles nearly serially. One at this cap measures at ~115 ms, up to ~4x its
    /// charge. The built-in templates hold at most 522.
    pub max_module_loops: usize,
    /// Maximum depth to which one function may nest `block`, `loop` and `if`. Cranelift's time on
    /// a function grows with the nesting as well as the count. The built-in templates nest at most
    /// 121 deep.
    pub max_function_nesting_depth: usize,
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
    max_module_functions: 2048,
    max_module_variables: 512 * 1024,
    max_function_metering_checks: 8192,
    max_function_loops: 512,
    max_module_loops: 2048,
    max_function_nesting_depth: 1024,
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
/// 256 outputs each carrying the view-key surcharge, 1024 inputs ≈ 2.23e9). Tightening it means tightening those
/// caps first.
///
/// A template publish is the one charge sized *against* this cap rather than bounded independently of it:
/// [`template_compile_points`] for a binary at [`EngineLimits::max_template_binary_size_bytes`] comes to ~2.34e9,
/// and that limit is chosen so the remainder still covers a fee intent (see
/// `the_largest_publishable_binary_leaves_room_to_source_a_fee`). So a max-size publish *can* reach this cap, by
/// construction — it is what makes the publish limit binding. Template return values are the other flow this cap
/// bounds: [`return_value_points`] is bounded per call, not per transaction, and about 18 maximal item-heavy
/// returns reach the cap, failing the transaction out of compute. Every remaining flow stays under it on the
/// structural caps alone.
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

/// Points charged for compiling a published template binary, before the compile runs.
///
/// A publish hands the engine a binary and makes every validator Cranelift-compile it. That is by
/// far the most expensive thing a single instruction can ask for — measured at 52 ms for a 151 KiB
/// binary and 142 ms for a 530 KiB one — and it is not metered WASM, so nothing else bounds it.
///
/// Charged against the same allowance as native verification, which means
/// [`MAX_NATIVE_POINTS_PER_TRANSACTION`] is what bounds how large a binary can be published:
/// `max_template_binary_size_bytes` is set to a size this charge leaves affordable, so an
/// unaffordable publish is refused for its size rather than part-way through paying for it.
///
/// The binary is billed at [`PER_TEMPLATE_COMPILE_BYTE`] for whichever is larger: its size, or
/// its [`template_compile_count_bytes`]. The byte price is fitted over templates whose functions
/// average 800 to 1500 bytes, declare under one variable per hundred bytes and need one metering
/// check per ~19 bytes, so it carries those costs within it; a module denser in any of them is
/// billed by its counts.
///
/// From `cargo run -p tari_engine --release --example instantiation_points_calibrate`, fitted over
/// the built-in templates and converted at the calibrated ~8.4M points/ms, rounded up.
pub const fn template_compile_points(binary_bytes: u64, counts: &CompileCounts) -> u64 {
    let count_bytes = template_compile_count_bytes(counts);
    let billed_bytes = if count_bytes > binary_bytes {
        count_bytes
    } else {
        binary_bytes
    };
    PER_TEMPLATE_COMPILE.saturating_add(PER_TEMPLATE_COMPILE_BYTE.saturating_mul(billed_bytes))
}

/// The binary size whose compile [`PER_TEMPLATE_COMPILE_BYTE`] prices the same as what `counts`
/// asks the compile to emit.
///
/// Wasmer compiles a module's functions in parallel, each on a single thread, so this is whichever
/// is larger of two bounds. The work all the functions add up to is
/// [`TEMPLATE_COMPILE_BYTES_PER_FUNCTION`] for each function,
/// [`TEMPLATE_COMPILE_BYTES_PER_VARIABLE`] for each parameter and local and
/// [`TEMPLATE_COMPILE_BYTES_PER_METERING_CHECK`] for each metering check. The compile also takes
/// at least as long as its largest function, which grows with the square of that function's
/// metering checks: see [`TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE`].
///
/// A module may ask for no more than [`EngineLimits::max_template_binary_size_bytes`] of it, so no
/// publish is charged more than one at the size cap. The function, variable and per-function
/// metering-check caps keep their own shares of it under the size cap; the metering checks summed
/// over a module are bounded by this alone.
pub const fn template_compile_count_bytes(counts: &CompileCounts) -> u64 {
    let all_functions = counts
        .functions
        .saturating_mul(TEMPLATE_COMPILE_BYTES_PER_FUNCTION)
        .saturating_add(counts.variables.saturating_mul(TEMPLATE_COMPILE_BYTES_PER_VARIABLE))
        .saturating_add(
            counts
                .metering_checks
                .saturating_mul(TEMPLATE_COMPILE_BYTES_PER_METERING_CHECK),
        );
    let largest_function = counts
        .largest_function_metering_checks
        .saturating_mul(counts.largest_function_metering_checks) /
        TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE;
    if largest_function > all_functions {
        largest_function
    } else {
        all_functions
    }
}

/// Fixed cost of a compile, independent of the binary: ~16 ms of Cranelift setup.
pub const PER_TEMPLATE_COMPILE: u64 = 140_000_000;

/// Each byte of the published binary. The marginal measured cost is ~2000 points/byte.
///
/// This price carries a thinner margin over its measurement (~1.06x) than the other native prices
/// here, which sit at 1.8x and above because the points-per-millisecond conversion moves with the
/// validator's microarchitecture. Cranelift's throughput relative to the Wasmer meter is exactly
/// that kind of ratio, so on hardware below the published recommended specification a max-size
/// publish costs more wall-clock than it is charged for. That is accepted: the margin is what sets
/// [`EngineLimits::max_template_binary_size_bytes`], since
/// [`MAX_NATIVE_POINTS_PER_TRANSACTION`] bounds the charge, and widening it to 2x would put the
/// publish limit below the size of the built-in account template. A validator under-specified
/// enough for this to bite should expect to miss its leader slots for the same reason.
pub const PER_TEMPLATE_COMPILE_BYTE: u64 = 2_100;

/// The fewest bytes a function is billed as by [`template_compile_points`]. Cranelift spends ~47 µs
/// on a function however small it is, mostly compiling the metering check injected into its entry,
/// and that is what [`PER_TEMPLATE_COMPILE_BYTE`] charges for ~215 bytes. Rounded up with a
/// ~1.2x margin.
pub const TEMPLATE_COMPILE_BYTES_PER_FUNCTION: u64 = 256;

/// The fewest bytes a parameter or local is billed as by [`template_compile_points`]. Cranelift
/// spends ~50 ns declaring a local and ~110 ns a parameter, against the ~250 ns of compile
/// [`PER_TEMPLATE_COMPILE_BYTE`] charges for.
pub const TEMPLATE_COMPILE_BYTES_PER_VARIABLE: u64 = 1;

/// The fewest bytes a metering check is billed as by [`template_compile_points`].
///
/// The compile emits a check against the metering allowance after every operator the metering
/// middleware ends a basic block at — a `loop`, `if`, `else`, `end`, branch, call or `return` — and
/// after every operator the bulk meter charges by length. Each check is a conditional trap, and the
/// checks are where Cranelift's time on these operators goes, while the operator they follow can be
/// as small as one byte. Modules of blocks and branches asking for the most compile work a module
/// may, spread over 32 to 2,047 functions, measure at 0.8 to 1.5x their charge.
///
/// The built-in templates need a check per ~19 bytes and are billed by their counts, at 1.03 to
/// 1.28x their size; each still measures at or above its charge.
pub const TEMPLATE_COMPILE_BYTES_PER_METERING_CHECK: u64 = 16;

/// A module's largest function is billed as the square of its metering checks over this many
/// bytes.
///
/// Within one function, Cranelift's time grows by 2.8e-6 to 3.6e-6 ms per check squared on top of
/// a linear cost, measured over runs of blocks and branches: the byte price of one byte for every
/// ~58 to ~76 checks squared. Set to the value at which a function at
/// [`WasmLimits::max_function_metering_checks`] is billed as the size cap, where the costliest such
/// function, its linear cost compiling on the same thread, measures at 2 to 3x its charge. A lower
/// value buys a wider margin only with a lower function cap. The account template's largest
/// function, its generated `Account_main` dispatcher of ~4,900 checks, is billed as ~370 KiB,
/// under the account template's own size.
pub const TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE: u64 = 64;

const _: () = assert!(
    WASM_LIMITS.max_module_functions as u64 * TEMPLATE_COMPILE_BYTES_PER_FUNCTION +
        WASM_LIMITS.max_module_variables as u64 * TEMPLATE_COMPILE_BYTES_PER_VARIABLE <=
        ENGINE_LIMITS.max_template_binary_size_bytes as u64,
    "a module at the function and variable caps must not cost more to publish than one at the size cap"
);

const _: () = assert!(
    (WASM_LIMITS.max_function_metering_checks as u64 * WASM_LIMITS.max_function_metering_checks as u64) /
        TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE <=
        ENGINE_LIMITS.max_template_binary_size_bytes as u64,
    "a function at the metering-check cap must not cost more to publish than a module at the size cap"
);

/// What a module asks the compile to emit beyond its bytes. Priced by [`template_compile_points`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompileCounts {
    /// Functions, imported and defined together.
    pub functions: u64,
    /// Parameters and locals, summed over every function.
    pub variables: u64,
    /// Metering checks, summed over every function. See
    /// [`TEMPLATE_COMPILE_BYTES_PER_METERING_CHECK`].
    pub metering_checks: u64,
    /// Metering checks in the function that has the most of them. See
    /// [`TEMPLATE_COMPILE_FUNCTION_CHECKS_SQUARED_PER_BYTE`].
    pub largest_function_metering_checks: u64,
}

/// Fixed cost of one instantiation: mapping the memory, wiring the imports and building the tables.
/// Measured at 0.008 to 0.010 ms across runs, taken at the top of that spread.
pub const PER_TEMPLATE_INSTANTIATION: u64 = 100_000;

/// Each byte of data segment copied into the fresh linear memory. The per-byte figure measures at 3
/// to 8 points depending on how warm the allocator is, so it is set above the middle of that
/// spread: the built-in templates end up charged ~1.8x what they measure, and a module carrying the
/// largest data segment a publish admits ~2.1x.
pub const PER_TEMPLATE_DATA_SEGMENT_BYTE: u64 = 5;

/// Each active data segment, whatever its length.
///
/// The per-byte price alone would make a zero-length segment free, and a module's data section is
/// not capped: ~5 bytes of binary buys one such segment, and `Instance::new` still evaluates its
/// offset expression, bounds-checks it against linear memory and calls into the copy. 10,000 of them
/// measure at 0.685 ms — 58x what the flat instantiation cost covers. Measured at ~590 points each.
pub const PER_TEMPLATE_DATA_SEGMENT: u64 = 700;

/// Each slot a declared table claims, whether or not anything is written into it.
///
/// Allocating and zeroing the tables is separate from filling them: four tables of
/// `max_table_elements` with no element section at all measure at 0.114 ms against 192 bytes of
/// binary. Measured at ~13 points per slot.
pub const PER_TEMPLATE_TABLE_SLOT: u64 = 16;

/// Each element-segment entry written into a table.
///
/// Active element segments initialise the instance's tables at every instantiation, the same class
/// of work as copying the data segments but per funcref rather than per byte.
///
/// What bounds the entry count is the binary size, not the table limits. Nothing caps how many
/// element segments a module declares, and several may target one table at overlapping offsets —
/// each is written out in turn — so `max_tables` x `max_table_elements` (65,536) is the widest a
/// single pass over the tables can be, not the most a module can ask for. An entry is a LEB128 func
/// index, so roughly a byte of binary buys one: a publish at
/// [`EngineLimits::max_template_binary_size_bytes`] can carry on the order of a million.
///
/// 65,536 entries measure at ~0.21 ms once the table allocation they used to be conflated with is
/// priced separately by [`PER_TEMPLATE_TABLE_SLOT`], i.e. ~28 points each. Set above that.
pub const PER_TEMPLATE_ELEMENT_ENTRY: u64 = 32;

/// What a module's binary says about the work instantiating it will cost.
///
/// Compiled code is laid down once at publish; what every instantiation repeats is building the
/// instance's storage from the module — allocating its tables, and writing its active segments into
/// memory and into those tables. Those counts, not the binary size, are what
/// [`instantiation_points`] prices. Populated by `tari_engine`'s module loader.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModuleShape {
    /// Bytes across all active data segments.
    pub data_segment_bytes: u64,
    /// Number of active data segments. Each is walked, its offset evaluated and bounds-checked,
    /// whatever its length — a zero-length segment costs ~5 bytes of binary and does real work.
    pub data_segment_count: u64,
    /// Entries across all active element segments.
    pub element_segment_entries: u64,
    /// Slots the module's tables claim at instantiation, summed over its declared tables. Allocating
    /// and zeroing them happens whether or not any element segment writes to them.
    pub declared_table_slots: u64,
}

/// Points charged for building the `Store` and `Instance` a template call runs in, before its first
/// metered operator.
///
/// Charged on every call to a template, including nested cross-template calls. A call runs on a new
/// instance or on one the transaction already created, restored to its freshly instantiated state;
/// both pay this. Instantiating maps linear memory, copies the module's data segments into it,
/// writes its element segments into its tables, and wires up the imports. Compiled
/// code is laid down once at publish and costs nothing to instantiate, so the only part that scales
/// with the binary is the data copy — measured across the built-in templates, a 150 KiB and a 520
/// KiB module instantiate in the same ~0.015 ms because both carry a few KiB of data. Pricing this
/// off the binary size would therefore overcharge a code-heavy template by an order of magnitude.
///
/// Both figures from `cargo run -p tari_engine --release --example instantiation_points_calibrate`,
/// rounded up.
pub const fn instantiation_points(shape: &ModuleShape) -> u64 {
    PER_TEMPLATE_INSTANTIATION
        .saturating_add(PER_TEMPLATE_DATA_SEGMENT_BYTE.saturating_mul(shape.data_segment_bytes))
        .saturating_add(PER_TEMPLATE_DATA_SEGMENT.saturating_mul(shape.data_segment_count))
        .saturating_add(PER_TEMPLATE_ELEMENT_ENTRY.saturating_mul(shape.element_segment_entries))
        .saturating_add(PER_TEMPLATE_TABLE_SLOT.saturating_mul(shape.declared_table_slots))
}

/// Points charged for the host-side handling of a template call's return value, before it is
/// decoded.
///
/// A template function hands the engine up to `max_call_size` bytes of CBOR. The engine decodes it
/// into an `IndexedValue`, indexes it for well-known types, validates what it references, keeps a
/// clone as the instruction output and encodes it again for a calling template. That work runs
/// outside the Wasmer meter and follows the number of CBOR items far more than the number of bytes —
/// a run of one-byte integers is an item a byte, where a byte string is one item however long — so
/// `items` is counted by [`tari_bor::count_data_items`] and priced per item, with the copies priced
/// per byte.
///
/// Measured with `cargo run -p tari_engine --release --example return_value_points_calibrate`, which
/// times that whole sequence over return values filling `max_call_size` in each item shape.
pub const fn return_value_points(bytes: u64, items: u64) -> u64 {
    NativeExecutionPoints::PER_RETURN_VALUE_ITEM
        .saturating_mul(items)
        .saturating_add(NativeExecutionPoints::PER_RETURN_VALUE_BYTE.saturating_mul(bytes))
}

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
    /// Each byte of a template call's return value, which the engine copies out of linear memory,
    /// into the decoded value, its clone and its re-encoding. A single 128 KiB byte string, which is
    /// nothing but those copies, measures at ~0.7 points a byte. Charged by [`return_value_points`].
    pub const PER_RETURN_VALUE_BYTE: u64 = 1;
    /// Each CBOR data item in a template call's return value: counting it, decoding it into a `Value`
    /// node, visiting it for well-known types, cloning it and encoding it again. Arrays of one-byte
    /// integers or empty containers measure at ~450-600 points an item; the costliest shapes, where
    /// every item is an allocation of its own (one-character strings, singly nested arrays), at
    /// ~800-900. Set above those. Charged by [`return_value_points`].
    pub const PER_RETURN_VALUE_ITEM: u64 = 1_000;
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
    /// One scalar inversion: an exponentiation by `l - 2`, hundreds of field multiplications rather
    /// than one, so within a small factor of a group multiplication (~11µs measured through the
    /// intrinsic dispatch). Priced with the headroom the group multiplications carry.
    pub const PER_SCALAR_INVERT: u64 = 200_000;
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
    /// Maximum encoded bytes one transaction may persist: every substate it writes, published
    /// templates included, plus its transaction receipt.
    ///
    /// `max_substate_outputs × max_substate_size` is about a gigabyte, and the substates of one entity
    /// share a shard. This bounds what one transaction adds to a shard's state version well inside the
    /// consensus per-shard block budget, which a lone transaction must fit. It must leave room for the
    /// largest legitimate transaction: a max-size template publish alongside a max-size substate and
    /// a full receipt.
    pub max_transaction_output_bytes: usize,
    pub max_call_size: usize,
    pub max_internal_call_size: usize,
    pub max_logs: usize,
    pub max_log_size_bytes: usize,
    /// Maximum number of `tari_debug` messages one template call may write. Debug output is
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
    /// Largest WASM binary a `PublishTemplate` instruction may carry.
    ///
    /// This is a policy rule about what may be published, applied by `PublishTemplateLimitValidator` at mempool
    /// ingress and block validation and again by the engine at execution. It is free to move within
    /// [`crate::published_template::MAX_TEMPLATE_BLOB_WIRE_BYTES`], which is what the substate format can carry and
    /// therefore what an already-published template needs to stay readable.
    pub max_template_binary_size_bytes: usize,
    pub max_template_name_length: usize,
    pub max_call_depth: usize,
    pub max_random_bytes_len: usize,
}

pub const ENGINE_LIMITS: EngineLimits = EngineLimits {
    max_substate_outputs: 1000,
    max_substate_size: 1024 * 1024,                // 1 MiB
    max_transaction_output_bytes: 4 * 1024 * 1024, // 4 MiB
    max_call_size: 128 * 1024,                     // 128 KiB
    max_internal_call_size: 1024 * 1024,           // 1 MiB
    max_logs: 256,
    max_log_size_bytes: 32 * 1024, // 32 KiB
    max_debug_messages: 256,
    max_events: 256,
    max_event_size_bytes: 2 * 1024, // 2 KiB; 256 * 2 KiB = 512 KiB of the 1 MiB substate budget
    max_panic_message_size: 32 * 1024, // 32 KiB
    // Sized by what a publish costs, not by what a binary could hold: every validator
    // Cranelift-compiles it, that compile is charged by `template_compile_points`, and the charge
    // has to leave a whole fee intent's worth of compute inside `MAX_NATIVE_POINTS_PER_TRANSACTION`
    // so publishing and sourcing the fee confidentially remain possible in one transaction. Asserted
    // by `the_largest_publishable_binary_leaves_room_to_source_a_fee`. The largest built-in template
    // is ~530 KiB.
    max_template_binary_size_bytes: 1024 * 1024, // 1 MiB
    max_template_name_length: 64,
    max_call_depth: 10,
    max_random_bytes_len: 1024, // 1 KiB per call
};

/// Maximum container nesting accepted from an untrusted CBOR payload.
///
/// Applied at each boundary where bytes this node did not produce first reach a native decoder: a
/// published template's `tari_tdef` section, a gossiped or RPC-carried message, a peer's substate
/// bytes and a transaction's engine arguments. A derived `Decode` on a self-recursive type descends
/// one native stack frame per level and cannot thread a counter of its own, so a payload of a few
/// hundred bytes reaches the end of the stack — a guard-page abort of the validator process, which
/// no caller can catch. Every boundary applies the same figure, because a payload one node accepts
/// and another rejects is a consensus split.
///
/// It sits generously above anything a legitimate payload nests to, because rejecting a valid
/// payload is the worse failure. This figure guarantees that the deepest accepted input still decodes
/// within the smallest stack untrusted decode runs on: a 2 MiB tokio worker.
pub const MAX_CBOR_NESTING_DEPTH: usize = 256;

pub const MAX_DIVISIBILITY: u8 = 18;

pub const MAX_TOKEN_SYMBOL_LEN: usize = 10;

/// Maximum number of `PublishTemplate` instructions a single transaction may contain.
///
/// Publishing a template registers a new global substate and carries a WASM binary up to
/// [`ENGINE_LIMITS`]`.max_template_binary_size_bytes`. Capping at one keeps each publishing transaction to a single,
/// bounded template registration; multiple publishes would stack several large binaries and their validation/storage
/// cost into one transaction with no benefit a caller cannot get from separate transactions. Enforced by
/// `PublishTemplateLimitValidator`, which backs both mempool ingress and block validation, so every validator applies
/// it to every transaction before it can execute — a validator chain that omits it bounds nothing.
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
    /// evaluated in full at spend time, and a builtin predicate (e.g. a timelock or a hashlock) runs native,
    /// unmetered work — so an unbounded conjunction would be a denial-of-service amplifier. This caps the
    /// worst-case work of evaluating one leaf. A covenant balance proof is metered and verified once per partition,
    /// so repeating it within a leaf costs only a cached lookup. The condition tree itself supplies breadth (a spender
    /// reveals only one leaf plus a logarithmic inclusion proof), so the tree's size is not a spend-time cost and
    /// is not bounded here.
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

#[cfg(test)]
mod publish_budget_tests {
    use super::*;

    /// The publish cap must leave a transaction enough of the native budget to source its fee
    /// confidentially, since that budget is a hard cap and a publisher has no way to discover that
    /// the binary size is what made the transaction unaffordable.
    #[test]
    fn the_largest_publishable_binary_leaves_room_to_source_a_fee() {
        let largest = ENGINE_LIMITS.max_template_binary_size_bytes as u64;
        let compile = template_compile_points(largest, &CompileCounts {
            functions: WASM_LIMITS.max_module_functions as u64,
            variables: WASM_LIMITS.max_module_variables as u64,
            metering_checks: 0,
            largest_function_metering_checks: 0,
        });

        assert!(
            compile + FREE_COMPUTE_GRACE_POINTS <= MAX_NATIVE_POINTS_PER_TRANSACTION,
            "a {largest}-byte publish costs {compile} points and leaves {} of the {MAX_NATIVE_POINTS_PER_TRANSACTION} \
             budget, under the {FREE_COMPUTE_GRACE_POINTS} a fee intent may spend",
            MAX_NATIVE_POINTS_PER_TRANSACTION.saturating_sub(compile),
        );

        // Without an upper bound the reserve could swallow the cap and nobody would notice.
        assert!(
            template_compile_points(largest + 128 * 1024, &CompileCounts::default()) + FREE_COMPUTE_GRACE_POINTS >
                MAX_NATIVE_POINTS_PER_TRANSACTION,
            "the publish cap is well under what the budget admits and is costing publishers room"
        );
    }
}
