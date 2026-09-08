//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Microbenchmark template for pricing the WASM ABI boundary in guest metering points, driven by
//! `crates/engine/examples/abi_measure.rs`.
//!
//! Four hops cross the boundary: host-to-guest call arguments (1), guest-to-host return values (2),
//! guest-to-host engine-call arguments (3) and host-to-guest engine-call responses (4). Each probe
//! below isolates one of them, and each has a `*_glue` twin that performs the identical work minus
//! the boundary crossing. Subtracting the twin leaves the hop's own cost, the way
//! `metering_bench` isolates a single WASM operator. Probes that take a count `n` are read as a
//! slope across two values of `n`, which also cancels the fixed dispatch cost of the call itself.

use tari_template_abi::{EngineOp, call_engine};
use tari_template_lib::{
    args::{
        CallerContextAction,
        CallerContextInvokeArg,
        ComponentAction,
        ComponentInvokeArg,
        ComponentRef,
        InvokeResult,
    },
    prelude::*,
};

/// Bytes of log message per `emit_logs` iteration must stay under
/// `ENGINE_LIMITS.max_log_size_bytes`, and the iteration count under `max_logs`.
const LOG_LEVEL: tari_template_lib::types::LogLevel = tari_template_lib::types::LogLevel::Debug;

#[template]
mod abi_probe {
    use super::*;

    pub struct AbiProbe {
        blob: Bytes,
    }

    impl AbiProbe {
        /// A component whose state encodes to roughly `state_bytes`, so state round-trips can be
        /// priced against payload size.
        pub fn new(state_bytes: u32) -> Component<Self> {
            Component::new(Self {
                blob: Bytes::from_vec(vec![0x5A; state_bytes as usize]),
            })
            .with_access_rules(AccessRules::new().default(rule!(allow_all)))
            .create()
        }

        // Hop 0: dispatch ------------------------------------------------------------------
        /// Baseline: enter the dispatcher, decode a header, encode a unit return.
        pub fn noop() {}

        // Metering reference ---------------------------------------------------------------
        /// A dependent integer chain: work whose metering points track real execution time closely,
        /// as the reference the copy loop below is compared against.
        pub fn spin(rounds: u32) -> u64 {
            let mut acc: u64 = 0x9E37_79B9_7F4A_7C17;
            for _ in 0..rounds {
                acc = (acc ^ (acc >> 7)).wrapping_mul(0xD1B5_4A32_D192_ED03).rotate_left(13);
            }
            acc
        }

        /// `rounds` copies of `size` bytes. `copy_from_slice` lowers to a single `memory.copy`,
        /// which metering charges a flat rate regardless of length.
        pub fn bulk_copy(rounds: u32, size: u32) -> u32 {
            let src = vec![0x5Au8; size as usize];
            let mut dst = vec![0u8; size as usize];
            let mut acc = 0u32;
            for _ in 0..rounds {
                dst.copy_from_slice(&src);
                acc = acc.wrapping_add(u32::from(core::hint::black_box(&dst)[0]));
            }
            acc
        }

        // Buffer strategy for the hop 3 encode ----------------------------------------------
        /// `n` encodes of a realistic `SetState` engine-call argument carrying a `size`-byte
        /// payload, buffered one of four ways. Nothing but the encode's own buffer differs, so the
        /// slopes across `n` price the `encoded_len` pre-pass against the alternatives.
        ///
        /// 0: `encoded_len` pre-pass, then an exactly sized buffer, which is what `call_engine`
        /// does today. 1: an empty buffer left to grow. 2 and 3: one 512- or 1024-byte allocation
        /// up front, growing only for an argument larger than that.
        pub fn encode_buffered(&self, n: u32, size: u32, strategy: u32) -> u32 {
            let arg = ComponentInvokeArg {
                component_ref: ComponentRef::Ref(CallerContext::current_component_address()),
                action: ComponentAction::SetState,
                args: vec![Bytes::from_vec(vec![0x5A; size as usize])],
            };
            let mut acc = 0u32;
            // The branch sits outside the loop so every strategy runs identical loop glue.
            match strategy {
                0 => {
                    for _ in 0..n {
                        let len = tari_bor::encoded_len(&arg).unwrap();
                        let mut buf = Vec::with_capacity(len);
                        tari_bor::encode_into_writer(&arg, &mut buf).unwrap();
                        acc = acc.wrapping_add(core::hint::black_box(&buf).len() as u32);
                    }
                },
                1 => {
                    for _ in 0..n {
                        let mut buf = Vec::new();
                        tari_bor::encode_into_writer(&arg, &mut buf).unwrap();
                        acc = acc.wrapping_add(core::hint::black_box(&buf).len() as u32);
                    }
                },
                2 => {
                    for _ in 0..n {
                        let mut buf = Vec::with_capacity(512);
                        tari_bor::encode_into_writer(&arg, &mut buf).unwrap();
                        acc = acc.wrapping_add(core::hint::black_box(&buf).len() as u32);
                    }
                },
                _ => {
                    for _ in 0..n {
                        let mut buf = Vec::with_capacity(1024);
                        tari_bor::encode_into_writer(&arg, &mut buf).unwrap();
                        acc = acc.wrapping_add(core::hint::black_box(&buf).len() as u32);
                    }
                },
            }
            acc
        }

