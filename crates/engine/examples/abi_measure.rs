//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Measures what the WASM ABI boundary costs today, in validator wall time and in guest metering
//! points, so a decision to replace minicbor on that boundary rests on numbers rather than on
//! intuition. It changes nothing about the ABI itself.
//!
//!     cargo run -p tari_engine --example abi_measure --release --features abi-metrics
//!
//! Four hops cross the boundary per template invocation:
//!
//! | Hop | Direction | Cost falls on |
//! |---|---|---|
//! | 1 | host to guest, call arguments | host encode, guest decode (metered) |
//! | 2 | guest to host, return value | guest encode (metered), host `IndexedValue` walk |
//! | 3 | guest to host, engine-call argument | guest encode (metered), host decode |
//! | 4 | host to guest, engine-call response | host encode, guest decode (metered) |
//!
//! The report has three parts. The **probe** section prices each hop in guest points by running
//! `tests/templates/abi_probe`, whose functions come in pairs that differ by exactly one boundary
//! crossing; subtracting the pair and taking a slope across payload size or call count leaves the
//! hop's own cost. The **census** section runs realistic transactions with
//! `tari_engine::abi_metrics` on and reports, per engine op, how many calls were made, how many
//! bytes crossed, and how much host time went to serialisation rather than to the runtime handler.
//! The **summary** applies the probe's fits to the census counts to estimate what share of a real
//! transaction's points and wall time the boundary accounts for.

#[cfg(not(feature = "abi-metrics"))]
fn main() {
    eprintln!(
        "abi_measure needs the instrumentation it reads on:\n    cargo run -p tari_engine --example abi_measure \
         --release --features abi-metrics"
    );
    std::process::exit(1);
}

#[cfg(feature = "abi-metrics")]
fn main() {
    measure::run();
}

#[cfg(feature = "abi-metrics")]
mod measure {
    use std::time::Instant;

    use tari_engine::abi_metrics;
    use tari_ootle_common_types::substate_type::SubstateType;
    use tari_ootle_transaction::{Epoch, Transaction, args, builder::named_args::NamedArg};
    use tari_template_abi::EngineOp;
    use tari_template_lib::{
        args::metrics as value_metrics,
        types::{Amount, ComponentAddress, NonFungibleAddress, ResourceAddress, bytes::Bytes},
    };
    use tari_template_test_tooling::TemplateTest;

    const CRATE_PATH: &str = env!("CARGO_MANIFEST_DIR");
    const PROBE: &str = "tests/templates/abi_probe";
    const TARISWAP: &str = "tests/templates/tariswap";
    const FAUCET: &str = "tests/templates/faucet";

    /// Payload sizes for the size-scaling fits, in bytes. The largest stays under
    /// `ENGINE_LIMITS.max_call_size` (128 KiB) so hop 1 can carry it.
    const SIZES: [u32; 5] = [0, 1_024, 8_192, 32_768, 65_536];
    /// Call counts for the per-call slope fits. `max_logs` (256) bounds the upper one.
    const COUNTS: [u32; 2] = [32, 224];
    /// Transactions per census workload. The fastest run is reported: a slower run differs by
    /// scheduling noise, not by work done.
    const CENSUS_TRIALS: usize = 7;

    pub fn run() {
        let timer_ns = abi_metrics::timer_overhead_ns();
        println!("Timer overhead per instrumented phase: {timer_ns:.1} ns (two clock reads).");
        println!("Host nanoseconds below include it; subtract it once per phase call to compare against the handler.");

        let fits = probe_points();
        buffer_strategies();
        metering_of_bulk_copies();
        let censuses = census(timer_ns);
        summary(&fits, &censuses, timer_ns);
    }

    // -- Part 1: guest points per hop ---------------------------------------------------------

    /// A hop priced as `points = intercept + slope * x`, where `x` is payload bytes or call count.
    struct Fit {
        label: &'static str,
        unit: &'static str,
        intercept: f64,
        slope: f64,
    }

    impl Fit {
        fn at(&self, x: f64) -> f64 {
            self.intercept + self.slope * x
        }
    }

    struct Fits {
        hop1_arg: Fit,
        hop2_return: Fit,
        hop3_arg: Fit,
        state_get_typed: Fit,
        /// Points for an engine call whose response is a bare `()`: envelope encode, the host call
        /// and the length-prefixed free.
        bare_call: f64,
        /// Points to decode an engine call's response into the concrete type, over
        /// [`Self::bare_call`].
        response_decode: f64,
    }

