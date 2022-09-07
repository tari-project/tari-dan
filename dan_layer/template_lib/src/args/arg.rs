//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

#[cfg(feature = "serde")]
use std::fmt::Write;

use serde::de::VariantAccess;
use tari_template_abi::{decode, encode, rust::io, Decode, Encode};

#[derive(Debug, Clone, PartialEq, Eq, Decode, Encode)]
// #[cfg_attr(feature = "serde", derive(serde::Deserialize))]
pub enum Arg {
    FromWorkspace(Vec<u8>),
    Literal(Vec<u8>),
}

impl Arg {
    pub fn literal(data: Vec<u8>) -> Self {
        Arg::Literal(data)
    }

    pub fn from_workspace<T: Into<Vec<u8>>>(key: T) -> Self {
        Arg::FromWorkspace(key.into())
    }

    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        decode(bytes)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self).unwrap()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Arg {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        match *self {
            Arg::FromWorkspace(ref s) => {
                serializer.serialize_newtype_variant("FromWorkspace", 0, "FromWorkspace", &bytes_to_hex(s).unwrap())
            },
            Arg::Literal(ref s) => {
                serializer.serialize_newtype_variant("Literal", 1, "Literal", &bytes_to_hex(s).unwrap())
            },
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Arg {
    fn deserialize<D>(deserializer: D) -> Result<Arg, D::Error>
    where D: serde::Deserializer<'de> {
        struct ArgVisitor;

        impl<'de> serde::de::Visitor<'de> for ArgVisitor {
            type Value = Arg;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("Arg")
            }

            fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
            where A: serde::de::EnumAccess<'de> {
                if let Ok((variant_name, variant_value)) = data.variant::<String>() {
                    let value = variant_value
                        .newtype_variant::<String>()
                        .map_err(|_| serde::de::Error::custom("Missing variant data"))?;
                    let bytes = hex_to_bytes(&value).map_err(|_| serde::de::Error::custom("Invalid variant data"))?;
                    let arg = match variant_name.as_str() {
                        "FromWorkspace" => Arg::FromWorkspace(bytes),
                        "Literal" => Arg::Literal(bytes),
                        &_ => return Err(serde::de::Error::custom("Invalid variant")),
                    };

                    Ok(arg)
                } else {
                    Err(serde::de::Error::custom("Invalid data type"))
                }
            }
        }
        deserializer.deserialize_enum("Arg", &["FromWorkspace", "Literal"], ArgVisitor {})
    }
}

#[cfg(feature = "serde")]
fn bytes_to_hex(bytes: &Vec<u8>) -> Result<String, std::fmt::Error> {
    let mut hex = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        write!(hex, "{:02x?}", byte)?;
    }

    Ok(hex)
}

#[cfg(feature = "serde")]
pub fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    // hex string MUST have an even number of charactes (1 byte == 2 hex chars)
    if s.len() & 1 == 1 {
        return Err("Invalid hex len, it must be even".to_string());
    }

    let num_bytes = s.len() / 2;
    let mut bytes = Vec::with_capacity(num_bytes);
    for i in 0..num_bytes {
        let byte = u8::from_str_radix(&s[2 * i..2 * (i + 1)], 16).map_err(|_| "Invalid hex value")?;
        bytes.push(byte);
    }

    Ok(bytes)
}
