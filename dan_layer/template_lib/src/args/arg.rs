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

use tari_template_abi::{decode, encode, rust::io, Decode, Encode};

#[derive(Debug, Clone, PartialEq, Eq, Decode, Encode)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize))]
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
fn bytes_to_hex(bytes: &Vec<u8>) -> Result<String, std::fmt::Error> {
    let mut hex = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        write!(hex, "{:02x?}", byte)?;
    }

    Ok(hex)
}
