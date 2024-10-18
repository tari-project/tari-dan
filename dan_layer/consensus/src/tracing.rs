//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Instant;

use log::{log, warn};

pub struct TraceTimer {
    start: Instant,
    log_target: &'static str,
    context: &'static str,
    level: log::Level,
    iterations: Option<usize>,
    excessive_threshold: u128,
}

impl TraceTimer {
    pub fn new(log_target: &'static str, context: &'static str, level: log::Level) -> Self {
        Self {
            start: Instant::now(),
            log_target,
            context,
            level,
            iterations: None,
            excessive_threshold: 1000,
        }
    }

    pub fn with_excessive_threshold(mut self, excessive_threshold: u128) -> Self {
        self.excessive_threshold = excessive_threshold;
        self
    }

    pub fn with_iterations(mut self, iterations: usize) -> Self {
        self.iterations = Some(iterations);
        self
    }

    pub fn info(log_target: &'static str, context: &'static str) -> Self {
        Self::new(log_target, context, log::Level::Info)
    }

    pub fn debug(log_target: &'static str, context: &'static str) -> Self {
        Self::new(log_target, context, log::Level::Debug)
    }

    pub fn done(self) {
        // Drop
    }
}

impl Drop for TraceTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        if elapsed.as_millis() >= self.excessive_threshold {
            if let Some(iterations) = self.iterations {
                let avg = elapsed.as_millis() as f64 / iterations as f64;
                warn!(target: self.log_target, "⏲️ EXCESSIVE: {} took {:.2?} for {} iterations (avg: {:.0?}ms)", self.context, elapsed, iterations, avg);
            } else {
                warn!(target: self.log_target, "⏲️ EXCESSIVE: {} took {:.2?}", self.context, elapsed);
            }
        } else if let Some(iterations) = self.iterations {
            let avg = elapsed.as_millis() as f64 / iterations as f64;
            log!(target: self.log_target, self.level, "⏲️ {} took {:.2?} for {} iterations (avg: {:.0?}ms)", self.context, elapsed, iterations, avg);
        } else {
            log!(target: self.log_target, self.level, "⏲️ {} took {:.2?}", self.context, elapsed);
        }
    }
}
