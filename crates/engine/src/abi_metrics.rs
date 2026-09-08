//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Off-by-default instrumentation of the WASM ABI boundary, for the measurement described in
//! `crates/engine/examples/abi_measure.rs`.
//!
//! Compiled out unless the `abi-metrics` feature is on: with the feature off every entry point is
//! an empty inline function and [`Span`] is a zero-sized token, so nothing is timed, allocated or
//! branched on. With it on, the counters live in a thread-local, matching the engine's
//! single-threaded execution of one transaction.
//!
//! Timing a phase costs two `Instant::now()` reads. That overhead is not subtracted here — the
//! reader calibrates it (see `timer_overhead_ns`) and accounts for it against the phase counts.

pub use imp::*;

/// One engine call's measurements, handed to [`record_engine_op`] once the call completes.
///
/// The fields are populated whether or not the feature is on, so the call sites in
/// `wasm::process` read the same either way; with it off, nothing reads them back.
#[derive(Clone, Copy, Default)]
#[cfg_attr(not(feature = "abi-metrics"), allow(dead_code))]
pub struct OpSample {
    /// Encoded argument bytes read out of linear memory (hop 3 wire size).
    pub arg_bytes: usize,
    /// Encoded response bytes written into linear memory (hop 4 wire size).
    pub resp_bytes: usize,
    /// Decoding the argument.
    pub decode_ns: u64,
    /// The runtime handler itself: everything that is not ABI serialisation.
    pub handler_ns: u64,
    /// `encoded_len` plus `encode_into_writer` for the response.
    pub encode_ns: u64,
    /// The guest-side `tari_alloc` call that reserves room for the response.
    pub alloc_ns: u64,
}

#[cfg(not(feature = "abi-metrics"))]
mod imp {
    use tari_template_abi::EngineOp;

    use super::OpSample;

    /// A timing span. Without the `abi-metrics` feature it holds nothing and measures nothing.
    #[derive(Clone, Copy)]
    pub struct Span;

    impl Span {
        #[inline]
        pub fn start() -> Self {
            Span
        }

        #[inline]
        pub fn finish(self) -> u64 {
            0
        }
    }

    #[inline]
    pub fn record_engine_op(_op: EngineOp, _sample: OpSample) {}

    #[inline]
    pub fn record_call_info_size_pass(_bytes: usize, _ns: u64) {}

    #[inline]
    pub fn record_call_info_encode(_bytes: usize, _ns: u64) {}

    #[inline]
    pub fn record_guest_alloc(_bytes: usize, _ns: u64) {}

    #[inline]
    pub fn record_return_decode(_bytes: usize, _ns: u64) {}
}

#[cfg(feature = "abi-metrics")]
mod imp {
    use std::{cell::RefCell, collections::BTreeMap, time::Instant};

    use tari_template_abi::EngineOp;

    use super::OpSample;

    thread_local! {
        static CENSUS: RefCell<Census> = RefCell::new(Census::default());
    }

    /// A timing span over one ABI phase.
    #[derive(Clone, Copy)]
    pub struct Span(Instant);

    impl Span {
        #[inline]
        pub fn start() -> Self {
            Span(Instant::now())
        }

        /// Nanoseconds elapsed since [`Self::start`], including the cost of the two clock reads.
        #[inline]
        pub fn finish(self) -> u64 {
            self.0.elapsed().as_nanos() as u64
        }
    }

    pub fn record_engine_op(op: EngineOp, sample: OpSample) {
        CENSUS.with_borrow_mut(|census| census.ops.entry(op.as_i32()).or_default().add(&sample));
    }

    pub fn record_call_info_size_pass(bytes: usize, ns: u64) {
        CENSUS.with_borrow_mut(|census| census.call_info_size_pass.add(bytes, ns));
    }

    pub fn record_call_info_encode(bytes: usize, ns: u64) {
        CENSUS.with_borrow_mut(|census| census.call_info_encode.add(bytes, ns));
    }

    pub fn record_guest_alloc(bytes: usize, ns: u64) {
        CENSUS.with_borrow_mut(|census| census.guest_alloc.add(bytes, ns));
    }

    pub fn record_return_decode(bytes: usize, ns: u64) {
        CENSUS.with_borrow_mut(|census| census.return_decode.add(bytes, ns));
    }

