//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Compute-bound template for `tari-vn-bench`.
//!
//! The benchmark needs to know how fast this machine turns metering points into wall-clock time,
//! because the block execution-point budgets are denominated in points. Builtin templates cannot
//! answer that: their methods do a fixed, small amount of WASM work wrapped in a comparatively large
//! amount of engine work (substate loads, auth checks, invoke setup), so timing them measures engine
//! overhead, not execution. Fitting a slope across *instruction counts* does not help either — that
//! overhead is per-instruction, so it never cancels.
//!
//! This template moves the variable into the WASM itself. `grind` takes a round count, so two calls
//! that differ only in `rounds` differ only in executed WASM, and their slope is the machine's real
//! execution rate with every fixed and per-call cost removed.
//!
//! The loop is a serial dependent chain: each step's input is the previous step's output, so it
//! cannot be pipelined away and the time measured is the chain's true latency — the worst case a
//! compute-heavy transaction can realise, which is the conservative figure to size against.

use tari_template_lib::prelude::*;

/// Operations per outer round. A constant bound lets the optimiser unroll the inner loop, so the
/// outer-loop bookkeeping is amortised and the per-step time is dominated by the chain itself.
const INNER: u32 = 64;

const SEED: u64 = 0xD1B5_4A32_D192_ED03;

/// Odd 64-bit multiplier. Multiplying by it is a bijection mod 2^64, so the accumulator stays spread
/// across all 64 bits and the chain cannot collapse onto a trivial path.
const MIX: u64 = 0x9E37_79B9_7F4A_7C17;

#[template]
mod compute_bench {
    use super::*;

    pub struct ComputeBench {}

    impl ComputeBench {
        /// Runs `rounds` x `INNER` steps of a dependent arithmetic chain and returns the
        /// accumulator, which the caller discards. The return value exists only so the chain cannot
        /// be eliminated as dead code.
        pub fn grind(rounds: u64) -> u64 {
            let mut acc: u64 = SEED;
            let mut i: u64 = 0;
            while i < rounds {
                let mut j: u32 = 0;
                while j < INNER {
                    let d = (acc & 0xFFFF) | 1;
                    // A mix of cheap and expensive ops, so the rate reflects realistic template
                    // code rather than the best or worst single operator.
                    let v = acc.wrapping_mul(d) ^ (acc / d);
                    acc = (v ^ d).rotate_left(1).wrapping_mul(MIX);
                    j = j.wrapping_add(1);
                }
                i = i.wrapping_add(1);
            }
            acc
        }
    }
}