        // Hop 1: call argument decode ------------------------------------------------------
        /// Decodes `v` and drops it. Return is unit, so only hop 1 scales with size. The payload is
        /// a CBOR byte string (`Bytes`), the compact encoding a real payload uses; a `Vec<u8>`
        /// would arrive as an array of integers and price a different path.
        pub fn sink_bytes(v: Bytes) -> u32 {
            v.len() as u32
        }

        // Hop 2: return value encode -------------------------------------------------------
        /// Builds `n` bytes and returns them: hop 2 plus the allocation.
        pub fn make_bytes(n: u32) -> Bytes {
            Bytes::from_vec(vec![0x5A; n as usize])
        }

        /// `make_bytes` without the return crossing: the allocation alone.
        pub fn make_bytes_glue(n: u32) -> u32 {
            core::hint::black_box(Bytes::from_vec(vec![0x5A; n as usize])).len() as u32
        }

        // Hop 3: engine call argument encode ------------------------------------------------
        /// `n` `EmitLog` calls carrying a `size`-byte message. The response is unit, so cost
        /// scales with the argument only. Bounded by `max_logs` and `max_log_size_bytes`.
        pub fn emit_logs(n: u32, size: u32) -> u32 {
            let msg = message(size);
            let mut acc = 0u32;
            for _ in 0..n {
                let m = msg.clone();
                acc = acc.wrapping_add(m.len() as u32);
                engine().emit_log(LOG_LEVEL, m);
            }
            acc
        }

        /// `emit_logs` without the engine call: the message clone alone.
        pub fn emit_logs_glue(n: u32, size: u32) -> u32 {
            let msg = message(size);
            let mut acc = 0u32;
            for _ in 0..n {
                let m = core::hint::black_box(msg.clone());
                acc = acc.wrapping_add(m.len() as u32);
            }
            acc
        }

        // Hops 3 and 4: fixed per-call cost --------------------------------------------------
        /// `n` `CallerContextInvoke` calls. Both payloads are a handful of bytes, so this is the
        /// per-engine-call floor: envelope encode, response decode, `tari_alloc`, `tari_free`.
        pub fn caller_context_calls(n: u32) -> u32 {
            let mut acc = 0u32;
            for _ in 0..n {
                let pk = CallerContext::transaction_signer_public_key();
                acc = acc.wrapping_add(u32::from(pk.as_bytes()[0]));
            }
            acc
        }

        /// `n` `CallerContextInvoke` calls stopping at the response `Value`. The difference
        /// against `caller_context_calls` is the `InvokeResult` `from_value` conversion into a
        /// concrete type, which no change of codec removes.
        pub fn caller_context_raw(n: u32) -> u32 {
            let mut acc = 0u32;
            for _ in 0..n {
                let result: InvokeResult = call_engine(EngineOp::CallerContextInvoke, &CallerContextInvokeArg {
                    action: CallerContextAction::GetCallerPublicKey,
                    args: args![],
                });
                let value = core::hint::black_box(result.into_value().unwrap());
                acc = acc.wrapping_add(u32::from(!matches!(value, tari_bor::Value::Null)));
            }
            acc
        }

        // Hops 3 and 4 with a payload: component state ---------------------------------------
        /// `n` `GetState` round-trips decoded into the concrete type: hop 4 plus the
        /// `InvokeResult` `from_value` conversion.
        pub fn get_state_typed(&self, n: u32) -> u32 {
            let manager = engine().component_manager(CallerContext::current_component_address());
            let mut acc = 0u32;
            for _ in 0..n {
                let state = manager.get_state::<Self>();
                acc = acc.wrapping_add(state.blob.len() as u32);
            }
            acc
        }

        /// `n` `GetState` round-trips stopping at the `InvokeResult`'s own `Value`: hop 4 without
        /// the `from_value` conversion into a concrete type. The difference against
        /// `get_state_typed` is that conversion, which no change of codec removes.
        pub fn get_state_raw(&self, n: u32) -> u32 {
            let component = CallerContext::current_component_address();
            let mut acc = 0u32;
            for _ in 0..n {
                let result: InvokeResult = call_engine(EngineOp::ComponentInvoke, &ComponentInvokeArg {
                    component_ref: ComponentRef::Ref(component),
                    action: ComponentAction::GetState,
                    args: args![],
                });
                let value = core::hint::black_box(result.into_value().unwrap());
                acc = acc.wrapping_add(u32::from(!matches!(value, tari_bor::Value::Null)));
            }
            acc
        }

        /// `n` `SetState` calls: hop 3 with the state as payload.
        pub fn set_state(&mut self, n: u32) -> u32 {
            let manager = engine().component_manager(CallerContext::current_component_address());
            let mut acc = 0u32;
            for _ in 0..n {
                manager.set_state(Self {
                    blob: self.blob.clone(),
                });
                acc = acc.wrapping_add(1);
            }
            acc
        }

        /// `set_state` without the engine call: the state clone alone.
        pub fn set_state_glue(&mut self, n: u32) -> u32 {
            let mut acc = 0u32;
            for _ in 0..n {
                let state = core::hint::black_box(Self {
                    blob: self.blob.clone(),
                });
                acc = acc.wrapping_add(state.blob.len() as u32);
            }
            acc
        }
    }
}

/// A `size`-byte ASCII message. Built once per probe call so the per-iteration work is a clone.
fn message(size: u32) -> String {
    core::iter::repeat_n('x', size as usize).collect()
}
