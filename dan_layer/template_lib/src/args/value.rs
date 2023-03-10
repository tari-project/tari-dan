//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use tari_template_abi::{
    rust::{format, io},
    Decode,
    Encode,
};

use crate::models::{Amount, ComponentAddress};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[
    serde(tag = "type", content = "value")
]
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

// These are manually implemented because the derive macro fails to resolve with self-referencing enums
impl Decode for Value {
    fn deserialize(buf: &mut &[u8]) -> Result<Self, io::Error> {
        let variant_idx: u8 = Decode::deserialize(buf)?;
        let return_value = match variant_idx {
            0u8 => Value::Unit,
            1u8 => Value::Bool(Decode::deserialize(buf)?),
            2u8 => Value::I8(Decode::deserialize(buf)?),
            3u8 => Value::I16(Decode::deserialize(buf)?),
            4u8 => Value::I32(Decode::deserialize(buf)?),
            5u8 => Value::I64(Decode::deserialize(buf)?),
            6u8 => Value::I128(Decode::deserialize(buf)?),
            7u8 => Value::U8(Decode::deserialize(buf)?),
            8u8 => Value::U16(Decode::deserialize(buf)?),
            9u8 => Value::U32(Decode::deserialize(buf)?),
            10u8 => Value::U64(Decode::deserialize(buf)?),
            11u8 => Value::U128(Decode::deserialize(buf)?),
            12u8 => Value::String(Decode::deserialize(buf)?),
            13u8 => Value::Amount(Decode::deserialize(buf)?),
            14u8 => Value::Tuple(Decode::deserialize(buf)?),
            15u8 => Value::ComponentAddress(Decode::deserialize(buf)?),
            _ => {
                let msg = format!(
                    "Unexpected argument Value variant index: {:?} ({} bytes left to decode)",
                    variant_idx,
                    buf.len()
                );
                return Err(io::Error::new(io::ErrorKind::InvalidInput, msg));
            },
        };
        Ok(return_value)
    }
}

impl Encode for Value {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        let variant_idx: u8 = match self {
            Value::Unit => 0u8,
            Value::Bool(..) => 1u8,
            Value::I8(..) => 2u8,
            Value::I16(..) => 3u8,
            Value::I32(..) => 4u8,
            Value::I64(..) => 5u8,
            Value::I128(..) => 6u8,
            Value::U8(..) => 7u8,
            Value::U16(..) => 8u8,
            Value::U32(..) => 9u8,
            Value::U64(..) => 10u8,
            Value::U128(..) => 11u8,
            Value::String(..) => 12u8,
            Value::Amount(..) => 13u8,
            Value::Tuple(..) => 14u8,
            Value::ComponentAddress(..) => 15u8,
        };
        writer.write_all(&variant_idx.to_le_bytes())?;
        match self {
            Value::Unit => {},
            Value::Bool(id0) => {
                id0.serialize(writer)?;
            },
            Value::I8(id0) => {
                id0.serialize(writer)?;
            },
            Value::I16(id0) => {
                id0.serialize(writer)?;
            },
            Value::I32(id0) => {
                id0.serialize(writer)?;
            },
            Value::I64(id0) => {
                id0.serialize(writer)?;
            },
            Value::I128(id0) => {
                id0.serialize(writer)?;
            },
            Value::U8(id0) => {
                id0.serialize(writer)?;
            },
            Value::U16(id0) => {
                id0.serialize(writer)?;
            },
            Value::U32(id0) => {
                id0.serialize(writer)?;
            },
            Value::U64(id0) => {
                id0.serialize(writer)?;
            },
            Value::U128(id0) => {
                id0.serialize(writer)?;
            },
            Value::String(id0) => {
                id0.serialize(writer)?;
            },
            Value::Amount(id0) => {
                id0.serialize(writer)?;
            },
            Value::Tuple(id0) => {
                id0.serialize(writer)?;
            },
            Value::ComponentAddress(id0) => {
                id0.serialize(writer)?;
            },
        }
        Ok(())
    }
}
