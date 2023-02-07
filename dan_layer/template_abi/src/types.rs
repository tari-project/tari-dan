//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_bor::{borsh, Decode, Encode};

use crate::rust::{boxed::Box, format, io, string::String, vec::Vec};

#[derive(Debug, Clone, Encode, Decode)]
pub struct TemplateDef {
    pub template_name: String,
    pub functions: Vec<FunctionDef>,
}

impl TemplateDef {
    pub fn get_function(&self, name: &str) -> Option<&FunctionDef> {
        self.functions.iter().find(|f| f.name.as_str() == name)
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct FunctionDef {
    pub name: String,
    pub arguments: Vec<Type>,
    pub output: Type,
    pub is_mut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
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
    Vec(Box<Type>),
    Other { name: String },
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
            Type::Other { name } => write!(f, "{}", name),
        }
    }
}

impl Encode for Type {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        let variant_idx: u8 = match self {
            Type::Unit => 0u8,
            Type::Bool => 1u8,
            Type::I8 => 2u8,
            Type::I16 => 3u8,
            Type::I32 => 4u8,
            Type::I64 => 5u8,
            Type::I128 => 6u8,
            Type::U8 => 7u8,
            Type::U16 => 8u8,
            Type::U32 => 9u8,
            Type::U64 => 10u8,
            Type::U128 => 11u8,
            Type::String => 12u8,
            Type::Vec(_) => 13u8,
            Type::Other { .. } => 100u8,
        };
        Encode::serialize(&variant_idx, writer)?;
        match self {
            Type::Unit => {},
            Type::Bool => {},
            Type::I8 => {},
            Type::I16 => {},
            Type::I32 => {},
            Type::I64 => {},
            Type::I128 => {},
            Type::U8 => {},
            Type::U16 => {},
            Type::U32 => {},
            Type::U64 => {},
            Type::U128 => {},
            Type::String => {},
            Type::Vec(ty) => {
                Encode::serialize(ty, writer)?;
            },
            Type::Other { name } => {
                Encode::serialize(name, writer)?;
            },
        }
        Ok(())
    }
}

impl Decode for Type {
    fn deserialize(buf: &mut &[u8]) -> Result<Self, io::Error> {
        let variant_idx: u8 = Decode::deserialize(buf)?;
        let return_value = match variant_idx {
            0u8 => Type::Unit,
            1u8 => Type::Bool,
            2u8 => Type::I8,
            3u8 => Type::I16,
            4u8 => Type::I32,
            5u8 => Type::I64,
            6u8 => Type::I128,
            7u8 => Type::U8,
            8u8 => Type::U16,
            9u8 => Type::U32,
            10u8 => Type::U64,
            11u8 => Type::U128,
            12u8 => Type::String,
            13u8 => Type::Vec(Decode::deserialize(buf)?),
            100u8 => Type::Other {
                name: Decode::deserialize(buf)?,
            },
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Unexpected variant index for Type: {:?}", variant_idx),
                ));
            },
        };
        Ok(return_value)
    }
}