    fn probe_points() -> Fits {
        println!("\nCompiling probe template...");
        let mut test = TemplateTest::new(CRATE_PATH, [PROBE]);

        let noop = call_points(&mut test, "noop", args![]);
        println!("\n== Guest points, by hop ==");
        println!("Empty call (dispatch, header decode, unit return): {noop} points");

        // Hop 1: argument decode. `sink_bytes` returns a u32, so only the argument scales.
        let hop1: Vec<(f64, f64)> = SIZES
            .iter()
            .map(|&size| {
                let points = call_points(&mut test, "sink_bytes", args![Bytes::from_vec(vec![
                    0x5A;
                    size as usize
                ])]);
                (f64::from(size), points as f64)
            })
            .collect();
        let hop1_arg = fit("hop 1: whole call, arg decode", "byte", &hop1);
        table(
            "Hop 1: one `sink_bytes(Bytes)` call, whole invocation",
            "arg bytes",
            &hop1,
        );

        // Hop 2: return encode, less the allocation its twin also performs.
        let hop2: Vec<(f64, f64)> = SIZES
            .iter()
            .map(|&size| {
                let full = call_points(&mut test, "make_bytes", args![size]);
                let glue = call_points(&mut test, "make_bytes_glue", args![size]);
                (f64::from(size), full.saturating_sub(glue) as f64)
            })
            .collect();
        let hop2_return = fit("hop 2: return encode", "byte", &hop2);
        table("Hop 2: returning `Bytes` rather than a scalar", "return bytes", &hop2);

        // Hop 3: engine-call argument encode, sized by the log message it carries.
        let hop3: Vec<(f64, f64)> = SIZES
            .iter()
            .filter(|&&size| size <= 16_384)
            .map(|&size| {
                let per_call = slope_over_counts(&mut test, "emit_logs", "emit_logs_glue", size);
                (f64::from(size), per_call)
            })
            .collect();
        let hop3_arg = fit("hop 3: engine arg encode", "byte", &hop3);
        table("Hop 3: one `EmitLog` call, argument only", "message bytes", &hop3);

        // Hops 3 and 4 with payloads of a few bytes each way: the per-engine-call floor, split
        // into the three stages a response passes through.
        let floor = slope_over_counts_single(&mut test, "caller_context_calls");
        let raw = slope_over_counts_single(&mut test, "caller_context_raw");
        let bare_call = hop3_arg.intercept;
        println!();
        println!("One engine call with payloads of a few tens of bytes, by stage (guest points):");
        println!("  argument encode + host call + unit response     {bare_call:>8.0}   (EmitLog, empty message)");
        println!(
            "  response decoded into the concrete type         {:>8.0}   (CallerContextInvoke, 32-byte key)",
            floor - bare_call
        );
        println!("  total                                          {floor:>8.0}");
        println!(
            "  the same response built as a Value tree         {:>8.0}   (what a response no longer does)",
            raw - bare_call
        );

        let (state_get_typed, state_get_value, state_set) = state_fits(&mut test);
        let value_tree_fixed = state_get_value.intercept - state_get_typed.intercept;
        let value_tree_slope = state_get_value.slope - state_get_typed.slope;

        println!(
            "\nOne argument costs {:.0} points before its first byte: the {noop}-point empty invocation is already \
             paid.",
            hop1_arg.intercept - noop as f64
        );
        println!();
        println!("{:<34} {:>14} {:>16}", "hop", "fixed (points)", "per unit (points)");
        println!("{}", "-".repeat(66));
        for f in [
            &hop1_arg,
            &hop2_return,
            &hop3_arg,
            &state_get_typed,
            &state_get_value,
            &state_set,
        ] {
            println!("{:<34} {:>14.0} {:>10.3} /{}", f.label, f.intercept, f.slope, f.unit);
        }
        println!(
            "\nBuilding a Value tree from a GetState response rather than decoding it: {value_tree_fixed:.0} points \
             fixed, {value_tree_slope:.3} points/byte"
        );

        Fits {
            hop1_arg,
            hop2_return,
            hop3_arg,
            state_get_typed,
            bare_call,
            response_decode: floor - bare_call,
        }
    }

