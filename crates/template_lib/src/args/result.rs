//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use tari_bor::{BorError, RawCbor};
use tari_template_abi::rust::prelude::*;

/// The result of an engine call: either the CBOR encoding of the returned value, or a `String` with
/// an error message.
///
/// A published template carries its own compiled-in decoder for this type, so the bytes must stay
/// as they are: `#[n(0)] Result<RawCbor, String>` and `#[n(0)] Result<tari_bor::Value, String>`
/// encode a given response identically, and the tests below hold that.
#[derive(Clone, Debug, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InvokeResult(#[n(0)] Result<RawCbor, String>);

impl InvokeResult {
    pub fn from_value(value: tari_bor::Value) -> Result<Self, BorError> {
        Ok(Self(Ok(RawCbor::from_value(&value)?)))
    }

    pub fn encode<T: Encode<()> + ?Sized>(output: &T) -> Result<Self, BorError> {
        let span = metrics::Span::start();
        let raw = RawCbor::from_encodable(output)?;
        metrics::record_encode(span);
        Ok(Self(Ok(raw)))
    }

    pub fn decode<T: for<'b> Decode<'b, ()>>(self) -> Result<T, BorError> {
        match self.0 {
            Ok(output) => {
                let span = metrics::Span::start();
                let decoded = output.decode();
                metrics::record_decode(span);
                decoded
            },
            Err(err) => Err(BorError::new(err)),
        }
    }

    pub fn into_value(self) -> Result<tari_bor::Value, BorError> {
        self.0.map_err(BorError::new)?.to_value()
    }

    pub fn unit() -> Self {
        Self(Ok(RawCbor::from_encodable(&()).expect("unit always encodes")))
    }
}

/// Timing of the encode and decode on either side of an [`InvokeResult`], compiled out unless the
/// `metrics` feature is on. See `tari_engine::abi_metrics`.
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
    pub fn record_encode(_span: Span) {}

    #[inline]
    pub fn record_decode(_span: Span) {}
}

#[cfg(feature = "metrics")]
pub mod metrics {
    use std::{cell::Cell, time::Instant};

    thread_local! {
        static ENCODE: Cell<Phase> = const { Cell::new(Phase::ZERO) };
        static DECODE: Cell<Phase> = const { Cell::new(Phase::ZERO) };
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

    pub fn record_encode(span: Span) {
        add(&ENCODE, span);
    }

    pub fn record_decode(span: Span) {
        add(&DECODE, span);
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
        ENCODE.with(|c| c.set(Phase::ZERO));
        DECODE.with(|c| c.set(Phase::ZERO));
    }

    /// Building an `InvokeResult` from a response value, since the last [`reset`].
    pub fn encode() -> Phase {
        ENCODE.with(|c| c.get())
    }

    /// Reading a response out of an `InvokeResult`, since the last [`reset`].
    pub fn decode() -> Phase {
        DECODE.with(|c| c.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_decode() {
        InvokeResult::unit().decode::<()>().unwrap();
    }

    /// The shape a published template decodes engine responses with. What the engine writes must
    /// encode identically to this.
    #[derive(Encode, Decode, CborLen)]
    struct ValueBackedInvokeResult(#[n(0)] Result<tari_bor::Value, String>);

    fn assert_same_wire_form(value: tari_bor::Value) {
        let legacy = tari_bor::encode(&ValueBackedInvokeResult(Ok(value.clone()))).unwrap();
        let current = tari_bor::encode(&InvokeResult::from_value(value).unwrap()).unwrap();
        assert_eq!(current, legacy);
    }

    #[test]
    fn responses_keep_the_wire_form_published_templates_decode() {
        assert_same_wire_form(tari_bor::Value::Array(Vec::new()));
        assert_same_wire_form(tari_bor::Value::Null);
        assert_same_wire_form(tari_bor::Value::Bool(true));
        assert_same_wire_form(tari_bor::Value::Integer(i128::from(u64::MAX)));
        assert_same_wire_form(tari_bor::Value::Integer(i128::from(i64::MIN)));
        assert_same_wire_form(tari_bor::Value::Bytes(vec![0x5A; 300]));
        assert_same_wire_form(tari_bor::Value::Text("a string".to_string()));
        assert_same_wire_form(tari_bor::Value::Array(vec![
            tari_bor::Value::Integer(1),
            tari_bor::Value::Text("two".to_string()),
        ]));
        assert_same_wire_form(tari_bor::Value::Map(vec![(
            tari_bor::Value::Text("key".to_string()),
            tari_bor::Value::Bytes(vec![1, 2, 3]),
        )]));
        assert_same_wire_form(tari_bor::Value::Tag(42, Box::new(tari_bor::Value::Integer(7))));
    }

    #[test]
    fn errors_keep_the_wire_form_published_templates_decode() {
        let legacy = tari_bor::encode(&ValueBackedInvokeResult(Err("boom".to_string()))).unwrap();
        let current = tari_bor::encode(&InvokeResult(Err("boom".to_string()))).unwrap();
        assert_eq!(current, legacy);
    }

    /// A unit response encodes as the empty array a published template decodes.
    #[test]
    fn unit_keeps_its_wire_form() {
        let legacy = tari_bor::encode(&tari_bor::Value::Array(Vec::new())).unwrap();
        assert_eq!(InvokeResult::unit().0.unwrap().as_bytes(), legacy.as_slice());
    }

    /// A typed response and the same response routed through a `Value` put the same bytes on the
    /// wire.
    #[test]
    fn typed_and_value_responses_agree() {
        let typed = tari_bor::encode(&InvokeResult::encode(&(1u32, "two", vec![3u8])).unwrap()).unwrap();
        let value = tari_bor::encode(
            &InvokeResult::from_value(tari_bor::to_value(&(1u32, "two", vec![3u8])).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(typed, value);
    }
}
