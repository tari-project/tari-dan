//  Copyright 2022. The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};

use crate::{
    rust::{boxed::Box, string::String, vec::Vec},
    version::WasmAbiVersion,
};

#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum TemplateDef {
    #[n(0)]
    V1(#[n(0)] TemplateDefV1),
}

impl TemplateDef {
    pub fn template_name(&self) -> &str {
        match self {
            TemplateDef::V1(def) => def.template_name.as_str(),
        }
    }

    pub fn abi_version(&self) -> u16 {
        match self {
            TemplateDef::V1(def) => def.abi_version,
        }
    }

    pub fn get_function(&self, name: &str) -> Option<&FunctionDef> {
        match self {
            TemplateDef::V1(def) => def.get_function(name),
        }
    }

    pub fn functions(&self) -> &[FunctionDef] {
        match self {
            TemplateDef::V1(def) => &def.functions,
        }
    }

    /// Encodes the template definition with a length prefix, which is required for passing data to the engine in wasm.
    /// The length prefix is a 4-byte little-endian integer representing the total length of the encoded data (including
    /// the prefix itself).
    #[cfg(all(not(target_arch = "wasm32"), feature = "std"))]
    pub fn encode_for_wasm_embedding(&self) -> Result<Vec<u8>, tari_bor::BorError> {
        use std::io::Write;

        use tari_bor::{encode_into_writer, encoded_len};

        use crate::WASM_PTR_SIZE;
        let data_len = encoded_len(self);
        // for the length prefix
        let mut buf = Vec::with_capacity(data_len + WASM_PTR_SIZE);
        let full_len = data_len + WASM_PTR_SIZE;
        let writer = &mut buf;
        // length prefix
        writer.write_all(&(full_len as u32).to_le_bytes())?;
        encode_into_writer(self, writer)?;
        Ok(buf)
    }
}

#[derive(Debug, Clone, Default, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct TemplateDefV1 {
    #[n(0)]
    pub template_name: String,
    #[n(1)]
    pub abi_version: WasmAbiVersion,
    #[n(2)]
    pub functions: Vec<FunctionDef>,
}

impl TemplateDefV1 {
    pub fn get_function(&self, name: &str) -> Option<&FunctionDef> {
        self.functions.iter().find(|f| f.name.as_str() == name)
    }
}

#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FunctionDef {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub arguments: Vec<ArgDef>,
    #[n(2)]
    pub output: Type,
    #[n(3)]
    pub is_mut: bool,
    #[n(4)]
    pub is_migration: bool,
}

