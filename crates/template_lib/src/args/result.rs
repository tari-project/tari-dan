//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use tari_bor::{BorError, from_value, to_value};
use tari_template_abi::rust::prelude::*;

/// The result of an instruction invocation, which is either the CBOR encoded result value or a `String` with an error
/// message
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InvokeResult(#[n(0)] Result<tari_bor::Value, String>);

impl InvokeResult {
    pub fn from_value(value: tari_bor::Value) -> Self {
        Self(Ok(value))
    }

    pub fn encode<T: Encode<()> + ?Sized>(output: &T) -> Result<Self, BorError> {
        let span = metrics::Span::start();
        let value = to_value(output)?;
        metrics::record_to_value(span);
        Ok(Self(Ok(value)))
    }

    pub fn decode<T: for<'b> Decode<'b, ()>>(self) -> Result<T, BorError> {
        match self.0 {
            Ok(output) => {
                let span = metrics::Span::start();
                let decoded = from_value(&output);
                metrics::record_from_value(span);
                decoded
            },
            Err(err) => Err(BorError::new(err)),
        }
    }

    pub fn into_value(self) -> Result<tari_bor::Value, BorError> {
        self.0.map_err(BorError::new)
    }

    pub fn unit() -> Self {
        Self(Ok(tari_bor::Value::Array(Vec::new())))
    }
}

/// Timing of the `tari_bor::Value` conversions on either side of an [`InvokeResult`], compiled out
/// unless the `metrics` feature is on. See `tari_engine::abi_metrics`.
#[cfg(not(feature = "metrics"))]
pub mod metrics {
    /// A timing span. Without the `metrics` feature it holds nothing and measures nothing.
    #[derive(Clone, Copy)]
    pub struct Span;

    impl Span {
        #[inline]
        pub fn start() -> Self {
            Span
        }
    }

    #[inline]
    pub fn record_to_value(_span: Span) {}

    #[inline]
    pub fn record_from_value(_span: Span) {}
}

#[cfg(feature = "metrics")]
pub mod metrics {
    use std::{cell::Cell, time::Instant};

    thread_local! {
        static TO_VALUE: Cell<Phase> = const { Cell::new(Phase::ZERO) };
        static FROM_VALUE: Cell<Phase> = const { Cell::new(Phase::ZERO) };
    }

    /// Calls and elapsed nanoseconds of one conversion direction.
    #[derive(Clone, Copy, Default, Debug)]
    pub struct Phase {
        pub calls: u64,
        pub ns: u64,
    }

    impl Phase {
        const ZERO: Self = Self { calls: 0, ns: 0 };

        pub fn mean_ns(&self) -> f64 {
            if self.calls == 0 {
                return 0.0;
            }
            self.ns as f64 / self.calls as f64
        }
    }

    /// A timing span over one conversion.
    #[derive(Clone, Copy)]
    pub struct Span(Instant);

    impl Span {
        #[inline]
        pub fn start() -> Self {
            Span(Instant::now())
        }
    }

    pub fn record_to_value(span: Span) {
        add(&TO_VALUE, span);
    }

    pub fn record_from_value(span: Span) {
        add(&FROM_VALUE, span);
    }

    fn add(cell: &'static std::thread::LocalKey<Cell<Phase>>, span: Span) {
        let ns = span.0.elapsed().as_nanos() as u64;
        cell.with(|c| {
            let mut phase = c.get();
            phase.calls += 1;
            phase.ns += ns;
            c.set(phase);
        });
    }

    /// Clears both counters on the calling thread.
    pub fn reset() {
        TO_VALUE.with(|c| c.set(Phase::ZERO));
        FROM_VALUE.with(|c| c.set(Phase::ZERO));
    }

    /// Host-side `to_value` (building an `InvokeResult`) since the last [`reset`].
    pub fn to_value() -> Phase {
        TO_VALUE.with(|c| c.get())
    }

    /// Host-side `from_value` (reading an `InvokeResult`) since the last [`reset`].
    pub fn from_value() -> Phase {
        FROM_VALUE.with(|c| c.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_decode() {
        from_value::<()>(&InvokeResult::unit().0.unwrap()).unwrap();
    }
}