    /// Points consumed by one call of a probe function. The return value is discarded: decoding it
    /// happens on the host and is not what is being priced.
    /// Component state round-trips, priced against the size of the state that crosses.
    fn state_fits(test: &mut TemplateTest) -> (Fit, Fit, Fit) {
        let mut typed = Vec::new();
        let mut value_only = Vec::new();
        let mut set = Vec::new();
        for &size in &[0u32, 1_024, 8_192, 32_768] {
            let component: ComponentAddress = test.call_function("AbiProbe", "new", args![size], vec![]);
            typed.push((
                f64::from(size),
                method_slope_over_counts(test, component, "get_state_typed", None),
            ));
            value_only.push((
                f64::from(size),
                method_slope_over_counts(test, component, "get_state_raw", None),
            ));
            set.push((
                f64::from(size),
                method_slope_over_counts(test, component, "set_state", Some("set_state_glue")),
            ));
        }
        table(
            "Hop 4: one GetState round trip, decoded into the type",
            "state bytes",
            &typed,
        );
        table(
            "Hop 4: one GetState round trip, response Value only",
            "state bytes",
            &value_only,
        );
        table("Hop 3: one SetState call", "state bytes", &set);
        (
            fit("hop 4: GetState into a type", "state byte", &typed),
            fit("hop 4: GetState, response as a Value", "state byte", &value_only),
            fit("hop 3: SetState", "state byte", &set),
        )
    }

    fn call_points(test: &mut TemplateTest, func: &str, args: Vec<NamedArg>) -> u64 {
        let template = test.get_template_address("AbiProbe");
        test.execute_expect_success(
            test.transaction()
                .call_function(template, func, args)
                .build_and_seal(test.secret_key()),
            vec![],
        );
        test.last_execution_points().wasm
    }

    fn method_points(test: &mut TemplateTest, component: ComponentAddress, method: &str, args: Vec<NamedArg>) -> u64 {
        test.execute_expect_success(
            test.transaction()
                .call_method(component, method, args)
                .build_and_seal(test.secret_key()),
            vec![],
        );
        test.last_execution_points().wasm
    }

    /// Points per iteration of `func`, less those of `glue`, from the slope across [`COUNTS`].
    fn slope_over_counts(test: &mut TemplateTest, func: &str, glue: &str, size: u32) -> f64 {
        let net = |test: &mut TemplateTest, n: u32| -> f64 {
            let full = call_points(test, func, args![n, size]);
            let glue = call_points(test, glue, args![n, size]);
            full.saturating_sub(glue) as f64
        };
        let low = net(test, COUNTS[0]);
        let high = net(test, COUNTS[1]);
        (high - low) / f64::from(COUNTS[1] - COUNTS[0])
    }

    fn slope_over_counts_single(test: &mut TemplateTest, func: &str) -> f64 {
        let low = call_points(test, func, args![COUNTS[0]]) as f64;
        let high = call_points(test, func, args![COUNTS[1]]) as f64;
        (high - low) / f64::from(COUNTS[1] - COUNTS[0])
    }

    fn method_slope_over_counts(
        test: &mut TemplateTest,
        component: ComponentAddress,
        method: &str,
        glue: Option<&str>,
    ) -> f64 {
        let net = |test: &mut TemplateTest, n: u32| -> f64 {
            let full = method_points(test, component, method, args![n]);
            let glue_points = glue.map_or(0, |glue| method_points(test, component, glue, args![n]));
            full.saturating_sub(glue_points) as f64
        };
        let low = net(test, COUNTS[0]);
        let high = net(test, COUNTS[1]);
        (high - low) / f64::from(COUNTS[1] - COUNTS[0])
    }

    /// The measurements a fit is drawn through. Points are deterministic, so these are exact.
    fn table(title: &str, x_label: &str, points: &[(f64, f64)]) {
        println!("\n{title}:");
        println!("  {x_label:>14} {:>10}", "points");
        for (x, y) in points {
            println!("  {x:>14.0} {y:>10.0}");
        }
    }

    /// Ordinary least squares through `(x, y)`.
    fn fit(label: &'static str, unit: &'static str, points: &[(f64, f64)]) -> Fit {
        let n = points.len() as f64;
        let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / n;
        let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / n;
        let covariance: f64 = points.iter().map(|(x, y)| (x - mean_x) * (y - mean_y)).sum();
        let variance: f64 = points.iter().map(|(x, _)| (x - mean_x).powi(2)).sum();
        let slope = if variance == 0.0 { 0.0 } else { covariance / variance };
        Fit {
            label,
            unit,
            intercept: mean_y - slope * mean_x,
            slope,
        }
    }

