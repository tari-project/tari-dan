//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use tari_template_abi::{
    rust::{format, io},
};

use crate::models::{Amount, ComponentAddress};

/// All the possible value types that can be passed as arguments or returned from instruction calls
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
//#[serde(tag = "type", content = "value")]
pub enum Value {
    // Basic values
    Unit,
    Bool(bool),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    String(String),

    // Complex values
    Amount(Amount),
    Tuple(Vec<Value>),
    ComponentAddress(ComponentAddress),
}
