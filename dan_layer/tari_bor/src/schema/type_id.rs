//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use borsh::maybestd::io;

use crate::{Decode, Encode};

const EXTENDED_TYPE_ID_START: u8 = 32; // 0x20

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum TypeIdRepr<T> {
    Unit,
    Bool,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    String,
    Enum,
    Array,
    Tuple,
    Extended(T),
}

impl<T: ExtendedTypeId> Encode for TypeIdRepr<T> {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        let variant_idx = match self {
            TypeIdRepr::Unit => 0u8,
            TypeIdRepr::Bool => 1u8,
            TypeIdRepr::I8 => 2u8,
            TypeIdRepr::I16 => 3u8,
            TypeIdRepr::I32 => 4u8,
            TypeIdRepr::I64 => 5u8,
            TypeIdRepr::I128 => 6u8,
            TypeIdRepr::U8 => 7u8,
            TypeIdRepr::U16 => 8u8,
            TypeIdRepr::U32 => 9u8,
            TypeIdRepr::U64 => 10u8,
            TypeIdRepr::U128 => 11u8,
            TypeIdRepr::String => 12u8,
            TypeIdRepr::Enum => 13u8,
            TypeIdRepr::Array => 14u8,
            TypeIdRepr::Tuple => 15u8,
            TypeIdRepr::Extended(ext) => {
                let id = ext.as_type_byte();
                if id < EXTENDED_TYPE_ID_START {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "Extended type id must be in range {} to 255 but was {}",
                            EXTENDED_TYPE_ID_START, id
                        ),
                    ));
                }
                id
            },
        };
        writer.write_all(&variant_idx.to_le_bytes())?;
        Ok(())
    }
}

impl<T: ExtendedTypeId> Decode for TypeIdRepr<T> {
    fn deserialize(buf: &mut &[u8]) -> io::Result<Self> {
        let variant_idx: u8 = Decode::deserialize(buf)?;
        let return_value = match variant_idx {
            0u8 => TypeIdRepr::Unit,
            1u8 => TypeIdRepr::Bool,
            2u8 => TypeIdRepr::I8,
            3u8 => TypeIdRepr::I16,
            4u8 => TypeIdRepr::I32,
            5u8 => TypeIdRepr::I64,
            6u8 => TypeIdRepr::I128,
            7u8 => TypeIdRepr::U8,
            8u8 => TypeIdRepr::U16,
            9u8 => TypeIdRepr::U32,
            10u8 => TypeIdRepr::U64,
            11u8 => TypeIdRepr::U128,
            12u8 => TypeIdRepr::String,
            13u8 => TypeIdRepr::Enum,
            14u8 => TypeIdRepr::Array,
            15u8 => TypeIdRepr::Tuple,
            b => TypeIdRepr::Extended(T::from_type_byte(b).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, format!("Invalid extended type id {}", b))
            })?),
        };
        Ok(return_value)
    }
}

pub trait TypeId<T> {
    fn as_type_id() -> TypeIdRepr<T>;
}

pub trait ExtendedTypeId {
    fn as_type_byte(&self) -> u8;
    fn from_type_byte(byte: u8) -> Option<Self>;
}