    /// What the `encoded_len` pre-pass costs the guest, and what the alternatives cost. The probe
    /// encodes a realistic `SetState` argument, varying only how the destination buffer is obtained.
    fn buffer_strategies() {
        const COUNTS: [u32; 2] = [64, 512];
        const PAYLOADS: [u32; 4] = [64, 512, 4_096, 16_384];
        const STRATEGIES: [(&str, u32); 5] = [
            ("encoded_len, exact buffer", 0),
            ("empty buffer, grown", 1),
            ("512-byte buffer", 2),
            ("1024-byte buffer", 3),
            ("4096-byte buffer", 4),
        ];

        let mut test = TemplateTest::new(CRATE_PATH, [PROBE]);
        let component: ComponentAddress = test.call_function("AbiProbe", "new", args![0u32], vec![]);

        println!("\n== Guest points per engine-call argument encode ==");
        print!("{:<28}", "buffer strategy");
        for size in PAYLOADS {
            print!("{:>16}", format!("{size} B payload"));
        }
        println!();
        println!("{}", "-".repeat(76));
        for (label, strategy) in STRATEGIES {
            print!("{label:<28}");
            for size in PAYLOADS {
                let low = method_points(&mut test, component, "encode_buffered", args![
                    COUNTS[0], size, strategy
                ]);
                let high = method_points(&mut test, component, "encode_buffered", args![
                    COUNTS[1], size, strategy
                ]);
                let slope = (high - low) as f64 / f64::from(COUNTS[1] - COUNTS[0]);
                print!("{slope:>16.0}");
            }
            println!();
        }
    }

    /// Why the per-byte figures above are near zero: `memory.copy` is metered at a flat rate
    /// whatever its length, so bytes crossing the boundary are close to free in points while
    /// costing the validator real time. Reported as points per microsecond of execution against a
    /// dependent integer chain, work whose points do track time.
    fn metering_of_bulk_copies() {
        const COPY_BYTES: u32 = 256 * 1024;
        const COPY_ROUNDS: [u32; 2] = [2_000, 10_000];
        const SPIN_ROUNDS: [u32; 2] = [200_000, 1_000_000];

        let mut test = TemplateTest::new(CRATE_PATH, [PROBE]);
        let slope = |test: &mut TemplateTest, func: &str, rounds: [u32; 2], extra: Option<u32>| -> (f64, f64) {
            let run = |test: &mut TemplateTest, n: u32| -> (f64, f64) {
                let args = extra.map_or_else(|| args![n], |extra| args![n, extra]);
                let start = Instant::now();
                let points = call_points(test, func, args);
                (start.elapsed().as_nanos() as f64, points as f64)
            };
            let (low_ns, low_points) = run(test, rounds[0]);
            let (high_ns, high_points) = run(test, rounds[1]);
            let span = f64::from(rounds[1] - rounds[0]);
            ((high_ns - low_ns) / span, (high_points - low_points) / span)
        };

        let (spin_ns, spin_points) = slope(&mut test, "spin", SPIN_ROUNDS, None);

        println!("\n== Side finding: what a byte costs the meter ==");
        println!(
            "{:<24} {:>10} {:>12} {:>14}",
            "work per round", "points", "ns", "points/µs"
        );
        println!("{}", "-".repeat(62));
        println!(
            "{:<24} {spin_points:>10.1} {spin_ns:>12.1} {:>14.1}",
            "dependent integer chain",
            spin_points * 1000.0 / spin_ns
        );
        let mut copy = (0.0, 0.0);
        for bytes in [16 * 1024, 64 * 1024, COPY_BYTES] {
            copy = slope(&mut test, "bulk_copy", COPY_ROUNDS, Some(bytes));
            println!(
                "{:<24} {:>10.1} {:>12.1} {:>14.3}",
                format!("{} KiB memory.copy", bytes / 1024),
                copy.1,
                copy.0,
                copy.1 * 1000.0 / copy.0
            );
        }
        let (copy_ns, copy_points) = copy;
        println!(
            "A copied byte costs {:.4} points, so the meter charges {:.0}x less per microsecond for copying than for \
             arithmetic.",
            copy_points / f64::from(COPY_BYTES),
            (spin_points / spin_ns) / (copy_points / copy_ns),
        );
        println!(
            "At the {} million point per-transaction cap that is {:.0} GB of copying in one transaction, {:.1} s of \
             validator time.",
            tari_engine_types::limits::MAX_WASM_POINTS_PER_TRANSACTION / 1_000_000,
            (limits_cap() / copy_points) * f64::from(COPY_BYTES) / 1e9,
            (limits_cap() / copy_points) * copy_ns / 1e9,
        );
    }

