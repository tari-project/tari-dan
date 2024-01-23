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

use serde::{Deserialize, Serialize};
use tari_bor::encode;

/// The possible ways to represent an instruction's argument
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Arg {
    /// The argument is in the transaction execution's workspace, which means it is the result of a previous
    /// instruction
    Workspace(Vec<u8>),
    /// The argument is a value specified in the transaction
    Literal(Vec<u8>),
    // Literal(tari_bor::Value),
}

impl Arg {
    pub fn literal(value: tari_bor::Value) -> Result<Self, tari_bor::BorError> {
        // TODO: Unfortunately, CBOR value does not serialize consistently in JSON so we have to use the byte encoded
        // form for now.
        Ok(Arg::Literal(encode(&value)?))
    }

    pub fn from_type<T: Serialize>(val: &T) -> Result<Self, tari_bor::BorError> {
        Ok(Arg::Literal(encode(val)?))
    }

    pub fn workspace<T: Into<Vec<u8>>>(key: T) -> Self {
        Arg::Workspace(key.into())
    }

    pub fn as_literal_bytes(&self) -> Option<&[u8]> {
        match self {
            Arg::Workspace(_) => None,
            Arg::Literal(bytes) => Some(bytes),
        }
    }
}