#[derive(Debug, Clone, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ArgDef {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub arg_type: Type,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Type {
    #[default]
    #[n(0)]
    Unit,
    #[n(1)]
    Bool,
    #[n(2)]
    I8,
    #[n(3)]
    I16,
    #[n(4)]
    I32,
    #[n(5)]
    I64,
    #[n(6)]
    I128,
    #[n(7)]
    U8,
    #[n(8)]
    U16,
    #[n(9)]
    U32,
    #[n(10)]
    U64,
    #[n(11)]
    U128,
    #[n(12)]
    String,
    #[n(13)]
    Vec(#[n(0)] Box<Type>),
    #[n(14)]
    Tuple(#[n(0)] Vec<Type>),
    #[n(15)]
    Other {
        #[n(0)]
        name: String,
    },
    #[n(16)]
    Option(#[n(0)] Box<Type>),
}

impl Type {
    pub fn other(&self) -> Option<&str> {
        match self {
            Type::Other { name } => Some(name),
            _ => None,
        }
    }
}

#[cfg(feature = "std")]
impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Unit => write!(f, "Unit"),
            Type::Bool => write!(f, "Bool"),
            Type::I8 => write!(f, "I8"),
            Type::I16 => write!(f, "I16"),
            Type::I32 => write!(f, "I32"),
            Type::I64 => write!(f, "I64"),
            Type::I128 => write!(f, "I128"),
            Type::U8 => write!(f, "U8"),
            Type::U16 => write!(f, "U16"),
            Type::U32 => write!(f, "U32"),
            Type::U64 => write!(f, "U64"),
            Type::U128 => write!(f, "U128"),
            Type::String => write!(f, "String"),
            Type::Vec(t) => write!(f, "Vec<{}>", t),
            Type::Option(t) => write!(f, "Option<{}>", t),
            Type::Tuple(types) => {
                let type_list = types.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",");
                write!(f, "Tuple<{}>", type_list)
            },
            Type::Other { name } => write!(f, "{}", name),
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod nesting_depth_tests {
    use super::*;

    // The bound the engine applies at its trust boundaries
    // (`tari_engine_types::limits::MAX_CBOR_NESTING_DEPTH`). Named here rather than imported
    // because `tari_engine_types` depends on this crate, not the other way around.
    const MAX_NESTING_DEPTH: usize = 256;

    // `Type` is self-recursive through `Vec`/`Tuple`/`Option` and both of its decoders are derived,
    // so neither can thread a nesting bound of its own. The bound belongs to `tari_bor`'s decode
    // entry points, and these payloads are what prove it is there: without it the decoder recurses
    // once per level and the stack overflow aborts the process instead of returning an error.
    const PAST_THE_BOUND: usize = 100_000;

    // The smallest stack untrusted decode runs on: a tokio worker. Whatever the bound admits has to
    // decode within it, or the bound is not a bound.
    const WORKER_STACK_BYTES: usize = 2 * 1024 * 1024;

    // Repeats one `Type::Vec` wrapper's worth of framing, taken from a real encode so the fixture
    // follows whatever framing the codec emits.
    fn nested_payload(one_level: &[u8], innermost: &[u8], levels: usize) -> Vec<u8> {
        let prefix = &one_level[..one_level.len() - innermost.len()];
        let mut bytes = Vec::with_capacity(prefix.len() * levels + innermost.len());
        for _ in 0..levels {
            bytes.extend_from_slice(prefix);
        }
        bytes.extend_from_slice(innermost);
        bytes
    }

    // The deepest payload the bound lets through, found by asking the bound rather than by counting
    // wire levels per `Type` level — that count is the codec's business and differs between the two.
    fn deepest_payload_within_bound(build: impl Fn(usize) -> Vec<u8>) -> Vec<u8> {
        (1..=MAX_NESTING_DEPTH)
            .rev()
            .map(build)
            .find(|bytes| tari_bor::check_nesting_depth(bytes, MAX_NESTING_DEPTH).is_ok())
            .expect("no nesting level is within the bound")
    }

    fn on_a_worker_sized_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(WORKER_STACK_BYTES)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    fn minicbor_payload(levels: usize) -> Vec<u8> {
        let innermost = tari_bor::encode(&Type::Unit).unwrap();
        let one_level = tari_bor::encode(&Type::Vec(Box::new(Type::Unit))).unwrap();
        nested_payload(&one_level, &innermost, levels)
    }

    #[test]
    fn deeply_nested_type_is_rejected_by_the_minicbor_decoder() {
        let bytes = minicbor_payload(PAST_THE_BOUND);
        assert!(tari_bor::decode_with_max_depth::<Type>(&bytes, MAX_NESTING_DEPTH).is_err());
    }

    #[test]
    fn minicbor_decode_at_the_bound_fits_a_worker_stack() {
        let bytes = deepest_payload_within_bound(minicbor_payload);
        on_a_worker_sized_stack(move || {
            tari_bor::decode_with_max_depth::<Type>(&bytes, MAX_NESTING_DEPTH).unwrap();
        });
    }

    #[cfg(feature = "serde")]
    mod serde_route {
        use tari_bor::serde_codec;

        use super::*;

        fn serde_payload(levels: usize) -> Vec<u8> {
            let innermost = serde_codec::to_vec(Type::Unit).unwrap();
            let one_level = serde_codec::to_vec(Type::Vec(Box::new(Type::Unit))).unwrap();
            nested_payload(&one_level, &innermost, levels)
        }

        #[test]
        fn deeply_nested_type_is_rejected_by_the_serde_decoder() {
            let bytes = serde_payload(PAST_THE_BOUND);
            assert!(serde_codec::from_slice_with_max_depth::<Type>(&bytes, MAX_NESTING_DEPTH).is_err());
        }

        #[test]
        fn serde_decode_at_the_bound_fits_a_worker_stack() {
            let bytes = deepest_payload_within_bound(serde_payload);
            on_a_worker_sized_stack(move || {
                serde_codec::from_slice_with_max_depth::<Type>(&bytes, MAX_NESTING_DEPTH).unwrap();
            });
        }
    }
}