    fn limits_cap() -> f64 {
        tari_engine_types::limits::MAX_WASM_POINTS_PER_TRANSACTION as f64
    }

    // -- Part 2: host time and wire bytes on real transactions --------------------------------

    struct WorkloadCensus {
        name: &'static str,
        wall_ns: u64,
        wasm_points: u64,
        /// Whether the trials disagreed on points, i.e. the workload did not run the same
        /// transaction each time.
        points_vary: bool,
        census: abi_metrics::Census,
        result_encode: value_metrics::Phase,
        result_decode: value_metrics::Phase,
    }

    impl WorkloadCensus {
        /// Every host nanosecond attributed to ABI serialisation, the `Value` conversions included.
        fn serde_ns(&self) -> u64 {
            self.census.ops().values().map(|op| op.serde_ns()).sum::<u64>() +
                self.census.call_info_size_pass().ns +
                self.census
                    .call_info_encode()
                    .ns
                    .saturating_sub(self.census.guest_alloc().ns) +
                self.census.return_decode().ns +
                self.result_encode.ns +
                self.result_decode.ns
        }

        fn engine_calls(&self) -> u64 {
            self.census.ops().values().map(|op| op.calls).sum()
        }
    }

    fn census(timer_ns: f64) -> Vec<WorkloadCensus> {
        println!("\n\nCompiling census workload templates...");
        let mut test = TemplateTest::new(CRATE_PATH, [TARISWAP, FAUCET, PROBE]);
        let swap = SwapFixture::setup(&mut test);
        let probe = test.get_template_address("AbiProbe");

        let mut out = Vec::new();
        out.push(measure_workload("account create + fund", &mut test, |test| {
            test.create_funded_account();
        }));
        out.push(measure_workload("tariswap swap", &mut test, |test| {
            swap.swap(test, Amount::from(10u32));
        }));
        out.push(measure_workload("account balance read", &mut test, |test| {
            let _: Amount = test.call_method(swap.account, "balance", args![swap.a_resource], vec![]);
        }));
        // Hops 1 and 2 at a size where the host's own work is visible: a 64 KiB argument, then a
        // 64 KiB return value walked by `IndexedValue`.
        let payload = Bytes::from_vec(vec![0x5A; 64 * 1024]);
        out.push(measure_workload("64 KiB call argument", &mut test, |test| {
            test.execute_expect_success(
                test.transaction()
                    .call_function(probe, "sink_bytes", args![payload])
                    .build_and_seal(test.secret_key()),
                vec![],
            );
        }));
        out.push(measure_workload("64 KiB return value", &mut test, |test| {
            test.execute_expect_success(
                test.transaction()
                    .call_function(probe, "make_bytes", args![64u32 * 1024])
                    .build_and_seal(test.secret_key()),
                vec![],
            );
        }));

        for workload in &out {
            report_workload(workload, timer_ns);
        }
        out
    }

    fn measure_workload(
        name: &'static str,
        test: &mut TemplateTest,
        mut execute: impl FnMut(&mut TemplateTest),
    ) -> WorkloadCensus {
        // One untimed run pays for anything the first execution caches.
        execute(test);

        let mut best: Option<WorkloadCensus> = None;
        let mut all_points = Vec::with_capacity(CENSUS_TRIALS);
        for _ in 0..CENSUS_TRIALS {
            abi_metrics::reset();
            value_metrics::reset();
            let start = Instant::now();
            execute(test);
            let wall_ns = start.elapsed().as_nanos() as u64;
            let candidate = WorkloadCensus {
                name,
                wall_ns,
                wasm_points: test.last_execution_points().wasm,
                points_vary: false,
                census: abi_metrics::snapshot(),
                result_encode: value_metrics::encode(),
                result_decode: value_metrics::decode(),
            };
            all_points.push(candidate.wasm_points);
            if best.as_ref().is_none_or(|best| candidate.wall_ns < best.wall_ns) {
                best = Some(candidate);
            }
        }
        let mut best = best.expect("at least one trial");
        // Every figure reported describes one trial, so a ratio drawn between two of them is a
        // ratio over the same transaction. Points are deterministic per transaction, so trials
        // that disagree mean the workload mutated state and ran a different transaction each time;
        // that makes the points column incomparable across runs, which the flag says rather than
        // the numbers hiding it.
        best.points_vary = all_points.iter().any(|p| *p != all_points[0]);
        best
    }