    /// Clears every counter on the calling thread.
    pub fn reset() {
        CENSUS.with_borrow_mut(|census| *census = Census::default());
    }

    /// The counters accumulated on the calling thread since the last [`reset`].
    pub fn snapshot() -> Census {
        CENSUS.with_borrow(Clone::clone)
    }

    /// Measured cost of one [`Span`] (two clock reads), for accounting the instrumentation's own
    /// overhead against a phase's call count.
    pub fn timer_overhead_ns() -> f64 {
        const ROUNDS: u32 = 200_000;
        // One untimed pass warms the clock's vDSO path so the measured pass is steady-state.
        for _ in 0..ROUNDS {
            std::hint::black_box(Span::start().finish());
        }
        let start = Instant::now();
        for _ in 0..ROUNDS {
            std::hint::black_box(Span::start().finish());
        }
        start.elapsed().as_nanos() as f64 / f64::from(ROUNDS)
    }

    /// Totals for one ABI phase.
    #[derive(Clone, Copy, Default, Debug)]
    pub struct PhaseStats {
        pub count: u64,
        pub bytes: u64,
        pub ns: u64,
    }

    impl PhaseStats {
        fn add(&mut self, bytes: usize, ns: u64) {
            self.count += 1;
            self.bytes += bytes as u64;
            self.ns += ns;
        }

        pub fn mean_ns(&self) -> f64 {
            if self.count == 0 {
                return 0.0;
            }
            self.ns as f64 / self.count as f64
        }

        pub fn mean_bytes(&self) -> f64 {
            if self.count == 0 {
                return 0.0;
            }
            self.bytes as f64 / self.count as f64
        }
    }

    /// Per-op totals across every call of one engine op.
    #[derive(Clone, Copy, Default, Debug)]
    pub struct OpStats {
        pub calls: u64,
        pub arg_bytes: u64,
        pub resp_bytes: u64,
        pub max_arg_bytes: usize,
        pub max_resp_bytes: usize,
        pub decode_ns: u64,
        pub handler_ns: u64,
        pub encode_ns: u64,
        pub alloc_ns: u64,
    }

    impl OpStats {
        fn add(&mut self, sample: &OpSample) {
            self.calls += 1;
            self.arg_bytes += sample.arg_bytes as u64;
            self.resp_bytes += sample.resp_bytes as u64;
            self.max_arg_bytes = self.max_arg_bytes.max(sample.arg_bytes);
            self.max_resp_bytes = self.max_resp_bytes.max(sample.resp_bytes);
            self.decode_ns += sample.decode_ns;
            self.handler_ns += sample.handler_ns;
            self.encode_ns += sample.encode_ns;
            self.alloc_ns += sample.alloc_ns;
        }

        /// Every nanosecond spent on serialisation, i.e. everything but the handler.
        pub fn serde_ns(&self) -> u64 {
            self.decode_ns + self.encode_ns
        }
    }

    /// Every counter accumulated on one thread.
    #[derive(Clone, Default, Debug)]
    pub struct Census {
        ops: BTreeMap<i32, OpStats>,
        /// Hop 1, host side: the `encoded_len` pre-pass over a `CallInfo`.
        call_info_size_pass: PhaseStats,
        /// Hop 1, host side: allocating guest memory for a `CallInfo` and encoding into it.
        call_info_encode: PhaseStats,
        /// The `tari_alloc` call inside [`Self::call_info_encode`].
        guest_alloc: PhaseStats,
        /// Hop 2, host side: `IndexedValue::from_raw` over a template's return value.
        return_decode: PhaseStats,
    }

    impl Census {
        pub fn ops(&self) -> &BTreeMap<i32, OpStats> {
            &self.ops
        }

        pub fn call_info_size_pass(&self) -> &PhaseStats {
            &self.call_info_size_pass
        }

        pub fn call_info_encode(&self) -> &PhaseStats {
            &self.call_info_encode
        }

        /// The `tari_alloc` call that reserves guest memory for a `CallInfo`, counted inside
        /// [`Self::call_info_encode`]'s time: it enters WASM, so it is not serialisation.
        pub fn guest_alloc(&self) -> &PhaseStats {
            &self.guest_alloc
        }

        pub fn return_decode(&self) -> &PhaseStats {
            &self.return_decode
        }
    }
}
