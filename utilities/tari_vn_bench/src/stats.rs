//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Summary statistics for a set of timings.
//!
//! Every phase reports the same five numbers so that a fast result and a slow-but-erratic one are
//! distinguishable. That distinction is the point: a validator misses a proposal on its bad views,
//! not its average ones, so a host whose tail is far from its minimum fails differently from one
//! that is uniformly slow, and the report has to be able to say which.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Sample {
    /// The fastest observation. Scheduling noise, interrupts and frequency ramping only ever add
    /// time, so the minimum is the closest estimate of what the machine can actually do — this is
    /// the estimator the throughput figures are derived from.
    pub min_ms: f64,
    pub p50_ms: f64,
    /// The tail the pacemaker actually sees. A host passes on `min_ms` and still misses proposals
    /// if this is far above it.
    pub p99_ms: f64,
    pub max_ms: f64,
    pub samples: usize,
}

impl Sample {
    /// Consumes the timings and summarises them. Panics on an empty set, which is a programming
    /// error: every phase decides its own sample count and none of them may choose zero.
    pub fn from_millis(mut ms: Vec<f64>) -> Self {
        assert!(!ms.is_empty(), "cannot summarise an empty sample set");
        ms.sort_by(f64::total_cmp);
        Self {
            min_ms: ms[0],
            p50_ms: quantile(&ms, 0.50),
            p99_ms: quantile(&ms, 0.99),
            max_ms: ms[ms.len() - 1],
            samples: ms.len(),
        }
    }

    /// Spread of the median above the minimum, as a fraction. A large value means the run was
    /// contaminated by other load — on an idle machine the two sit close together — and is
    /// surfaced so a noisy run is not silently reported as a slow machine.
    pub fn dispersion(&self) -> f64 {
        if self.min_ms == 0.0 {
            return 0.0;
        }
        (self.p50_ms - self.min_ms) / self.min_ms
    }
}

/// Nearest-rank quantile over an already-sorted slice.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() as f64 * q).ceil() as usize).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}