    fn report_workload(workload: &WorkloadCensus, timer_ns: f64) {
        println!("\n== Census: {} ==", workload.name);
        println!(
            "Wall time {:.1} µs, {} WASM points{}, {} engine calls",
            workload.wall_ns as f64 / 1000.0,
            workload.wasm_points,
            if workload.points_vary {
                " (lowest; trials differ, so this workload is not a stable point comparison)"
            } else {
                ""
            },
            workload.engine_calls()
        );
        println!();
        println!(
            "{:<26} {:>6} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "engine op", "calls", "arg B/call", "rsp B/call", "decode ns", "encode ns", "alloc ns", "handler ns"
        );
        println!("{}", "-".repeat(98));
        let mut ops: Vec<_> = workload.census.ops().iter().collect();
        ops.sort_by_key(|(_, stats)| std::cmp::Reverse(stats.serde_ns()));
        for (op, stats) in ops {
            let calls = stats.calls as f64;
            println!(
                "{:<26} {:>6} {:>10.0} {:>10.0} {:>10.0} {:>10.0} {:>10.0} {:>10.0}",
                EngineOp::from_i32(*op).map_or_else(|| format!("op {op:#x}"), |op| op.to_string()),
                stats.calls,
                stats.arg_bytes as f64 / calls,
                stats.resp_bytes as f64 / calls,
                stats.decode_ns as f64 / calls,
                stats.encode_ns as f64 / calls,
                stats.alloc_ns as f64 / calls,
                stats.handler_ns as f64 / calls,
            );
        }

        let size_pass = workload.census.call_info_size_pass();
        let call_info = workload.census.call_info_encode();
        let alloc = workload.census.guest_alloc();
        let returns = workload.census.return_decode();
        println!();
        println!(
            "hop 1 host size pass (CallInfo):     {:>4} calls, {:>8.0} B/call, {:>8.0} ns/call",
            size_pass.count,
            size_pass.mean_bytes(),
            size_pass.mean_ns()
        );
        println!(
            "hop 1 host encode (CallInfo):        {:>4} calls, {:>8.0} B/call, {:>8.0} ns/call, of which {:.0} ns is \
             the guest tari_alloc",
            call_info.count,
            call_info.mean_bytes(),
            call_info.mean_ns(),
            alloc.mean_ns()
        );
        println!(
            "hop 2 host decode (IndexedValue):    {:>4} calls, {:>8.0} B/call, {:>8.0} ns/call",
            returns.count,
            returns.mean_bytes(),
            returns.mean_ns()
        );
        println!(
            "InvokeResult encode (host):          {:>4} calls, {:>19.0} ns/call",
            workload.result_encode.calls,
            workload.result_encode.mean_ns()
        );
        println!(
            "InvokeResult decode (host):          {:>4} calls, {:>19.0} ns/call",
            workload.result_decode.calls,
            workload.result_decode.mean_ns()
        );

        let phases = workload.engine_calls() * 4 + size_pass.count + call_info.count * 2 + returns.count;
        let overhead = phases as f64 * timer_ns;
        let serde = workload.serde_ns() as f64;
        println!();
        println!(
            "ABI serialisation: {:.1} µs of {:.1} µs wall time = {:.1}% (timer overhead in that figure: {:.1} µs, \
             {:.1}% of wall)",
            serde / 1000.0,
            workload.wall_ns as f64 / 1000.0,
            100.0 * serde / workload.wall_ns as f64,
            overhead / 1000.0,
            100.0 * overhead / workload.wall_ns as f64,
        );
    }

    // -- Part 3: what the boundary costs a real transaction -----------------------------------

    fn summary(fits: &Fits, censuses: &[WorkloadCensus], timer_ns: f64) {
        for workload in censuses {
            per_hop_table(fits, workload);
        }

        println!("\n\n== Summary ==");
        println!(
            "{:<26} {:>12} {:>12} {:>14} {:>12}",
            "workload", "host serde", "host total", "guest ABI pts", "of WASM pts"
        );
        println!("{}", "-".repeat(80));
        for workload in censuses {
            let phases = workload.engine_calls() * 4 +
                workload.census.call_info_size_pass().count +
                workload.census.call_info_encode().count * 2 +
                workload.census.return_decode().count;
            let serde = (workload.serde_ns() as f64 - phases as f64 * timer_ns).max(0.0);
            let guest = estimate_guest_points(fits, workload);
            println!(
                "{:<26} {:>10.1} µs {:>10.1} µs {:>14.0} {:>11.1}%",
                workload.name,
                serde / 1000.0,
                workload.wall_ns as f64 / 1000.0,
                guest,
                if workload.wasm_points == 0 {
                    0.0
                } else {
                    100.0 * guest / workload.wasm_points as f64
                },
            );
        }
        println!(
            "\nGuest ABI points are the probe fits applied to the census counts and byte sizes, not a direct \
             measurement.\nEvery engine call pays {:.0} points before any payload; a response that is not a bare unit \
             adds {:.0} more.\nHost wall time is the whole harness execution, so the boundary's share of a \
             validator's own work is higher than it reads here.\nA nested call's handler time contains the whole \
             sub-invocation, ABI hops included.",
            fits.bare_call, fits.response_decode,
        );
    }

    /// Guest points the boundary costs one op, from the probe fits: hop 3 for its argument, then
    /// hop 4 for its response. A response of one or two bytes is a bare `()`; anything larger
    /// arrives as an `InvokeResult` and pays its framing on top.
    fn op_points(fits: &Fits, arg_bytes: f64, resp_bytes: f64) -> f64 {
        let mut points = fits.bare_call + fits.hop3_arg.slope * arg_bytes;
        if resp_bytes > 2.0 {
            points += fits.response_decode + fits.state_get_typed.slope * resp_bytes;
        }
        points
    }

    fn estimate_guest_points(fits: &Fits, workload: &WorkloadCensus) -> f64 {
        let mut total = 0.0;
        for stats in workload.census.ops().values() {
            let calls = stats.calls as f64;
            total += op_points(fits, stats.arg_bytes as f64 / calls, stats.resp_bytes as f64 / calls) * calls;
        }
        // The hop 1 fit is the cost of a whole `sink_bytes` invocation, so it already carries the
        // dispatch framing and a scalar return; hop 2 adds only what a payload return costs above
        // that scalar.
        total += fits.hop1_arg.at(workload.census.call_info_encode().mean_bytes()) *
            workload.census.call_info_encode().count as f64;
        total += fits.hop2_return.at(workload.census.return_decode().mean_bytes()) *
            workload.census.return_decode().count as f64 -
            fits.hop2_return.intercept * workload.census.return_decode().count as f64;
        total
    }

    /// The plan's per-hop deliverable for one workload: crossings, bytes, host time, guest points.
    fn per_hop_table(fits: &Fits, workload: &WorkloadCensus) {
        let size_pass = workload.census.call_info_size_pass();
        let call_info = workload.census.call_info_encode();
        let alloc = workload.census.guest_alloc();
        let returns = workload.census.return_decode();
        let calls = workload.engine_calls() as f64;
        let per_call = |total: u64| if calls == 0.0 { 0.0 } else { total as f64 / calls };
        let sum = |field: fn(&abi_metrics::OpStats) -> u64| workload.census.ops().values().map(field).sum::<u64>();
        let arg_bytes = per_call(sum(|op| op.arg_bytes));
        let resp_bytes = per_call(sum(|op| op.resp_bytes));
        let decode_ns = per_call(sum(|op| op.decode_ns));
        let encode_ns = per_call(sum(|op| op.encode_ns));
        let result_encode_ns = per_call(workload.result_encode.ns);

        println!("\n== Per hop: {} ==", workload.name);
        println!(
            "{:<38} {:>10} {:>10} {:>12} {:>14}",
            "hop", "crossings", "bytes", "host ns", "guest points"
        );
        println!("{}", "-".repeat(88));
        let rows: [(&str, f64, f64, f64, f64); 4] = [
            (
                "1 host -> guest, call arguments",
                call_info.count as f64,
                call_info.mean_bytes(),
                size_pass.mean_ns() + call_info.mean_ns() - alloc.mean_ns(),
                fits.hop1_arg.at(call_info.mean_bytes()),
            ),
            (
                "2 guest -> host, return value",
                returns.count as f64,
                returns.mean_bytes(),
                returns.mean_ns(),
                fits.hop2_return.slope * returns.mean_bytes(),
            ),
            (
                "3 guest -> host, engine arguments",
                calls,
                arg_bytes,
                decode_ns,
                fits.bare_call + fits.hop3_arg.slope * arg_bytes,
            ),
            (
                "4 host -> guest, engine responses",
                calls,
                resp_bytes,
                encode_ns + result_encode_ns,
                op_points(fits, arg_bytes, resp_bytes) - fits.bare_call - fits.hop3_arg.slope * arg_bytes,
            ),
        ];
        for (label, crossings, bytes, ns, points) in rows {
            let points = if crossings == 0.0 { 0.0 } else { points };
            println!(
                "{:<38} {:>10.0} {:>10.0} {:>12.0} {:>14.0}",
                label, crossings, bytes, ns, points
            );
        }
        println!(
            "Totals: {:.1} µs host, {:.0} guest points of {} measured.",
            rows.iter().map(|r| r.1 * r.3).sum::<f64>() / 1000.0,
            rows.iter().map(|r| r.1 * r.4).sum::<f64>(),
            workload.wasm_points
        );
    }

    // -- Workload fixture ----------------------------------------------------------------------

    struct SwapFixture {
        a_resource: ResourceAddress,
        pool: ComponentAddress,
        account: ComponentAddress,
        account_proof: NonFungibleAddress,
        b_resource: ResourceAddress,
    }

    impl SwapFixture {
        fn setup(test: &mut TemplateTest) -> Self {
            let (a_faucet, a_resource) = faucet(test, "A".to_string());
            let (b_faucet, b_resource) = faucet(test, "B".to_string());
            let pool = pool(test, a_resource, b_resource);
            let (account, account_proof, _) = test.create_funded_account();
            fund(test, account, a_faucet);
            fund(test, account, b_faucet);

            let fixture = Self {
                a_resource,
                b_resource,
                pool,
                account,
                account_proof,
            };
            fixture.add_liquidity(test, Amount::from(400u32), Amount::from(400u32));
            fixture
        }

        fn add_liquidity(&self, test: &mut TemplateTest, a: Amount, b: Amount) {
            let owner = test.owner_proof();
            test.build_and_execute(
                Transaction::builder_localnet(Epoch(1))
                    .call_method(self.account, "withdraw", args![self.a_resource, a])
                    .put_last_instruction_output_on_workspace("a_bucket")
                    .call_method(self.account, "withdraw", args![self.b_resource, b])
                    .put_last_instruction_output_on_workspace("b_bucket")
                    .call_method(self.pool, "add_liquidity", args![
                        Workspace("a_bucket"),
                        Workspace("b_bucket")
                    ])
                    .put_last_instruction_output_on_workspace("lp_bucket")
                    .call_method(self.account, "deposit", args![Workspace("lp_bucket")]),
                vec![self.account_proof.clone(), owner],
            )
            .expect_success();
        }

        fn swap(&self, test: &mut TemplateTest, amount: Amount) {
            test.build_and_execute(
                Transaction::builder_localnet(Epoch(1))
                    .call_method(self.account, "withdraw", args![self.a_resource, amount])
                    .put_last_instruction_output_on_workspace("input_bucket")
                    .call_method(self.pool, "swap", args![Workspace("input_bucket"), self.b_resource])
                    .put_last_instruction_output_on_workspace("output_bucket")
                    .call_method(self.account, "deposit", args![Workspace("output_bucket")]),
                vec![self.account_proof.clone()],
            )
            .expect_success();
        }
    }

    fn faucet(test: &mut TemplateTest, symbol: String) -> (ComponentAddress, ResourceAddress) {
        let component: ComponentAddress = test.call_function(
            "TestFaucet",
            "mint_with_symbol",
            args![Amount::from(1_000_000_000_000u64), symbol],
            vec![],
        );
        let resource = test
            .get_previous_output_address(SubstateType::Resource)
            .as_resource_address()
            .unwrap();
        (component, resource)
    }

    fn pool(test: &mut TemplateTest, a: ResourceAddress, b: ResourceAddress) -> ComponentAddress {
        let template = test.get_template_address("TariSwapPool");
        let result = test.execute_expect_success(
            test.transaction()
                .call_function(template, "new", args![a, b, 1u16])
                .build_and_seal(test.secret_key()),
            vec![],
        );
        let (address, _) = result
            .expect_success()
            .up_iter()
            .find(|(address, substate)| {
                address.is_component() && *substate.substate_value().component().unwrap().template_address() == template
            })
            .unwrap();
        address.as_component_address().unwrap()
    }

    fn fund(test: &mut TemplateTest, account: ComponentAddress, faucet: ComponentAddress) {
        test.build_and_execute(
            Transaction::builder_localnet(Epoch(1))
                .call_method(faucet, "take_free_coins", args![])
                .put_last_instruction_output_on_workspace("free_coins")
                .call_method(account, "deposit", args![Workspace("free_coins")]),
            vec![],
        )
        .expect_success();
    }
}
